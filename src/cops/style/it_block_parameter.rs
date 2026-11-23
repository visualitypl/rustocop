//! `Style/ItBlockParameter` — fixed Ruby 3.4, allow_single_line policy.
use super::implicit_parameters;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ItBlockParameter;
impl Cop for ItBlockParameter {
    fn name(&self) -> &'static str {
        "Style/ItBlockParameter"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    /// Upstream's Style/For lists this cop as autocorrect-incompatible and
    /// runs first.
    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &["Style/For"]
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some(b) = implicit_parameters::block(node) {
            if b.parameters
                .is_some_and(|p| p.as_it_parameters_node().is_some())
                && file.line_of(b.opening.start_offset()) != file.line_of(b.closing.start_offset())
            {
                implicit_parameters::report(
                    self,
                    node,
                    cx,
                    file,
                    out,
                    "Avoid using `it` block parameter for multi-line blocks.",
                );
            }
            return;
        }
        if !numbered_read(node, cx) {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `it` block parameter.".into(),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| {
                    let root = cx.ancestors().next_back()?;
                    file.it_parameter_facts(root)
                        .edits
                        .get(&node.location().start_offset())
                        .cloned()
                })
                .flatten(),
        });
    }
}
/// Pinned matching excludes inner block bodies, but includes a nested lambda's
/// header/default expressions before its opening delimiter. It deliberately
/// excludes a read that is the body's root (`each_descendant`, not `each_node`).
fn numbered_read(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    if !node
        .as_local_variable_read_node()
        .is_some_and(|r| r.name().as_slice() == b"_1")
    {
        return false;
    }
    let mut child = node;
    for ancestor in cx.ancestors() {
        if let Some(b) = implicit_parameters::block(ancestor) {
            if ancestor.as_lambda_node().is_some() && child.as_block_parameters_node().is_some() {
                child = ancestor;
                continue;
            }
            return b
                .parameters
                .and_then(|p| p.as_numbered_parameters_node())
                .is_some_and(|p| p.maximum() == 1)
                && !implicit_parameters::body_root(b.body, node);
        }
        child = ancestor;
    }
    false
}
/// Correction-only all-use edits from the shared VariableForce analysis. One
/// complete structural walk owns node edges; byte containment cannot correctly
/// assign deferred heredoc interpolations in definition receivers/headers.
#[derive(Default)]
pub(crate) struct Facts {
    edits: std::collections::HashMap<usize, crate::offense::Correction>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        use crate::{
            cops::{
                variables::{ReferenceKind, ScopeAnalysis, ScopeId, scope_owns_child},
                walk::{NodeVisitor, walk},
            },
            offense::Correction,
        };
        use std::collections::{HashMap, HashSet};
        let Some(program) = root.as_program_node() else {
            return Self::default();
        };
        let analysis = ScopeAnalysis::of(&program);
        let mut scopes = HashMap::new();
        let mut it_locals = HashSet::new();
        for id in analysis.scopes_in_exit_order() {
            let s = analysis.scope(id);
            scopes.insert(
                (
                    s.node.location().start_offset(),
                    s.node.location().end_offset(),
                    std::mem::discriminant(&s.node),
                ),
                id,
            );
            if s.variables().any(|v| analysis.variable(v).name == b"it") {
                it_locals.insert(id);
            }
        }
        struct Frame<'pr> {
            node: Node<'pr>,
            active: ScopeId,
            body: ScopeId,
        }
        struct Index<'a, 'pr> {
            scopes: &'a HashMap<(usize, usize, std::mem::Discriminant<Node<'pr>>), ScopeId>,
            frames: Vec<Frame<'pr>>,
            bare_it: HashSet<ScopeId>,
            reads: HashMap<usize, ScopeId>,
            metadata: bool,
            visits: usize,
        }
        fn metadata(name: &[u8]) -> bool {
            matches!(
                name,
                b"parameters"
                    | b"local_variables"
                    | b"local_variable_get"
                    | b"local_variable_set"
                    | b"local_variable_defined?"
                    | b"define_method"
                    | b"define_singleton_method"
            )
        }
        impl<'pr> NodeVisitor<'pr> for Index<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                self.visits += 1;
                let own = self
                    .scopes
                    .get(&(
                        node.location().start_offset(),
                        node.location().end_offset(),
                        std::mem::discriminant(&node),
                    ))
                    .copied();
                let active = self
                    .frames
                    .last()
                    .map(|p| {
                        if scope_owns_child(&p.node, &node) {
                            p.body
                        } else {
                            p.active
                        }
                    })
                    .or(own)
                    .expect("root scope");
                let body = own.unwrap_or(active);
                if node
                    .as_local_variable_read_node()
                    .is_some_and(|r| r.name().as_slice() == b"_1")
                {
                    self.reads.insert(node.location().start_offset(), active);
                }
                if node.as_call_node().is_some_and(|c| {
                    c.receiver().is_none() && c.is_variable_call() && c.name().as_slice() == b"it"
                }) {
                    self.bare_it.insert(active);
                }
                self.metadata |= node.as_implicit_node().is_some()
                    || node
                        .as_symbol_node()
                        .is_some_and(|s| metadata(s.unescaped()))
                    || node
                        .as_call_node()
                        .is_some_and(|c| metadata(c.name().as_slice()))
                    || node
                        .as_call_or_write_node()
                        .is_some_and(|c| metadata(c.read_name().as_slice()))
                    || node
                        .as_call_and_write_node()
                        .is_some_and(|c| metadata(c.read_name().as_slice()))
                    || node
                        .as_call_operator_write_node()
                        .is_some_and(|c| metadata(c.read_name().as_slice()));
                self.frames.push(Frame { node, active, body });
            }
            fn leave(&mut self) {
                self.frames.pop();
            }
        }
        let mut index = Index {
            scopes: &scopes,
            frames: Vec::new(),
            bare_it: HashSet::new(),
            reads: HashMap::new(),
            metadata: false,
            visits: 0,
        };
        walk(&mut index, root);
        let mut facts = Self::default();
        #[cfg(test)]
        {
            facts.visits = index.visits;
        }
        if index.metadata {
            return facts;
        }
        let visible_it = |mut scope| loop {
            if it_locals.contains(&scope) {
                break true;
            }
            let s = analysis.scope(scope);
            if !s.kind.is_block() {
                break false;
            }
            let Some(parent) = s.parent else {
                break false;
            };
            scope = parent;
        };
        for id in analysis.scopes_in_exit_order() {
            let scope = analysis.scope(id);
            if !implicit_parameters::block(&scope.node)
                .and_then(|b| b.parameters)
                .and_then(|p| p.as_numbered_parameters_node())
                .is_some_and(|p| p.maximum() == 1)
                || visible_it(id)
                || index.bare_it.contains(&id)
            {
                continue;
            }
            let Some(var) = scope
                .variables()
                .map(|v| analysis.variable(v))
                .find(|v| v.name == b"_1")
            else {
                continue;
            };
            if !var.assignments.is_empty() || var.has_pattern_bindings {
                continue;
            }
            let edits: Option<Vec<_>> = var
                .references
                .iter()
                .map(|&r| {
                    let r = analysis.reference(r);
                    let active = *index.reads.get(&r.range.start)?;
                    if r.kind != ReferenceKind::Read
                        || visible_it(active)
                        || file.bytes[r.range.clone()] != *b"_1"
                    {
                        return None;
                    }
                    Some(Correction {
                        range: r.range.clone(),
                        replacement: "it".into(),
                    })
                })
                .collect();
            // Each read is its own offense and edit, as upstream. Upstream
            // leaves a disabled read as `_1` beside `it`, which is invalid;
            // here a block with a disabled read is not corrected at all.
            let Some(edits) = edits else { continue };
            if edits.iter().all(|edit| {
                let line = file.line_of(edit.range.start);
                file.cop_enabled_for_lines("Style/ItBlockParameter", line, line)
            }) {
                for edit in edits {
                    facts.edits.insert(edit.range.start, edit);
                }
            }
        }
        facts
    }
}

pub(crate) static IT_PARAMETER_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn it_parameter_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        IT_PARAMETER_FACTS.get(self, || Facts::build(root, self))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/it_block_parameter_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ItBlockParameter.name())
            .collect()
    }
    #[test]
    fn pinned_multiline_it_is_detection_only() {
        let f = offenses("block do\n  do_something(it)\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].line, f[0].col, f[0].end_line), (1, 1, 3));
        assert_eq!(
            f[0].message,
            "Avoid using `it` block parameter for multi-line blocks."
        );
        assert!(f[0].correction.is_none());
    }
    #[test]
    fn pinned_numbered_read_positions() {
        for (source, positions) in [
            ("block { do_something(_1) }", vec![(1, 22)]),
            (
                "block do\n  foo(_1)\n  bar(_1)\nend\n",
                vec![(2, 7), (3, 7)],
            ),
            ("foo { bar(_1, _2) }.baz { qux(_1) }", vec![(1, 31)]),
        ] {
            let f = offenses(source);
            assert_eq!(
                f.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
            assert!(f.iter().all(|o| o.message == "Use `it` block parameter."));
        }
    }
    #[test]
    fn implicit_parameters_have_variable_force_identities() {
        use crate::cops::variables::{ReferenceKind, ScopeAnalysis};
        let source = "foo { use(_1); def (_1).tag; end; class << _1; end }; foo { use(it) }; foo { use(_2) }";
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let a = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        let vars: Vec<_> = a
            .scopes_in_exit_order()
            .flat_map(|s| a.scope(s).variables())
            .map(|id| a.variable(id))
            .filter(|v| [b"_1".as_slice(), b"_2", b"it"].contains(&v.name))
            .collect();
        assert_eq!(vars.len(), 4);
        let one = vars
            .iter()
            .find(|v| v.name == b"_1" && !v.references.is_empty())
            .unwrap();
        assert!(one.is_parameter());
        assert_eq!(
            one.references
                .iter()
                .filter(|&&r| a.reference(r).kind == ReferenceKind::Read)
                .count(),
            3
        );
        assert!(!one.captured_by_block);
    }
    #[test]
    fn pinned_canonical_corrections_are_atomic() {
        for (before, after) in [
            ("block { do_something(_1) }", "block { do_something(it) }"),
            (
                "block do\n  foo(_1)\n  bar(_1)\nend\n",
                "block do\n  foo(it)\n  bar(it)\nend\n",
            ),
            (
                "foo { bar(_1, _2) }.baz { qux(_1) }",
                "foo { bar(_1, _2) }.baz { qux(it) }",
            ),
        ] {
            crate::cops::testing::assert_correction(&ItBlockParameter, before, after);
        }
    }
    #[test]
    fn independent_sibling_and_closed_scopes_do_not_inherit_collisions() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "outer { |it| foo { use(_1) } }; bar { use(_1) }",
                "outer { |it| foo { use(_1) } }; bar { use(it) }",
            ),
            (
                "it=7; def f; foo { use(_1) }; end",
                "it=7; def f; foo { use(it) }; end",
            ),
            (
                "foo { use(_1); inner { |it| use(it) } }",
                "foo { use(it); inner { |it| use(it) } }",
            ),
            (
                "foo { use(_1); inner { |x=1| x } }",
                "foo { use(it); inner { |x=1| x } }",
            ),
            (
                "foo { use(_1); inner { |(it,x)| x } }",
                "foo { use(it); inner { |(it,x)| x } }",
            ),
            (
                "foo { use(_1); ->(x=1) { x } }",
                "foo { use(it); ->(x=1) { x } }",
            ),
            (
                "foo { use(_1); inner { it } }",
                "foo { use(it); inner { it } }",
            ),
            (
                "foo { use(_1); it(); self.it; :it }",
                "foo { use(it); it(); self.it; :it }",
            ),
            (
                "foo { use(_1); def f(x=it); it; end }",
                "foo { use(it); def f(x=it); it; end }",
            ),
            (
                "foo { use(_1); class C; it; end }",
                "foo { use(it); class C; it; end }",
            ),
        ] {
            assert_correction(&ItBlockParameter, before, after);
        }
    }
    #[test]
    fn all_control_flow_uses_are_renamed_without_reordering() {
        use crate::cops::testing::assert_correction;
        for prefix in ["proc", "lambda", "->", "xs.map", "Proc.new", "obj&.each"] {
            for body in [
                "return _1",
                "break _1",
                "next _1",
                "use(_1); _1",
                "(_1)",
                "[_1, _1]",
                "_1.foo { |x| x }",
                "_1 ? use(_1) : other",
                "begin; use(_1); rescue; use(_1); ensure; use(_1); end",
            ] {
                let before = format!("{prefix} {{ {body} }}");
                assert_correction(&ItBlockParameter, &before, &before.replace("_1", "it"));
            }
        }
    }
    #[test]
    fn deferred_headers_and_closed_defaults_have_structural_scope_ownership() {
        use crate::cops::testing::assert_correction;
        for header in [
            "def (build(<<~TXT)).tag; end",
            "class << build(<<~TXT); end",
            "class C < build(<<~TXT); end",
            "class build(<<~TXT)::C; end",
            "module build(<<~TXT)::M; end",
        ] {
            let before = format!("foo {{ use(_1); {header} }}\n#{{_1}}\nTXT\n");
            assert_correction(&ItBlockParameter, &before, &before.replace("_1", "it"));
            // An `it` binding introduced in the outer header would capture the
            // newly spelled read, even past the inline definition's end.
            let collision = format!("foo {{ use(_1); {header} }}\n#{{it=7}}\nTXT\n");
            assert_correction(&ItBlockParameter, &collision, &collision);
        }
        for closed in [
            "def f(x=<<~TXT); x; end",
            "def f = <<~TXT",
            "class C; <<~TXT; end",
            "module M; <<~TXT; end",
            "class << self; <<~TXT; end",
        ] {
            let before = format!("foo {{ use(_1); {closed} }}\n#{{it=7}}\nTXT\n");
            assert_correction(
                &ItBlockParameter,
                &before,
                &before.replace("use(_1)", "use(it)"),
            );
        }
    }
    #[test]
    fn fixed_34_prism_binding_witness_not_merely_parser_success() {
        use crate::cops::variables::ScopeAnalysis;
        let before = "it=99; p = proc { use(_1) }";
        let after = before.replace("_1", "it");
        for (source, implicit) in [(before, true), (after.as_str(), false)] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let a = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let scope = a
                .scopes_in_exit_order()
                .find(|&s| a.scope(s).kind.is_block())
                .unwrap();
            let b = implicit_parameters::block(&a.scope(scope).node).unwrap();
            assert_eq!(
                b.parameters
                    .is_some_and(|p| p.as_numbered_parameters_node().is_some()),
                implicit
            );
            let block_vars: Vec<_> = a.scope(scope).variables().collect();
            assert_eq!(block_vars.len(), usize::from(implicit));
            if !implicit {
                let outer = a.scope(scope).parent.unwrap();
                let it = a
                    .scope(outer)
                    .variables()
                    .map(|id| a.variable(id))
                    .find(|v| v.name == b"it")
                    .unwrap();
                assert!(it.captured_by_block);
                assert_eq!(it.references.len(), 1); // naive edit changed binding AND proc arity
            }
        }
        crate::cops::testing::assert_correction(&ItBlockParameter, before, before);
    }
    #[test]
    fn invalid_nested_implicit_and_explicit_combinations_are_not_corrected() {
        for source in [
            "foo { use(_1); inner { use(_1) } }",
            "foo { use(_1); inner { |x=_1| x } }",
            "foo { use(_1); ->(x=_1) { x } }",
            "foo { |x| use(_1) }",
            "foo { || use(_1) }",
            "foo { use(_1); it }",
        ] {
            assert!(
                ruby_prism::parse(source.as_bytes()).errors().count() > 0,
                "{source}"
            );
            assert!(
                crate::cops::testing::correction_offenses(&ItBlockParameter, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn pinned_clean_forms_and_body_root_quirk() {
        for source in [
            "collection.each\n          .foo { puts it }",
            "block { do_something(it) }",
            "block { |arg| do_something(arg) }",
            "block { do_something(_1, _2) }",
            "block { do_something(_2) }",
            "block { _1 }",
            "block { |arg| other { |x,y| arg } }",
            "block do |arg|\n foo(arg)\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
