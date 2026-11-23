//! `Lint/UnusedPrivateMethod`: opt-in, assigned-index private instance method use.
//! Fixed UseProjectIndex=false supplies no production index, even when forced.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::{
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet},
    sync::{LazyLock, OnceLock},
};

pub struct UnusedPrivateMethod;
#[cfg(test)]
thread_local! {
    static LITERAL_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static NAMESPACE_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl Cop for UnusedPrivateMethod {
    fn name(&self) -> &'static str {
        "Lint/UnusedPrivateMethod"
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
struct Declaration {
    namespace: bool,
    module: bool,
    private: bool,
    owner: Option<String>,
    ancestors: Vec<String>,
    descendants: Vec<String>,
    members: HashMap<String, String>,
}
#[cfg_attr(test, derive(serde::Deserialize))]
struct Index {
    declarations: HashMap<String, Declaration>,
    lexical_scopes: HashMap<String, Vec<String>>,
    references: Vec<String>,
    #[cfg_attr(test, serde(skip))]
    reference_names: OnceLock<HashSet<String>>,
}
impl Index {
    fn referenced_names(&self) -> &HashSet<String> {
        // Index-owned immutable cache: shared across investigations/cops and
        // freed with this graph, never retained globally after replacement.
        self.reference_names.get_or_init(|| {
            self.references
                .iter()
                .map(|name| name.strip_suffix("()").unwrap_or(name).to_owned())
                .collect()
        })
    }
    fn member(&self, owner: &str, name: &str) -> Option<&str> {
        self.declarations
            .get(owner)?
            .ancestors
            .iter()
            .find_map(|ancestor| {
                self.declarations
                    .get(ancestor)?
                    .members
                    .get(name)
                    .map(String::as_str)
            })
    }
    fn resolve(&self, path: &Node<'_>, nesting: &[String], file: &SourceFile) -> Option<&str> {
        let name = const_name(path, file)?;
        let mut parts = name.split("::");
        let first = parts.next()?;
        let scopes = if path.location().as_slice().starts_with(b"::") || nesting.is_empty() {
            &[][..]
        } else {
            self.lexical_scopes.get(&nesting.join("\0"))?.as_slice()
        };
        let mut declaration = scopes.iter().rev().find_map(|scope| {
            self.declarations
                .get(scope)?
                .members
                .get(first)
                .map(String::as_str)
        });
        if declaration.is_none() {
            if let Some(scope) = scopes.last() {
                declaration = self.member(scope, first);
                if declaration.is_none() && !self.declarations.get(scope)?.module {
                    return None;
                }
            }
            declaration = declaration.or_else(|| self.member("Object", first));
        }
        let mut declaration = declaration?;
        for part in parts {
            if !self.declarations.get(declaration)?.namespace {
                return None;
            }
            declaration = self.member(declaration, part)?;
        }
        Some(declaration)
    }
}
fn const_name(node: &Node<'_>, file: &SourceFile) -> Option<String> {
    if let Some(read) = node.as_constant_read_node() {
        return Some(
            String::from_utf8_lossy(&file.match_pattern_source(read.name().as_slice()))
                .into_owned(),
        );
    }
    let path = node.as_constant_path_node()?;
    let short =
        String::from_utf8_lossy(&file.match_pattern_source(path.name()?.as_slice())).into_owned();
    Some(match path.parent() {
        None => short,
        Some(parent) => format!("{}::{short}", const_name(&parent, file)?),
    })
}
fn namespace_parts<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Option<Node<'pr>>)> {
    if let Some(class) = node.as_class_node() {
        Some((class.constant_path(), class.body()))
    } else {
        node.as_module_node()
            .map(|module| (module.constant_path(), module.body()))
    }
}
impl UnusedPrivateMethod {
    /// The explicit internal seam is not a production index provider.
    fn check_with_index(
        &self,
        index: Option<&Index>,
        root: &Node<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(index) = index else { return };
        let mut visitor = Methods {
            index,
            root,
            file,
            out,
            stack: vec![],
            reports: RefCell::default(),
            literals: OnceCell::new(),
            namespaces: HashMap::new(),
        };
        walk(&mut visitor, root);
    }
}
struct Methods<'a, 'pr> {
    index: &'a Index,
    root: &'a Node<'pr>,
    literals: OnceCell<Literals>,
    namespaces: HashMap<usize, Option<String>>,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    stack: Vec<Node<'pr>>,
    reports: RefCell<HashSet<(usize, usize)>>,
}
impl Methods<'_, '_> {
    fn check(&mut self, node: &Node<'_>) -> Option<Offense> {
        let def = node.as_def_node()?;
        if def.receiver().is_some() {
            return None;
        }
        let name = String::from_utf8_lossy(&self.file.match_pattern_source(def.name().as_slice()))
            .into_owned();
        if IMPLICIT.contains(&name.as_str()) {
            return None;
        }
        let cx = NodeContext {
            ancestors: &self.stack,
            corrections_requested: false,
            require_order_reports: &self.reports,
        };
        if cx.ancestors().any(|a| {
            matches!(
                a,
                Node::DefNode { .. }
                    | Node::BlockNode { .. }
                    | Node::LambdaNode { .. }
                    | Node::SingletonClassNode { .. }
            ) || literal_block(a)
        }) {
            return None;
        }
        let namespace = cx.ancestors().find(|a| namespace_parts(a).is_some())?;
        let key = namespace.location().start_offset();
        let namespace_name = self
            .namespaces
            .entry(key)
            .or_insert_with(|| {
                #[cfg(test)]
                NAMESPACE_RESOLUTIONS.with(|n| n.set(n.get() + 1));
                let (path, _) = namespace_parts(namespace)?;
                // Only actual body edges create lexical scope. Physical source
                // containment is wrong for nodes inside out-of-line heredocs.
                let mut ancestors = cx.ancestors();
                ancestors.find(|a| std::ptr::eq(*a, namespace));
                let mut child = namespace;
                let mut nesting = vec![];
                for ancestor in ancestors {
                    if let Some((outer_path, Some(body))) = namespace_parts(ancestor)
                        && std::mem::discriminant(child) == std::mem::discriminant(&body)
                        && child.location().start_offset() == body.location().start_offset()
                        && child.location().end_offset() == body.location().end_offset()
                    {
                        nesting.push(const_name(&outer_path, self.file)?);
                    }
                    child = ancestor;
                }
                nesting.reverse();
                self.index
                    .resolve(&path, &nesting, self.file)
                    .map(str::to_owned)
            })
            .as_ref()?;
        let namespace = self
            .index
            .declarations
            .get(namespace_name)
            .filter(|d| d.namespace)?;
        let member = format!("{name}()");
        let declaration = self
            .index
            .declarations
            .get(namespace.members.get(&member)?)
            .filter(|d| d.private)?;
        let names = self.index.referenced_names();
        if names.contains(&name) {
            return None;
        }
        let literals = self
            .literals
            .get_or_init(|| Literals::collect(self.root, self.file));
        if literals.names.contains(&name)
            || literals
                .prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
        {
            return None;
        }
        if let Some((owner_name, owner)) = declaration.owner.as_ref().and_then(|key| {
            self.index
                .declarations
                .get(key)
                .filter(|d| d.namespace)
                .map(|d| (key, d))
        }) && (owner.descendants.iter().any(|d| d != owner_name)
            || owner.ancestors.iter().any(|a| {
                a != owner_name
                    && self
                        .index
                        .declarations
                        .get(a)
                        .is_some_and(|d| d.members.contains_key(&member))
            }))
        {
            return None;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            def.def_keyword_loc().start_offset(),
            self.file,
        );
        Some(Offense {
            cop: "Lint/UnusedPrivateMethod",
            severity: UnusedPrivateMethod.severity(),
            message: format!("Private method `{name}` appears to be unused."),
            line,
            col,
            end_line: self
                .file
                .line_of(def.name_loc().end_offset().saturating_sub(1)),
            correction: None,
        })
    }
}
impl<'pr> NodeVisitor<'pr> for Methods<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(offense) = self.check(&node) {
            self.out.push(offense);
        }
        self.stack.push(node);
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}
fn literal_block(node: &Node<'_>) -> bool {
    // RuboCop's block wraps the send/super too: a def in its arguments is
    // inside any_block even though Prism puts the block below the call.
    let block = match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().block(),
        Node::SuperNode { .. } => node.as_super_node().unwrap().block(),
        Node::ForwardingSuperNode { .. } => {
            return node.as_forwarding_super_node().unwrap().block().is_some();
        }
        _ => None,
    };
    block.is_some_and(|b| b.as_block_node().is_some())
}
const IMPLICIT: &[&str] = &[
    "initialize",
    "initialize_copy",
    "initialize_clone",
    "initialize_dup",
    "method_missing",
    "respond_to_missing?",
    "marshal_dump",
    "marshal_load",
    "coerce",
    "instance_variables_to_inspect",
    "inherited",
    "included",
    "extended",
    "prepended",
    "append_features",
    "extend_object",
    "prepend_features",
    "const_missing",
    "const_added",
    "method_added",
    "method_removed",
    "method_undefined",
    "singleton_method_added",
    "singleton_method_removed",
    "singleton_method_undefined",
];
static IDENTIFIER: LazyLock<regex::bytes::Regex> =
    LazyLock::new(|| regex::bytes::Regex::new(r"(?-u)[a-zA-Z_]\w*[?!=]?").unwrap());
static PREFIX: LazyLock<regex::bytes::Regex> =
    LazyLock::new(|| regex::bytes::Regex::new(r"(?-u)[a-zA-Z_]\w*\z").unwrap());
#[derive(Default)]
struct Literals {
    names: HashSet<String>,
    prefixes: HashSet<String>,
}
impl Literals {
    fn scan(&mut self, value: &[u8]) {
        for name in IDENTIFIER.find_iter(value) {
            self.names
                .insert(String::from_utf8_lossy(name.as_bytes()).into_owned());
        }
    }
    fn collect(root: &Node<'_>, file: &SourceFile) -> Self {
        #[cfg(test)]
        LITERAL_WALKS.with(|n| n.set(n.get() + 1));
        struct Visitor<'a> {
            file: &'a SourceFile,
            result: Literals,
            root_literal: Option<usize>,
        }
        impl NodeVisitor<'_> for Visitor<'_> {
            fn enter(&mut self, node: Node<'_>) {
                if let Some(symbol) = node.as_symbol_node() {
                    let raw = symbol.value_loc();
                    let single = symbol
                        .opening_loc()
                        .is_some_and(|o| o.as_slice() == b":'" || o.as_slice().starts_with(b"%s"));
                    let parts = ast::require_literal::parts(
                        raw.as_ref().map_or(symbol.unescaped(), |r| r.as_slice()),
                        symbol.unescaped(),
                        single,
                        self.file,
                    );
                    if let [value] = parts.as_slice() {
                        self.result
                            .names
                            .insert(String::from_utf8_lossy(value).into_owned());
                    } else {
                        for part in parts {
                            self.result.scan(&part);
                        }
                    }
                } else if let Some(string) = node.as_string_node() {
                    let single = string.opening_loc().is_some_and(|o| {
                        o.as_slice() == b"'"
                            || o.as_slice().starts_with(b"%q")
                            || o.as_slice().starts_with(b"%w")
                            || o.as_slice().ends_with(b"'")
                    });
                    for part in ast::require_literal::parts(
                        string.content_loc().as_slice(),
                        string.unescaped(),
                        single,
                        self.file,
                    ) {
                        self.result.scan(&part);
                    }
                } else if let Some(string) = node.as_x_string_node() {
                    for part in ast::require_literal::parts(
                        string.content_loc().as_slice(),
                        string.unescaped(),
                        false,
                        self.file,
                    ) {
                        self.result.scan(&part);
                    }
                } else if let Some(regexp) = node.as_regular_expression_node() {
                    for part in ast::require_literal::parts(
                        regexp.content_loc().as_slice(),
                        regexp.unescaped(),
                        false,
                        self.file,
                    ) {
                        self.result.scan(&part);
                    }
                } else if node.as_source_file_node().is_some() {
                    self.result
                        .scan(self.file.source_path().as_os_str().as_encoded_bytes());
                }
                let parts = node
                    .as_interpolated_string_node()
                    .map(|n| n.parts())
                    .or_else(|| node.as_interpolated_symbol_node().map(|n| n.parts()));
                if let Some(parts) = parts
                    && self.root_literal != Some(node.location().start_offset())
                    && parts.iter().any(|p| p.as_string_node().is_none())
                    && let Some(leading) = parts.iter().next().and_then(|p| p.as_string_node())
                {
                    let values = ast::require_literal::parts(
                        leading.content_loc().as_slice(),
                        leading.unescaped(),
                        false,
                        self.file,
                    );
                    if let Some(value) = values.first()
                        && let Some(prefix) = PREFIX.find(value)
                    {
                        self.result
                            .prefixes
                            .insert(String::from_utf8_lossy(prefix.as_bytes()).into_owned());
                    }
                }
            }
            fn leave(&mut self) {}
        }
        // each_descendant excludes the parser AST root itself. A top-level
        // dstr/dsym can contain class/def nodes in its interpolation.
        let root_literal = root
            .as_program_node()
            .and_then(|p| {
                let body = p.statements().body();
                (body.len() == 1).then(|| body.iter().next().unwrap())
            })
            .filter(|n| {
                n.as_interpolated_string_node().is_some()
                    || n.as_interpolated_symbol_node().is_some()
            })
            .map(|n| n.location().start_offset());
        let mut visitor = Visitor {
            file,
            result: Self::default(),
            root_literal,
        };
        walk(&mut visitor, root);
        visitor.result
    }
}
#[cfg(test)]
mod tests {
    //! Ported RuboCop unused_private_method_spec.rb examples (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn supplied(source: &str, index: Option<&Index>) -> Vec<Offense> {
        supplied_bytes(source.as_bytes(), index)
    }
    fn supplied_bytes(source: &[u8], index: Option<&Index>) -> Vec<Offense> {
        let file = SourceFile::new("/lib/current.rb".into(), source.to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert!(parsed.errors().next().is_none(), "{source:?}");
        file.record_encoding(&parsed);
        let mut out = vec![];
        UnusedPrivateMethod.check_with_index(index, &parsed.node(), &file, &mut out);
        out.sort_by_key(|o| (o.line, o.col));
        out
    }
    fn service_index() -> Index {
        serde_json::from_value(serde_json::json!({
            "declarations": {
                "Object": {"namespace": true, "module": false, "private": false, "owner": null, "ancestors": ["Object"], "descendants": [], "members": {"Service": "Service"}},
                "Service": {"namespace": true, "module": false, "private": false, "owner": null, "ancestors": ["Service", "Object"], "descendants": [], "members": {"helper()": "Service#helper()"}},
                "Service#helper()": {"namespace": false, "module": false, "private": true, "owner": "Service", "ancestors": [], "descendants": [], "members": {}}
            }, "lexical_scopes": {}, "references": []
        })).unwrap()
    }
    #[test]
    fn index_reference_set_is_shared_and_replaced_without_retaining_old_graphs() {
        let first = service_index();
        let mut second = service_index();
        second.references = vec!["helper()".into(), "unrelated".into()];
        assert!(std::ptr::eq(
            first.referenced_names(),
            first.referenced_names()
        ));
        assert_ne!(first.referenced_names(), second.referenced_names());
        assert!(second.referenced_names().contains("helper"));
        let source = "class Service; private; def helper; end; end\n";
        for (index, expected) in [
            (Some(&first), 1),
            (Some(&second), 0),
            (None, 0),
            (Some(&first), 1),
        ] {
            assert_eq!(supplied(source, index).len(), expected);
        }
    }
    #[test]
    fn literal_walk_is_lazy_once_per_investigation_and_namespace_resolution_is_bounded() {
        let index = service_index();
        LITERAL_WALKS.with(|n| n.set(0));
        NAMESPACE_RESOLUTIONS.with(|n| n.set(0));
        assert!(supplied("class Service; def public_method; end; end", Some(&index)).is_empty());
        LITERAL_WALKS.with(|n| assert_eq!(n.get(), 0));
        let source = format!(
            "class Service; private; {}; end",
            "def helper; end;".repeat(200)
        );
        assert_eq!(supplied(&source, Some(&index)).len(), 200);
        LITERAL_WALKS.with(|n| assert_eq!(n.get(), 1));
        NAMESPACE_RESOLUTIONS.with(|n| assert_eq!(n.get(), 2));
    }
    #[test]
    fn literals_and_prefixes_never_leak_across_investigations() {
        let index = service_index();
        let source = "class Service; private; def helper; end; end\n";
        for (tail, expected) in [
            (":helper", 0),
            ("\"help#{type}\"", 0),
            ("\"#{type}help\"", 1),
            ("", 1),
        ] {
            assert_eq!(
                supplied(&format!("{source}{tail}"), Some(&index)).len(),
                expected
            );
        }
    }
    #[test]
    fn public_missing_and_nonnamespace_declarations_are_not_candidates() {
        let source = "class Service; private; def helper; end; end";
        let mut index = service_index();
        index
            .declarations
            .get_mut("Service#helper()")
            .unwrap()
            .private = false;
        assert!(supplied(source, Some(&index)).is_empty());
        index.declarations.remove("Service#helper()");
        assert!(supplied(source, Some(&index)).is_empty());
        let mut index = service_index();
        index.declarations.get_mut("Service").unwrap().namespace = false;
        assert!(supplied(source, Some(&index)).is_empty());
        assert!(
            crate::pipeline::lint_str(source)
                .iter()
                .all(|o| o.cop != "Lint/UnusedPrivateMethod")
        );
    }
    #[test]
    fn all_pinned_graphs_and_source_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/unused_private_method.json"
        ))
        .unwrap();
        for row in rows {
            let index: Option<Index> = crate::fixture::value(&row["index"]);
            let source: Vec<u8> = row["source"]
                .as_str()
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| crate::fixture::value(&row["source_bytes"]));
            let found: Vec<_> = supplied_bytes(&source, index.as_ref())
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
                "{}:{} {source:?}",
                row["origin"], row["line"]
            );
            assert!(supplied_bytes(&source, None).is_empty());
        }
    }
    #[test]
    fn reports_real_private_declaration_only_with_supplied_index() {
        let source = "class Service\n  private\n  def helper\n  end\nend\n";
        assert!(supplied(source, None).is_empty());
        let found = supplied(source, Some(&service_index()));
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 3));
        assert_eq!(
            found[0].message,
            "Private method `helper` appears to be unused."
        );
        assert!(found[0].correction.is_none());
    }
}
