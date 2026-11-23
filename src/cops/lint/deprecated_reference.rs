//! `Lint/DeprecatedReference`: YARD-deprecated references with an assigned index.
//! Fixed UseProjectIndex=false supplies no index, even when selected or enabled.
//! The private input contains declarations, members, ancestry and definition
//! comments, not precomputed reference resolutions or findings.
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct DeprecatedReference;
static DEPRECATION_DETAIL: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"@deprecated(?-u:\s)+(.+)").expect("fixed YARD detail pattern")
});
// Parser::Source::Comment::Associator::MAGIC_COMMENT_RE, with its paired
// Emacs marker backreference expanded into alternatives (Rust has no backrefs).
static MAGIC_COMMENT: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(
        r"(?m-u)^#\s*(?:-\*-\s*(?:frozen_string_literal|warn_indent|warn_past_scope):.*-\*-|(?:frozen_string_literal|warn_indent|warn_past_scope):.*)$",
    )
    .expect("fixed Parser magic comment pattern")
});
// Parser::Source::Buffer::ENCODING_RE: the leading whitespace/# boundary,
// optional whitespace around the separator and nonempty encoding are material.
static ENCODING_COMMENT: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(
        r"(?-u)[\s#](en)?coding\s*[:=]\s*(utf8-mac|[A-Za-z0-9_-]+?(-unix|-dos|-mac)|[A-Za-z0-9_-]+)",
    )
    .expect("fixed Parser encoding comment pattern")
});
impl Cop for DeprecatedReference {
    fn name(&self) -> &'static str {
        "Lint/DeprecatedReference"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Selection is not an index provider. Never self-index the supplied file.
        if node.as_program_node().is_some() {
            self.check_with_index(None, node, file, out);
        }
    }
}

#[cfg_attr(test, derive(serde::Deserialize))]
struct Definition {
    deprecated: bool,
    comments: Vec<String>,
}
#[cfg_attr(test, derive(serde::Deserialize))]
struct Declaration {
    namespace: bool,
    module: bool,
    ancestors: Vec<String>,
    members: HashMap<String, String>,
    definitions: Vec<Definition>,
}
#[cfg_attr(test, derive(serde::Deserialize))]
struct Index {
    declarations: HashMap<String, Declaration>,
    // Resolved lexical namespace-name chains actually present in the assigned
    // graph. Distinct syntactic nesting (A::B versus nested A / B) has distinct
    // index name handles, even when it denotes the same declaration.
    lexical_scopes: HashMap<String, Vec<String>>,
}
impl Index {
    fn declaration(&self, name: &str) -> Option<&Declaration> {
        self.declarations.get(name)
    }
    fn member(&self, owner: &str, member: &str) -> Option<&str> {
        let namespace = self.declaration(owner).filter(|d| d.namespace)?;
        namespace.ancestors.iter().find_map(|ancestor| {
            self.declaration(ancestor)?
                .members
                .get(member)
                .map(String::as_str)
        })
    }
    fn singleton_member(&self, owner: &str, member: &str) -> Option<&str> {
        let namespace = self.declaration(owner).filter(|d| d.namespace)?;
        // Stop at the first existing singleton, even if it has no such member.
        for ancestor in &namespace.ancestors {
            let singleton = format!("{ancestor}::<{}>", ancestor.rsplit("::").next()?);
            if self.declarations.contains_key(&singleton) {
                return self.member(&singleton, member);
            }
        }
        None
    }
    fn first_segment<'a>(&'a self, name: &str, nesting: &[String]) -> Option<&'a str> {
        // Use only actual resolved indexed lexical contexts, never manufacture
        // a missing name handle by joining/re-resolving declaration names.
        let scopes = if nesting.is_empty() {
            &[][..]
        } else {
            self.lexical_scopes.get(&nesting.join("\0"))?.as_slice()
        };
        self.lexical_member(name, scopes)
    }
    fn lexical_member<'a>(&'a self, name: &str, scopes: &[String]) -> Option<&'a str> {
        for scope in scopes.iter().rev() {
            if let Some(member) = self.declaration(scope)?.members.get(name) {
                return Some(member);
            }
        }
        if let Some(scope) = scopes.last() {
            if let Some(member) = self.member(scope, name) {
                return Some(member);
            }
            if !self.declaration(scope)?.module {
                return None;
            }
        }
        self.member("Object", name)
    }
    fn resolve<'a>(&'a self, node: &Node<'_>, nesting: &[String]) -> Option<&'a str> {
        // Do not accept dynamic paths such as object::Old.
        if let Some(path) = node.as_constant_path_node() {
            if let Some(parent) = path.parent() {
                ast::const_name(&parent)?;
            }
        } else if node.as_constant_read_node().is_none() {
            return None;
        }
        let name = ast::const_name(node)?;
        let mut parts = name.split("::");
        let first = parts.next()?;
        let mut declaration = self.first_segment(
            first,
            if node.location().as_slice().starts_with(b"::") {
                &[]
            } else {
                nesting
            },
        )?;
        for part in parts {
            declaration = self.member(declaration, part)?;
        }
        Some(declaration)
    }
}

impl DeprecatedReference {
    /// Explicit internal indexed input only. Normal production receives None.
    fn check_with_index(
        &self,
        index: Option<&Index>,
        root: &Node<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(index) = index else { return };
        let comments = associated_deprecations(root, file);
        let mut visitor = ReferenceVisitor {
            references: References {
                index,
                file,
                out,
                comments,
                namespace_cache: HashMap::new(),
            },
            stack: Vec::new(),
            reports: RefCell::default(),
        };
        walk(&mut visitor, root);
    }
}
struct References<'a> {
    index: &'a Index,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    comments: HashSet<usize>,
    // Per investigation, never retained across source/index changes.
    namespace_cache: HashMap<usize, Option<String>>,
}
impl References<'_> {
    fn nesting(&self, node: &Node<'_>, cx: &NodeContext<'_, '_>) -> Vec<String> {
        let mut nesting: Vec<_> = cx
            .ancestors()
            .filter_map(|ancestor| {
                let (path, body) = namespace_parts(ancestor)?;
                let body = body?;
                let loc = node.location();
                (loc.start_offset() >= body.location().start_offset()
                    && loc.end_offset() <= body.location().end_offset())
                .then(|| ast::const_name(&path))
                .flatten()
            })
            .collect();
        nesting.reverse();
        nesting
    }
    fn namespace(&mut self, cx: &NodeContext<'_, '_>) -> Option<String> {
        let Some(ancestor) = cx.ancestors().find(|n| namespace_parts(n).is_some()) else {
            return self.index.declaration("Object").map(|_| "Object".into());
        };
        let key = ancestor.location().start_offset();
        if let Some(found) = self.namespace_cache.get(&key) {
            return found.clone();
        }
        let (path, _) = namespace_parts(ancestor)?;
        let result = self
            .index
            .resolve(&path, &self.nesting(&path, cx))
            .map(str::to_string);
        self.namespace_cache.insert(key, result.clone());
        result
    }
    fn singleton_context(&self, cx: &NodeContext<'_, '_>) -> bool {
        let mut instance_def = false;
        for ancestor in cx.ancestors() {
            if ancestor.as_singleton_class_node().is_some() {
                return true;
            }
            if let Some(def) = ancestor.as_def_node() {
                if def.receiver().is_some() {
                    return true;
                }
                instance_def = true;
            }
            if namespace_parts(ancestor).is_some() {
                return !instance_def;
            }
        }
        false
    }
    fn check(&mut self, node: &Node<'_>, cx: &NodeContext<'_, '_>) {
        let nesting = self.nesting(node, cx);
        let (kind, name, key, loc) = if let Some(call) = send(node) {
            let name = call.name;
            let member = format!("{name}()");
            let key = match call.receiver {
                None => self.namespace(cx).and_then(|owner| {
                    if self.singleton_context(cx) {
                        self.index.singleton_member(&owner, &member)
                    } else {
                        self.index.member(&owner, &member)
                    }
                }),
                Some(r) if r.as_self_node().is_some() => self.namespace(cx).and_then(|owner| {
                    if self.singleton_context(cx) {
                        self.index.singleton_member(&owner, &member)
                    } else {
                        self.index.member(&owner, &member)
                    }
                }),
                Some(r) => self
                    .index
                    .resolve(&r, &nesting)
                    .and_then(|owner| self.index.singleton_member(owner, &member)),
            };
            ("Method", name, key, call.selector)
        } else if node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some() {
            // on_const excludes every direct child of a module/class definition,
            // including its superclass expression, but not nested qualifiers.
            let parent = cx
                .ancestors()
                .find(|n| !n.as_statements_node().is_some_and(|s| s.body().len() == 1));
            if parent.is_some_and(|parent| namespace_parts(parent).is_some()) {
                return;
            }
            // A casgn with an ordinary Class.new/Module.new send or block
            // also defines a module. Its direct qualifier is exempt, but
            // nested qualifiers remain ordinary const children to check.
            let mut ancestors = cx.ancestors();
            if let (Some(target), Some(write)) = (ancestors.next(), ancestors.next())
                && crate::cops::ast::constant_path_write_target(write).is_some_and(|n| {
                    n.location().start_offset() == target.location().start_offset()
                })
                && namespace_factory_assignment(write)
            {
                return;
            }
            // Prism's write target is a path node; RuboCop's casgn has only the
            // qualifier const as a child (not the assigned final component).
            if cx.parent().is_some_and(|p| {
                crate::cops::ast::constant_path_write_target(p).is_some_and(|target| {
                    target.location().start_offset() == node.location().start_offset()
                })
            }) {
                return;
            }
            let Some(name) = ast::const_name(node) else {
                return;
            };
            (
                "Constant",
                name,
                self.index.resolve(node, &nesting),
                node.location(),
            )
        } else {
            return;
        };
        let Some(declaration) = key.and_then(|k| self.index.declaration(k)) else {
            return;
        };
        if declaration.definitions.is_empty()
            || !declaration.definitions.iter().all(|d| d.deprecated)
        {
            return;
        }
        if cx.ancestors().any(|ancestor| {
            (namespace_parts(ancestor).is_some() || ancestor.as_def_node().is_some())
                && self.comments.contains(&ancestor.location().start_offset())
        }) {
            return;
        }
        let text = declaration.definitions.iter().find_map(|definition| {
            definition
                .comments
                .iter()
                .find(|comment| comment.contains("@deprecated"))
        });
        let detail = text.and_then(|text| {
            let capture = DEPRECATION_DETAIL.captures(text)?.get(1)?;
            let text = capture
                .as_str()
                .trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0');
            (!text.is_empty()).then_some(text)
        });
        let detail = match detail {
            None => ".".into(),
            Some(text) if text.ends_with('.') => format!(": {text}"),
            Some(text) => format!(": {text}."),
        };
        let (line, col) = self.file.line_col(loc.start_offset());
        self.out.push(Offense {
            cop: "Lint/DeprecatedReference",
            severity: DeprecatedReference.severity(),
            correction: None,
            line,
            col,
            end_line: self.file.line_of(loc.end_offset().saturating_sub(1)),
            message: format!("{kind} `{name}` is deprecated{detail}"),
        });
    }
}
struct ReferenceVisitor<'a, 'pr> {
    references: References<'a>,
    stack: Vec<Node<'pr>>,
    reports: RefCell<HashSet<(usize, usize)>>,
}
impl<'pr> NodeVisitor<'pr> for ReferenceVisitor<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let cx = NodeContext {
            ancestors: &self.stack,
            corrections_requested: false,
            require_order_reports: &self.reports,
        };
        self.references.check(&node, &cx);
        self.stack.push(node);
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}
fn namespace_parts<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Option<Node<'pr>>)> {
    if let Some(class) = node.as_class_node() {
        Some((class.constant_path(), class.body()))
    } else {
        node.as_module_node().map(|m| (m.constant_path(), m.body()))
    }
}
struct Send<'pr> {
    name: String,
    receiver: Option<Node<'pr>>,
    selector: ruby_prism::Location<'pr>,
}
/// Prism represents assignment sends separately; report the getter for compound
/// assignments and the setter for multiple-assignment targets, as RuboCop does.
fn send<'pr>(node: &Node<'pr>) -> Option<Send<'pr>> {
    if let Some(call) = node.as_call_node() {
        return Some(Send {
            name: String::from_utf8_lossy(call.name().as_slice()).into(),
            receiver: call.receiver(),
            selector: call.message_loc()?,
        });
    }
    if let Some(call) = node.as_call_target_node() {
        return Some(Send {
            name: String::from_utf8_lossy(call.name().as_slice()).into(),
            receiver: Some(call.receiver()),
            selector: call.message_loc(),
        });
    }
    if let Some(call) = node.as_index_target_node() {
        return Some(Send {
            name: "[]=".into(),
            receiver: Some(call.receiver()),
            selector: call.opening_loc().join(&call.closing_loc())?,
        });
    }
    macro_rules! attribute {
        ($access:ident) => {{
            let call = node.$access()?;
            Send {
                name: String::from_utf8_lossy(call.read_name().as_slice()).into(),
                receiver: call.receiver(),
                selector: call.message_loc()?,
            }
        }};
    }
    macro_rules! index {
        ($access:ident) => {{
            let call = node.$access()?;
            Send {
                name: "[]".into(),
                receiver: call.receiver(),
                selector: call.opening_loc().join(&call.closing_loc())?,
            }
        }};
    }
    Some(match node {
        Node::CallAndWriteNode { .. } => attribute!(as_call_and_write_node),
        Node::CallOrWriteNode { .. } => attribute!(as_call_or_write_node),
        Node::CallOperatorWriteNode { .. } => attribute!(as_call_operator_write_node),
        Node::IndexAndWriteNode { .. } => index!(as_index_and_write_node),
        Node::IndexOrWriteNode { .. } => index!(as_index_or_write_node),
        Node::IndexOperatorWriteNode { .. } => index!(as_index_operator_write_node),
        _ => return None,
    })
}
fn namespace_factory_assignment(node: &Node<'_>) -> bool {
    let Some(write) = node.as_constant_path_write_node() else {
        return false;
    };
    // Reuse the exact send/ordinary-block distinction: csend, numblock
    // and itblock do not satisfy Node#defined_module0 (rubocop-ast:390-398).
    super::constant_resolution::defines_module(&write.value())
}

/// Local port of Parser::Source::Comment::Associator's leading/sparse/decorating
/// ownership (MIT; Copyright (c) 2013-2024 parser project contributors,
/// Copyright (c) 2013-2016 Catherine/whitequark; MIT terms in LICENSE),
/// using one typed-child-complete collection, not per-node rescans.
/// Only definition ownership matters, but all expression children consume trivia.
fn associated_deprecations(root: &Node<'_>, file: &SourceFile) -> HashSet<usize> {
    struct Entry<'pr> {
        node: Node<'pr>,
        children: Vec<usize>,
    }
    #[derive(Default)]
    struct Tree<'pr> {
        entries: Vec<Entry<'pr>>,
        stack: Vec<usize>,
    }
    impl<'pr> NodeVisitor<'pr> for Tree<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            let id = self.entries.len();
            if let Some(parent) = self.stack.last() {
                self.entries[*parent].children.push(id);
            }
            self.entries.push(Entry {
                node,
                children: vec![],
            });
            self.stack.push(id);
        }
        fn leave(&mut self) {
            self.stack.pop();
        }
    }
    let mut tree = Tree::default();
    walk(&mut tree, root);
    for id in 0..tree.entries.len() {
        let mut children = std::mem::take(&mut tree.entries[id].children);
        children.sort_by_key(|child| tree.entries[*child].node.location().start_offset());
        tree.entries[id].children = children;
    }
    let comments = file.comments();
    let mut cursor = 0;
    // Match the associator's initial shebang / magic / encoding skips.
    for kind in 0..3 {
        if let Some(range) = comments.get(cursor) {
            let text = &file.bytes[range.clone()];
            let skip = match kind {
                0 => text.starts_with(b"#!"),
                1 => MAGIC_COMMENT.is_match(text),
                _ => ENCODING_COMMENT.is_match(text),
            };
            if skip {
                cursor += 1;
            }
        }
    }
    let mut result = HashSet::new();
    let mut pending = vec![(0, false)];
    while let Some((id, leaving)) = pending.pop() {
        let entry = &tree.entries[id];
        let node = &entry.node;
        let transparent = matches!(
            node,
            Node::ProgramNode { .. }
                | Node::StatementsNode { .. }
                | Node::ArgumentsNode { .. }
                | Node::ParenthesesNode { .. }
        ) || node
            .as_begin_node()
            .is_some_and(|b| b.begin_keyword_loc().is_none());
        if transparent {
            if !leaving {
                pending.extend(entry.children.iter().rev().map(|i| (*i, false)));
            }
            continue;
        }
        let loc = node.location();
        // Prism's ensure includes the enclosing def's end keyword, unlike the
        // parser AST ensure expression. A sparse comment after an empty ensure
        // therefore belongs to the def, not to a synthetic body wrapper.
        let end = node.as_ensure_node().map_or(loc.end_offset(), |ensure| {
            ensure
                .statements()
                .map_or(ensure.ensure_keyword_loc().end_offset(), |s| {
                    s.location().end_offset()
                })
        });
        while let Some(range) = comments.get(cursor) {
            let belongs = if leaving {
                range.end <= end || file.line_of(range.start) == file.line_of(end.saturating_sub(1))
            } else {
                range.end <= loc.start_offset()
            };
            if !belongs {
                break;
            }
            // Only actual definition owners grant exemptions. Modifier nodes
            // can start at the same offset as their enclosed def/class/module;
            // recording their offsets would wrongly deprecate that definition.
            if (namespace_parts(node).is_some() || node.as_def_node().is_some())
                && file.bytes[range.clone()]
                    .windows(b"@deprecated".len())
                    .any(|s| s == b"@deprecated")
            {
                result.insert(loc.start_offset());
            }
            cursor += 1;
        }
        if !leaving {
            pending.push((id, true));
            pending.extend(entry.children.iter().rev().map(|i| (*i, false)));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    //! RuboCop deprecated_reference_spec.rb: MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    fn indexed(source: &str, index: &Index) -> Vec<Offense> {
        supplied(source, Some(index))
    }
    fn supplied(source: &str, index: Option<&Index>) -> Vec<Offense> {
        let file = SourceFile::new("current.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert!(parsed.errors().next().is_none(), "{source}");
        file.record_comments(&parsed);
        let mut out = vec![];
        DeprecatedReference.check_with_index(index, &parsed.node(), &file, &mut out);
        out.sort_by_key(|o| (o.line, o.col));
        out
    }
    #[test]
    fn all_pinned_graphs_and_authored_controls() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_reference.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            let index: Option<Index> = crate::fixture::value(&row["index"]);
            let found = supplied(source, index.as_ref());
            assert!(supplied(source, None).is_empty());
            let found: Vec<_> = found
                .into_iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, crate::offense::Severity::Error);
                    (o.line, o.col, o.end_line, o.message)
                })
                .collect();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(
                found, expected,
                "{}:{} {}",
                row["origin"], row["line"], source
            );
        }
    }
    fn api_index() -> Index {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_reference.json"
        ))
        .unwrap();
        crate::fixture::value(&rows[1]["index"])
    }
    #[test]
    fn decoding_documentation_preserves_deprecated_definition_exemption() {
        let source = "class Client < Api\n  # @deprecated Handles decoding: use modern instead.\n  def legacy\n    old_method\n  end\nend\n";
        assert!(indexed(source, &api_index()).is_empty());
    }
    #[test]
    fn modifier_comment_owner_does_not_deprecate_enclosed_definition() {
        let index = api_index();
        for modifier in ["if ready", "unless ready", "while ready", "until ready"] {
            let source = format!(
                "class Client < Api\n# @deprecated\ndef legacy; old_method; end {modifier}\nend\n"
            );
            let found = indexed(&source, &index);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 13, 3));
            assert_eq!(
                found[0].message,
                "Method `old_method` is deprecated: Use `#new_method` instead."
            );
            assert!(found[0].correction.is_none());
            assert!(supplied(&source, None).is_empty());
        }
        // A comment actually owned by the def still exempts its references.
        assert!(
            indexed(
                "class Client < Api\n# @deprecated\ndef legacy; old_method; end\nend\n",
                &index
            )
            .is_empty()
        );
    }
    #[test]
    fn factory_exemption_requires_send_or_ordinary_block() {
        // A real assigned graph, not self-indexing or a public CLI activation.
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_reference.json"
        ))
        .unwrap();
        let row = rows
            .iter()
            .find(|r| r["source"] == "# @deprecated\nmodule Legacy; end\nLegacy::C = Class.new\n")
            .unwrap();
        let index: Index = crate::fixture::value(&row["index"]);
        for factory in [
            "Class&.new",
            "Module&.new",
            "Class.new { _1 }",
            "Module.new { it }",
        ] {
            let source = format!("# @deprecated\nmodule Legacy; end\nLegacy::C = {factory}\n");
            let found = indexed(&source, &index);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
            assert_eq!(found[0].message, "Constant `Legacy` is deprecated.");
            assert!(found[0].correction.is_none());
            assert!(supplied(&source, None).is_empty());
        }
        for factory in [
            "Class.new",
            "Module.new",
            "Class.new {}",
            "Module.new do; end",
        ] {
            let source = format!("# @deprecated\nmodule Legacy; end\nLegacy::C = {factory}\n");
            assert!(indexed(&source, &index).is_empty(), "{source}");
        }
    }
    #[test]
    fn declaration_requires_nonempty_all_deprecated_definitions() {
        let mut index = api_index();
        let source = "class Client < Api; def run; old_method; end; end\n";
        assert_eq!(indexed(source, &index).len(), 1);
        index
            .declarations
            .get_mut("Api#old_method()")
            .unwrap()
            .definitions
            .clear();
        assert!(indexed(source, &index).is_empty());
        for deprecated in [false, true] {
            index
                .declarations
                .get_mut("Api#old_method()")
                .unwrap()
                .definitions
                .push(Definition {
                    deprecated,
                    comments: vec![],
                });
            assert!(indexed(source, &index).is_empty());
        }
    }
    #[test]
    fn investigation_cache_never_survives_source_or_index_replacement() {
        let mut index = api_index();
        let source = "class Client < Api; def run; old_method; end; end\n";
        assert_eq!(indexed(source, &index).len(), 1);
        let empty = Index {
            declarations: HashMap::new(),
            lexical_scopes: HashMap::new(),
        };
        assert!(indexed(source, &empty).is_empty());
        assert!(supplied(source, None).is_empty());
        index
            .declarations
            .get_mut("Api#old_method()")
            .unwrap()
            .definitions[0]
            .deprecated = false;
        assert!(indexed(source, &index).is_empty());
        assert_eq!(indexed(source, &api_index()).len(), 1);
        assert!(
            indexed(
                "class Unrelated; def run; old_method; end; end",
                &api_index()
            )
            .is_empty()
        );
    }
    #[test]
    fn assigned_index_reports_selector_but_default_does_not_self_index() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_reference.json"
        ))
        .unwrap();
        let row = &rows.as_array().unwrap()[1];
        let index: Index = crate::fixture::value(&row["index"]);
        let source = "class Client < Api\n  def call; self.old_method; end\nend\n";
        let found = indexed(source, &index);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 18));
        assert_eq!(
            found[0].message,
            "Method `old_method` is deprecated: Use `#new_method` instead."
        );
        assert!(found[0].correction.is_none());
        assert!(
            crate::pipeline::lint_str(source)
                .iter()
                .all(|o| o.cop != "Lint/DeprecatedReference")
        );
    }
}
