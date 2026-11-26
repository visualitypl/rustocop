//! `Lint/NameTypo`: probable constant/method typos with an assigned index.
//! Production preserves fixed UseProjectIndex=false: selection is not a provider.
//! Private inputs are actual declarations/definitions, ancestry, members, lexical
//! handles and gem metadata, never pre-resolved findings or suggestions.
//! Unicode ranking follows Ruby's DidYouMean (BSD-2-Clause/Ruby license);
//! shared Jaro-Winkler and Levenshtein distances remain in name_similarity.
use crate::{
    cops::{
        Cop, NodeContext, ast, name_similarity,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};
use std::{
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet},
};

pub struct NameTypo;
#[cfg(test)]
thread_local! {
    static INDEX_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LITERAL_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl Cop for NameTypo {
    fn name(&self) -> &'static str {
        "Lint/NameTypo"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_some() {
            self.check_with_index(None, node, file, out);
        }
    }
}
#[cfg_attr(test, derive(serde::Deserialize))]
struct Reference {
    name: String,
    resolved: bool,
}
#[cfg_attr(test, derive(serde::Deserialize))]
struct Definition {
    superclass: Option<Reference>,
    mixins: Vec<Reference>,
}
#[cfg_attr(test, derive(serde::Deserialize))]
struct Declaration {
    namespace: bool,
    module: bool,
    ancestors: Vec<String>,
    members: HashMap<String, String>,
    member_names: Vec<String>,
    definitions: Vec<Definition>,
}
#[cfg_attr(test, derive(serde::Deserialize))]
struct Index {
    declarations: HashMap<String, Declaration>,
    lexical_scopes: HashMap<String, Vec<String>>,
    gem_names: Vec<String>,
    path_gems: Vec<String>,
    includes_gems: bool,
}
impl Index {
    fn namespace(&self, name: &str) -> Option<&Declaration> {
        self.declarations.get(name).filter(|d| d.namespace)
    }
    fn member(&self, owner: &str, name: &str) -> Option<&str> {
        self.namespace(owner)?
            .ancestors
            .iter()
            .find_map(|ancestor| {
                self.namespace(ancestor)?
                    .members
                    .get(name)
                    .map(String::as_str)
            })
    }
    fn singleton(&self, owner: &str) -> Option<String> {
        let name = format!("{owner}::<{}>", owner.rsplit("::").next()?);
        self.namespace(&name).map(|_| name)
    }
    fn singleton_member(&self, owner: &str, name: &str) -> Option<&str> {
        for ancestor in &self.namespace(owner)?.ancestors {
            if let Some(singleton) = self.singleton(ancestor) {
                return self.member(&singleton, name);
            }
        }
        None
    }
    fn lexical_member(&self, name: &str, nesting: &[String]) -> Option<&str> {
        let scopes = if nesting.is_empty() {
            &[][..]
        } else {
            self.lexical_scopes.get(&nesting.join("\0"))?.as_slice()
        };
        for scope in scopes.iter().rev() {
            if let Some(member) = self.namespace(scope)?.members.get(name) {
                return Some(member);
            }
        }
        if let Some(scope) = scopes.last() {
            if let Some(member) = self.member(scope, name) {
                return Some(member);
            }
            if !self.namespace(scope)?.module {
                return None;
            }
        }
        self.member("Object", name)
    }
    fn resolve(&self, node: &Node<'_>, nesting: &[String], file: &SourceFile) -> Option<&str> {
        if let Some(path) = node.as_constant_path_node() {
            if let Some(parent) = path.parent() {
                const_name(&parent, file)?;
            }
        } else if node.as_constant_read_node().is_none() {
            return None;
        }
        let name = const_name(node, file)?;
        let mut parts = name.split("::");
        let mut owner = self.lexical_member(
            parts.next()?,
            if node.location().as_slice().starts_with(b"::") {
                &[]
            } else {
                nesting
            },
        )?;
        for part in parts {
            owner = self.member(owner, part)?;
        }
        Some(owner)
    }
    fn complete(&self, owner: &str) -> bool {
        self.namespace(owner).is_some_and(|d| {
            d.ancestors.iter().all(|a| {
                self.namespace(a).is_some_and(|d| {
                    d.definitions.iter().all(|d| {
                        d.superclass
                            .as_ref()
                            .is_none_or(|r| r.resolved && !r.name.is_empty())
                            && d.mixins.iter().all(|r| r.resolved && !r.name.is_empty())
                    })
                })
            })
        })
    }
    fn gem_owned(&self, owner: &str) -> bool {
        !self.includes_gems
            && self
                .gem_names
                .iter()
                .filter(|g| !self.path_gems.contains(g))
                .any(|g| {
                    g.replace(['-', '_'], "").to_lowercase()
                        == owner.split("::").next().unwrap_or("").to_lowercase()
                })
    }
    fn constant_names(&self, owner: &str) -> Vec<&str> {
        self.namespace(owner)
            .into_iter()
            .flat_map(|d| &d.member_names)
            .filter(|n| !n.contains(['#', '<']))
            .filter_map(|n| n.rsplit("::").next())
            .collect()
    }
    fn method_names(&self, owner: &str) -> Vec<&str> {
        let mut scopes: Vec<String> = self
            .namespace(owner)
            .into_iter()
            .flat_map(|d| &d.ancestors)
            .filter_map(|a| self.singleton(a))
            .collect();
        scopes.push(owner.to_owned());
        scopes
            .iter()
            .filter_map(|s| self.namespace(s))
            .flat_map(|d| &d.member_names)
            .filter_map(|name| {
                let (_, method) = name.rsplit_once('#')?;
                let method = method.strip_suffix("()")?;
                let base = method.strip_suffix(['?', '!']).unwrap_or(method);
                let mut chars = base.bytes();
                let first = chars.next()?;
                ((first.is_ascii_alphabetic() || first == b'_')
                    && chars.all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                .then_some(method)
            })
            .collect()
    }
}
impl NameTypo {
    fn check_with_index(
        &self,
        index: Option<&Index>,
        root: &Node<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // No allocation, literal indexing or AST walk without the prerequisite.
        let Some(index) = index else { return };
        #[cfg(test)]
        INDEX_WALKS.with(|n| n.set(n.get() + 1));
        let mut visitor = TypoVisitor {
            index,
            root,
            file,
            out,
            stack: Vec::new(),
            reports: RefCell::default(),
            literals: OnceCell::new(),
        };
        walk(&mut visitor, root);
    }
}
struct TypoVisitor<'a, 'pr> {
    index: &'a Index,
    root: &'a Node<'pr>,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    stack: Vec<Node<'pr>>,
    reports: RefCell<HashSet<(usize, usize)>>,
    literals: OnceCell<HashSet<String>>,
}
impl TypoVisitor<'_, '_> {
    fn literal(&self, name: &str) -> bool {
        self.literals
            .get_or_init(|| literal_names(self.root, self.file))
            .contains(name)
    }
    fn check(&mut self, node: &Node<'_>) {
        let cx = NodeContext {
            ancestors: &self.stack,
            corrections_requested: false,
            require_order_reports: &self.reports,
        };
        if cx.ancestors().any(|a| a.as_defined_node().is_some()) {
            return;
        }
        // Follow actual body edges, not byte containment: heredoc descendants
        // may live beyond their enclosing expression's physical source span.
        let mut child = node;
        let mut nesting = Vec::new();
        for ancestor in cx.ancestors() {
            if let Some((path, Some(body))) = namespace_parts(ancestor)
                && body.location().start_offset() == child.location().start_offset()
                && body.location().end_offset() == child.location().end_offset()
                && let Some(name) = const_name(&path, self.file)
            {
                nesting.push(name);
            }
            child = ancestor;
        }
        nesting.reverse();
        if let Some(path) = node.as_constant_path_node() {
            let Some(parent) = path.parent() else { return };
            if !is_const(&parent) || definition_identifier(node, &cx) {
                return;
            }
            let Some(name) = path.name().map(|n| projected_name(n.as_slice(), self.file)) else {
                return;
            };
            let Some(owner) = self.index.resolve(&parent, &nesting, self.file) else {
                return;
            };
            if self.index.namespace(owner).is_none()
                || self.index.member(owner, &name).is_some()
                || !self.index.complete(owner)
                || self.index.gem_owned(owner)
                || self.literal(&name)
            {
                return;
            }
            let Some(suggestion) = spell_check(&name, self.index.constant_names(owner)) else {
                return;
            };
            let loc = path.name_loc();
            let namespace = const_name(&parent, self.file).unwrap();
            self.report(loc, format!("Possible typo: `{name}` is not defined in `{namespace}`. Did you mean `{suggestion}`?"));
        } else if let Some(call) = send(node, self.file) {
            if !is_const(&call.receiver) {
                return;
            }
            let Some(owner) = self.index.resolve(&call.receiver, &nesting, self.file) else {
                return;
            };
            let setter = call.name.ends_with('=') && !ast::is_operator_method(call.name.as_bytes());
            let base = if setter {
                call.name.strip_suffix('=').unwrap()
            } else {
                &call.name
            };
            if self.index.namespace(owner).is_none()
                || [&call.name, base].iter().any(|name| {
                    let member = format!("{name}()");
                    self.index.singleton_member(owner, &member).is_some()
                        || self.index.member(owner, &member).is_some()
                })
                || !self.index.complete(owner)
                || self.index.gem_owned(owner)
                || self.literal(base)
            {
                return;
            }
            let Some(suggestion) = spell_check(base, self.index.method_names(owner)) else {
                return;
            };
            let receiver = const_name(&call.receiver, self.file).unwrap();
            self.report(call.selector, format!("Possible typo: `{receiver}` does not respond to `{}`. Did you mean `{suggestion}{}`?", call.name, if setter { "=" } else { "" }));
        }
    }
    fn report(&mut self, loc: Location<'_>, message: String) {
        let (line, col) = self.file.reference_line_col(loc.start_offset());
        self.out.push(Offense {
            cop: "Lint/NameTypo",
            severity: NameTypo.severity(),
            correction: None,
            line,
            col,
            end_line: self.file.line_of(loc.end_offset().saturating_sub(1)),
            message,
        });
    }
}
impl<'pr> NodeVisitor<'pr> for TypoVisitor<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        self.check(&node);
        self.stack.push(node);
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}
fn spell_check<'a>(name: &str, dictionary: Vec<&'a str>) -> Option<&'a str> {
    let mut seen = HashSet::new();
    let dictionary: Vec<_> = dictionary.into_iter().filter(|n| seen.insert(*n)).collect();
    if name.is_ascii() && dictionary.iter().all(|word| word.is_ascii()) {
        return name_similarity::find_similar_name(name, &dictionary);
    }
    // DidYouMean uses Ruby3.4's Unicode15.0 context-free downcase. Rust's
    // newer tables and str::to_lowercase final-sigma context (ΟΣ -> ος rather
    // than οσ) are not equivalent for this cop's Unicode constant dictionary.
    // Port only that consumer's ranking here; reuse the existing distances.
    let input = normalize(name);
    let threshold = if input.len() > 3 { 0.834 } else { 0.77 };
    let mut words: Vec<_> = dictionary
        .into_iter()
        .filter(|word| {
            *word != name && name_similarity::jaro_winkler(&normalize(word), &input) >= threshold
        })
        .collect();
    words.sort_by(|a, b| {
        let distance =
            |word: &str| name_similarity::jaro_winkler(&word.chars().collect::<Vec<_>>(), &input);
        distance(a).total_cmp(&distance(b))
    });
    words.reverse();
    let threshold = input.len().div_ceil(4);
    words
        .iter()
        .copied()
        .find(|word| name_similarity::levenshtein(&normalize(word), &input) <= threshold)
        .or_else(|| {
            words.into_iter().find(|word| {
                let word = normalize(word);
                name_similarity::levenshtein(&word, &input) < input.len().min(word.len())
            })
        })
}
fn normalize(word: &str) -> Vec<char> {
    let mut normalized = Vec::with_capacity(word.len());
    for c in word.chars().filter(|&c| c != '@') {
        // Ruby3.4 uses Unicode15.0; these mappings were added in Unicode16/17
        // (Rust1.99 uses Unicode17). Preserve them verbatim, as Ruby does.
        // The native all-scalar test also catches mapping drift on Rust upgrades.
        if matches!(
            c,
            '\u{1c89}'
                | '\u{a7cb}'
                | '\u{a7cc}'
                | '\u{a7ce}'
                | '\u{a7d2}'
                | '\u{a7d4}'
                | '\u{a7da}'
                | '\u{a7dc}'
                | '\u{10d50}'..='\u{10d65}'
                | '\u{16ea0}'..='\u{16eb8}'
        ) {
            normalized.push(c);
        } else {
            normalized.extend(c.to_lowercase());
        }
    }
    normalized
}
// Project only identifier payloads, never the whole source or decoded escape
// results. Parser::Source::Buffer transcodes/composes before creating names.
fn projected_name(bytes: &[u8], file: &SourceFile) -> String {
    let projected = file.match_pattern_source(bytes);
    match std::str::from_utf8(&projected) {
        Ok(name) => name.to_owned(),
        Err(_) => projected
            .iter()
            .map(|b| {
                if b.is_ascii() {
                    char::from(*b).to_string()
                } else {
                    format!("\\x{b:02X}")
                }
            })
            .collect(),
    }
}
fn const_name(node: &Node<'_>, file: &SourceFile) -> Option<String> {
    if let Some(read) = node.as_constant_read_node() {
        return Some(projected_name(read.name().as_slice(), file));
    }
    let path = node.as_constant_path_node()?;
    let short = projected_name(path.name()?.as_slice(), file);
    Some(match path.parent() {
        None => short,
        Some(parent) => format!("{}::{short}", const_name(&parent, file)?),
    })
}
fn is_const(node: &Node<'_>) -> bool {
    node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some()
}
fn namespace_parts<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Option<Node<'pr>>)> {
    if let Some(c) = node.as_class_node() {
        Some((c.constant_path(), c.body()))
    } else {
        node.as_module_node().map(|m| (m.constant_path(), m.body()))
    }
}
fn definition_identifier(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let parent = cx
        .ancestors()
        .find(|p| !p.as_statements_node().is_some_and(|s| s.body().len() == 1));
    if parent.is_some_and(|p| {
        namespace_parts(p).is_some()
            || crate::cops::ast::constant_path_write_target(p)
                .is_some_and(|t| t.location().start_offset() == node.location().start_offset())
    }) {
        return true;
    }
    let mut ancestors = cx.ancestors();
    if let (Some(target), Some(write)) = (ancestors.next(), ancestors.next()) {
        return crate::cops::ast::constant_path_write_target(write)
            .is_some_and(|n| n.location().start_offset() == target.location().start_offset())
            && write
                .as_constant_path_write_node()
                .is_some_and(|w| super::constant_resolution::defines_module(&w.value()));
    }
    false
}
struct Send<'pr> {
    name: String,
    receiver: Node<'pr>,
    selector: Location<'pr>,
}
fn send<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Send<'pr>> {
    if let Some(c) = node.as_call_node() {
        return Some(Send {
            name: projected_name(c.name().as_slice(), file),
            receiver: c.receiver()?,
            selector: c.message_loc()?,
        });
    }
    if let Some(c) = node.as_call_target_node() {
        return Some(Send {
            name: projected_name(c.name().as_slice(), file),
            receiver: c.receiver(),
            selector: c.message_loc(),
        });
    }
    macro_rules! attribute {
        ($access:ident) => {{
            let c = node.$access()?;
            Send {
                name: projected_name(c.read_name().as_slice(), file),
                receiver: c.receiver()?,
                selector: c.message_loc()?,
            }
        }};
    }
    Some(match node {
        Node::CallAndWriteNode { .. } => attribute!(as_call_and_write_node),
        Node::CallOrWriteNode { .. } => attribute!(as_call_or_write_node),
        Node::CallOperatorWriteNode { .. } => attribute!(as_call_operator_write_node),
        _ => return None,
    })
}
fn literal_names(root: &Node<'_>, file: &SourceFile) -> HashSet<String> {
    #[cfg(test)]
    LITERAL_WALKS.with(|n| n.set(n.get() + 1));
    struct Literals<'a> {
        file: &'a SourceFile,
        names: HashSet<String>,
        root_literal: Option<usize>,
    }
    impl NodeVisitor<'_> for Literals<'_> {
        fn enter(&mut self, node: Node<'_>) {
            if self.root_literal == Some(node.location().start_offset()) {
                return;
            }
            if let Some(s) = node.as_symbol_node() {
                let raw = s.value_loc();
                let single = s
                    .opening_loc()
                    .is_some_and(|o| o.as_slice() == b":'" || o.as_slice().starts_with(b"%s"));
                let parts = ast::require_literal::parts(
                    raw.as_ref().map_or(s.unescaped(), |r| r.as_slice()),
                    s.unescaped(),
                    single,
                    self.file,
                );
                if let [value] = parts.as_slice() {
                    self.names.insert(String::from_utf8_lossy(value).into());
                } else {
                    for value in parts {
                        scan_names(&value, &mut self.names);
                    }
                }
            } else if let Some(s) = node.as_string_node() {
                let single = s.opening_loc().is_some_and(|o| {
                    o.as_slice() == b"'"
                        || o.as_slice().starts_with(b"%q")
                        || o.as_slice().starts_with(b"%w")
                        || o.as_slice().ends_with(b"'")
                });
                for value in ast::require_literal::parts(
                    s.content_loc().as_slice(),
                    s.unescaped(),
                    single,
                    self.file,
                ) {
                    scan_names(&value, &mut self.names);
                }
            } else if node.as_source_file_node().is_some() {
                scan_names(
                    self.file.source_path().as_os_str().as_encoded_bytes(),
                    &mut self.names,
                );
            } else if let Some(s) = node.as_regular_expression_node() {
                for value in ast::require_literal::parts(
                    s.content_loc().as_slice(),
                    s.unescaped(),
                    false,
                    self.file,
                ) {
                    scan_names(&value, &mut self.names);
                }
            } else if let Some(s) = node.as_x_string_node() {
                for value in ast::require_literal::parts(
                    s.content_loc().as_slice(),
                    s.unescaped(),
                    false,
                    self.file,
                ) {
                    scan_names(&value, &mut self.names);
                }
            }
        }
        fn leave(&mut self) {}
    }
    let body = root.as_program_node().and_then(|p| {
        p.statements()
            .body()
            .iter()
            .next()
            .filter(|_| p.statements().body().len() == 1)
    });
    let root_literal = body
        .filter(|n| {
            n.as_symbol_node().is_some()
                || n.as_string_node().is_some()
                || n.as_source_file_node().is_some()
        })
        .map(|n| n.location().start_offset());
    let mut visitor = Literals {
        file,
        names: HashSet::new(),
        root_literal,
    };
    walk(&mut visitor, root);
    visitor.names
}
fn scan_names(value: &[u8], names: &mut HashSet<String>) {
    let mut i = 0;
    while i < value.len() {
        if value[i].is_ascii_alphabetic() || value[i] == b'_' {
            let start = i;
            i += 1;
            while i < value.len() && (value[i].is_ascii_alphanumeric() || value[i] == b'_') {
                i += 1;
            }
            if value.get(i).is_some_and(|b| matches!(b, b'?' | b'!')) {
                i += 1;
            }
            names.insert(String::from_utf8_lossy(&value[start..i]).into());
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    //! RuboCop name_typo_spec.rb (MIT License),
    //! Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{offense::Offense, source::SourceFile};

    fn supplied(source: &[u8], index: Option<&Index>) -> Vec<Offense> {
        let file = SourceFile::new("/lib/current.rb".into(), source.to_vec());
        let parsed = crate::parser::parse(&file.bytes);
        assert!(parsed.errors().next().is_none(), "{source:?}");
        file.record_encoding(&parsed);
        file.record_comments(&parsed);
        let mut out = vec![];
        NameTypo.check_with_index(index, &parsed.node(), &file, &mut out);
        out.sort_by_key(|o| (o.line, o.col));
        out
    }
    #[test]
    fn all_pinned_assigned_graphs_and_controls() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/name_typo.json")).unwrap();
        assert_eq!(rows.len(), 538);
        assert_eq!(
            rows.iter().filter(|r| r["origin"] == "external").count(),
            41
        );
        for row in rows {
            let source: Vec<u8> = if let Some(source) = row["source"].as_str() {
                source.as_bytes().to_vec()
            } else {
                crate::fixture::value(&row["source_bytes"])
            };
            let index: Option<Index> = crate::fixture::value(&row["index"]);
            let actual: Vec<_> = supplied(&source, index.as_ref())
                .into_iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, crate::offense::Severity::Warning);
                    (o.line, o.col, o.end_line, o.message)
                })
                .collect();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(
                actual, expected,
                "{}:{} {source:?}",
                row["origin"], row["line"]
            );
            assert!(supplied(&source, None).is_empty());
            assert!(
                crate::pipeline::lint_str(&String::from_utf8_lossy(&source))
                    .iter()
                    .all(|o| o.cop != "Lint/NameTypo")
            );
        }
    }
    fn api_index() -> Index {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/name_typo.json")).unwrap();
        serde_json::from_value(
            rows.iter().find(|r| r["origin"] == "control").unwrap()["index"].clone(),
        )
        .unwrap()
    }
    #[test]
    fn qualified_constant_and_singleton_selector_have_exact_messages() {
        let index = api_index();
        let found = supplied(b"  Services::UserCraetor::Params.new\n  Report&.generate_sumary\n  Config.debug_mdoe = true\n", Some(&index));
        let actual: Vec<_> = found
            .iter()
            .map(|o| (o.line, o.col, o.message.as_str()))
            .collect();
        assert_eq!(
            actual,
            vec![
                (
                    1,
                    13,
                    "Possible typo: `UserCraetor` is not defined in `Services`. Did you mean `UserCreator`?"
                ),
                (
                    2,
                    11,
                    "Possible typo: `Report` does not respond to `generate_sumary`. Did you mean `generate_summary`?"
                ),
                (
                    3,
                    10,
                    "Possible typo: `Config` does not respond to `debug_mdoe=`. Did you mean `debug_mode=`?"
                ),
            ]
        );
    }
    #[test]
    fn unicode_suggestions_use_rubys_context_free_case_mapping() {
        assert_eq!(spell_check("ΟΣ", vec!["Οσ", "Ος"]), Some("Οσ"));
        let found = supplied(b"Greek::\xce\x9f\xce\xa3\n", Some(&api_index()));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 8));
        assert_eq!(
            found[0].message,
            "Possible typo: `ΟΣ` is not defined in `Greek`. Did you mean `Οσ`?"
        );
    }
    #[test]
    fn unicode_suggestions_preserve_ruby34_unmapped_characters() {
        let mut index = api_index();
        let services = index.declarations.get_mut("Services").unwrap();
        services.members = HashMap::from([("A\u{1c8a}".into(), "Services::A\u{1c8a}".into())]);
        services.member_names = vec!["Services::A\u{1c8a}".into()];
        assert!(supplied("Services::A\u{1c89}\n".as_bytes(), Some(&index)).is_empty());
        assert_eq!(spell_check("A\u{1c89}", vec!["A\u{1c8a}"]), None);
    }
    #[test]
    fn normalization_matches_native_ruby34_for_every_unicode_scalar() {
        let Ok(ruby) = std::env::var("RUSTOCOP_RUBY34") else {
            return;
        };
        // Execute only this fixed mapping probe, never inspected source.
        let output = std::process::Command::new(ruby)
            .args([
                "-e",
                r##"
                abort 'expected Ruby 3.4' unless RUBY_VERSION.start_with?('3.4.')
                (0..0x10ffff).each do |cp|
                  next if (0xd800..0xdfff).cover?(cp)
                  c = cp.chr(Encoding::UTF_8)
                  lower = c.downcase.delete('@')
                  puts "#{cp}:#{lower.codepoints.join(',')}" unless c == lower
                end
            "##,
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let reference: HashMap<u32, Vec<char>> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| {
                let (cp, lower) = line.split_once(':').unwrap();
                let lower = lower
                    .split(',')
                    .filter(|c| !c.is_empty())
                    .map(|c| char::from_u32(c.parse().unwrap()).unwrap())
                    .collect();
                (cp.parse().unwrap(), lower)
            })
            .collect();
        for c in (0..=0x10ffff).filter_map(char::from_u32) {
            let expected = reference
                .get(&(c as u32))
                .cloned()
                .unwrap_or_else(|| vec![c]);
            assert_eq!(normalize(&c.to_string()), expected, "U+{:04X}", c as u32);
        }
    }
    #[test]
    fn no_index_performs_no_private_walk_or_fact_allocation() {
        INDEX_WALKS.set(0);
        LITERAL_WALKS.set(0);
        assert!(supplied(b"Services::UserCraetor\nReport.generate_sumary\n", None).is_empty());
        assert_eq!(INDEX_WALKS.get(), 0);
        assert_eq!(LITERAL_WALKS.get(), 0);
    }
    #[test]
    fn literal_names_are_lazy_and_indexed_once_per_investigation() {
        let index = api_index();
        LITERAL_WALKS.set(0);
        assert!(
            supplied(
                b"Report.generate_summary\nExternal.generate_sumary\n",
                Some(&index)
            )
            .is_empty()
        );
        assert_eq!(LITERAL_WALKS.get(), 0);
        let source = "Report.generate_sumary\n".repeat(500);
        assert_eq!(supplied(source.as_bytes(), Some(&index)).len(), 500);
        assert_eq!(LITERAL_WALKS.get(), 1);
    }
    #[test]
    fn root_literal_is_excluded_and_symbols_are_whole_values() {
        for (source, present) in [
            (":generate_sumary", false),
            ("\"generate_sumary\"", false),
            ("p :generate_sumary", true),
            ("p :\"generate_sumary=\"", false),
            ("p \"generate_sumary=\"", true),
            ("p /generate_sumary/", true),
            ("p `generate_sumary`", true),
        ] {
            let file = SourceFile::new("a.rb".into(), source.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            file.record_encoding(&parsed);
            assert_eq!(
                literal_names(&parsed.node(), &file).contains("generate_sumary"),
                present,
                "{source}"
            );
        }
    }
    #[test]
    fn investigations_do_not_retain_literals_graph_or_gem_metadata() {
        let mut index = api_index();
        let source = b"Report.generate_sumary\n";
        assert_eq!(supplied(source, Some(&index)).len(), 1);
        assert!(supplied(b":generate_sumary\nReport.generate_sumary\n", Some(&index)).is_empty());
        assert_eq!(supplied(source, Some(&index)).len(), 1);
        index.gem_names.push("re_port".into());
        assert!(supplied(source, Some(&index)).is_empty());
        index.path_gems.push("re_port".into());
        assert_eq!(supplied(source, Some(&index)).len(), 1);
        index.declarations.clear();
        assert!(supplied(source, Some(&index)).is_empty());
        assert_eq!(supplied(source, Some(&api_index())).len(), 1);
    }
    #[test]
    fn did_you_mean_first_matches_order_normalization_and_ties() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/name_typo_spelling.json"
        ))
        .unwrap();
        for row in rows {
            let dictionary: Vec<&str> = row["dictionary"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect();
            assert_eq!(
                spell_check(row["name"].as_str().unwrap(), dictionary),
                row["suggestion"].as_str(),
                "{row}"
            );
        }
    }
}
