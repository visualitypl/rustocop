//! `Style/OrAssignment` — replace redundant variable initialization with `||=`.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct OrAssignment;
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
impl Cop for OrAssignment {
    fn name(&self) -> &'static str {
        "Style/OrAssignment"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((variable, default, conditional)) = candidate(node) else {
            return;
        };
        let correction = cx
            .corrections_requested()
            .then(|| correction(node, variable, &default, conditional, cx, file))
            .flatten();
        super::negated_if::report(
            self,
            node,
            file,
            "Use the double pipe equals operator `||=` instead.".into(),
            correction,
            out,
        );
    }
}

// Matching read/write kind, name and lexical depth is sufficient here: no
// declarations cross one another. In particular a captured local's identity
// must not be confused with a block-owned local with the same spelling.
type Variable<'pr> = (Kind, &'pr [u8], u32);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Local,
    Instance,
    Class,
    Global,
}
const REWRITE_LIMIT: usize = 16 * 1024;

fn correction(
    node: &Node<'_>,
    variable: Variable<'_>,
    default: &Node<'_>,
    conditional: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    // Class reads may raise where ||= initializes. Unconditional self-writes
    // also raise on frozen instance receivers or invoke global trace_var hooks.
    if loc.end_offset() - loc.start_offset() > REWRITE_LIMIT
        || variable.0 == Kind::Class
        || (matches!(variable.0, Kind::Instance | Kind::Global) && !conditional)
        || matches!(
            default,
            Node::ReturnNode { .. }
                | Node::BreakNode { .. }
                | Node::NextNode { .. }
                | Node::RedoNode { .. }
                | Node::RetryNode { .. }
        )
        || (conditional && !discarded(node, cx))
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    // Reuse cached scope analysis to verify that moving an unless target to
    // the start cannot introduce an earlier local binding or capture.
    if variable.0 == Kind::Local
        && !file
            .visible_locals(root)
            .declared_before(variable.1, node, cx, loc.start_offset())
    {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + loc.end_offset() - loc.start_offset()));
    if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, loc.start_offset()) {
        return None;
    }
    let name = std::str::from_utf8(variable.1).ok()?;
    let text = std::str::from_utf8(default.location().as_slice()).ok()?;
    // The original conditional protects low-precedence defaults. Retain an
    // expression envelope for arbitrary expressions (including command calls).
    let default = if crate::cops::ast::is_basic_literal(default)
        || read(default).is_some()
        || default.as_parentheses_node().is_some()
        || default.as_string_node().is_some()
    {
        text.into()
    } else {
        format!("({text})")
    };
    super::negated_if::finish(
        Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: format!("{name} ||= {default}"),
        },
        cx,
        file,
    )
}
fn read<'pr>(node: &Node<'pr>) -> Option<Variable<'pr>> {
    Some(match node {
        Node::LocalVariableReadNode { .. } => {
            let n = node.as_local_variable_read_node()?;
            (Kind::Local, n.name().as_slice(), n.depth())
        }
        Node::InstanceVariableReadNode { .. } => (
            Kind::Instance,
            node.as_instance_variable_read_node()?.name().as_slice(),
            0,
        ),
        Node::ClassVariableReadNode { .. } => (
            Kind::Class,
            node.as_class_variable_read_node()?.name().as_slice(),
            0,
        ),
        Node::GlobalVariableReadNode { .. } => (
            Kind::Global,
            node.as_global_variable_read_node()?.name().as_slice(),
            0,
        ),
        _ => return None,
    })
}
fn write<'pr>(node: &Node<'pr>) -> Option<(Variable<'pr>, Node<'pr>)> {
    Some(match node {
        Node::LocalVariableWriteNode { .. } => {
            let n = node.as_local_variable_write_node()?;
            ((Kind::Local, n.name().as_slice(), n.depth()), n.value())
        }
        Node::InstanceVariableWriteNode { .. } => {
            let n = node.as_instance_variable_write_node()?;
            ((Kind::Instance, n.name().as_slice(), 0), n.value())
        }
        Node::ClassVariableWriteNode { .. } => {
            let n = node.as_class_variable_write_node()?;
            ((Kind::Class, n.name().as_slice(), 0), n.value())
        }
        Node::GlobalVariableWriteNode { .. } => {
            let n = node.as_global_variable_write_node()?;
            ((Kind::Global, n.name().as_slice(), 0), n.value())
        }
        _ => return None,
    })
}
fn only<'pr>(body: Option<ruby_prism::StatementsNode<'pr>>) -> Option<Node<'pr>> {
    let body = body?;
    (body.body().len() == 1).then(|| body.body().first().unwrap())
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<(Variable<'pr>, Node<'pr>, bool)> {
    if let Some((variable, value)) = write(node) {
        let (predicate, truthy, default) = if let Some(condition) = value.as_if_node() {
            (
                condition.predicate(),
                only(condition.statements())?,
                only(condition.subsequent()?.as_else_node()?.statements())?,
            )
        } else {
            let condition = value.as_unless_node()?;
            (
                condition.predicate(),
                only(condition.else_clause()?.statements())?,
                only(condition.statements())?,
            )
        };
        if default.as_if_node().is_some() || default.as_unless_node().is_some() {
            return None;
        }
        return (read(&predicate)? == variable && read(&truthy)? == variable)
            .then_some((variable, default, false));
    }
    let (predicate, assignment) = if let Some(n) = node.as_unless_node() {
        if n.else_clause().is_some_and(|e| e.statements().is_some()) {
            return None;
        }
        (n.predicate(), only(n.statements())?)
    } else {
        let n = node.as_if_node()?;
        if n.statements().is_some() {
            return None;
        }
        (
            n.predicate(),
            only(n.subsequent()?.as_else_node()?.statements())?,
        )
    };
    let (variable, default) = write(&assignment)?;
    (read(&predicate)? == variable).then_some((variable, default, true))
}

// An unless/empty-then returns nil on its skipped branch; ||= returns the
// existing truthy variable. Only statement-discarding contexts permit this fix.
fn discarded(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    for parent in cx.ancestors() {
        if let Some(body) = parent.as_statements_node() {
            if body
                .body()
                .last()
                .is_some_and(|n| n.location().start_offset() != child.location().start_offset())
            {
                return true;
            }
        } else if parent.as_program_node().is_some() {
            return true;
        } else if !matches!(parent, Node::ParenthesesNode { .. }) {
            return false;
        }
        child = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/or_assignment_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn every_pinned_example_has_exact_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/or_assignment_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 25);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == OrAssignment.name())
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "{source}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{source}"
            );
        }
    }
    #[test]
    fn pinned_variable_forms_positions_and_corrections() {
        for name in ["foo", "@foo", "@@foo", "$foo"] {
            for expression in [
                format!("{name} = {name} ? {name} : 'default'"),
                format!("{name} = if {name}\n  {name}\nelse\n  'default'\nend"),
                format!("{name} = 'default' unless {name}"),
                format!("unless {name}\n  {name} = 'default'\nend"),
                format!("if {name}\nelse\n  {name} = 'default'\nend"),
            ] {
                let source = format!("{name} = nil\n  {expression}\n");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == OrAssignment.name())
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3), "{source}");
                assert_eq!(
                    found[0].message,
                    "Use the double pipe equals operator `||=` instead."
                );
                if name.starts_with("@@")
                    || ((name.starts_with('$') || name.starts_with('@'))
                        && expression.starts_with(&format!("{name} = "))
                        && !expression.ends_with(&format!("unless {name}")))
                {
                    assert!(
                        crate::cops::testing::correction_offenses(&OrAssignment, &source)[0]
                            .correction
                            .is_none()
                    );
                } else {
                    assert_correction(
                        &OrAssignment,
                        &source,
                        &format!("{name} = nil\n  {name} ||= 'default'\n"),
                    );
                }
            }
        }
    }
    #[test]
    fn reversed_unless_expression_is_normalized_like_upstream() {
        assert_correction(
            &OrAssignment,
            "foo = unless foo\n 'default'\nelse\n foo\nend",
            "foo ||= 'default'",
        );
    }
    #[test]
    fn global_write_hooks_and_class_initialization_keep_findings_without_fixes() {
        for source in [
            "@foo = @foo ? @foo : 1", // Frozen receivers still raise on the original self-write.
            "$foo = $foo ? $foo : 1",
            "@@foo = @@foo ? @@foo : 1",
            "unless @@foo\n @@foo = 1\nend",
        ] {
            let found = crate::cops::testing::correction_offenses(&OrAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn consumed_unless_keeps_nil_return_and_standalone_statements_can_fix() {
        for source in [
            "result = (@foo = 1 unless @foo)",
            "def f; unless @foo; @foo = 1; end; end",
            "items.map { @foo = 1 unless @foo }",
            "-> { unless @foo; @foo = 1; end }",
            "def f; if @foo; else; @foo = 1; end; end",
            "defined?((@foo = 1 unless @foo))",
        ] {
            let found = crate::cops::testing::correction_offenses(&OrAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &OrAssignment,
            "def f; @foo = 1 unless @foo; :done; end",
            "def f; @foo ||= 1; :done; end",
        );
    }
    #[test]
    fn binding_capture_default_precedence_and_control_flow() {
        for (before, after) in [
            ("foo = foo ? foo : -> { foo }", "foo ||= (-> { foo })"),
            (
                "foo = nil; proc { foo = foo ? foo : 3 }",
                "foo = nil; proc { foo ||= 3 }",
            ),
            (
                "foo = nil; proc { |foo| foo = foo ? foo : 3 }",
                "foo = nil; proc { |foo| foo ||= 3 }",
            ),
            (
                "foo = nil; proc { |;foo| foo = foo ? foo : 3 }",
                "foo = nil; proc { |;foo| foo ||= 3 }",
            ),
            (
                "foo = if foo; foo; else; false or 3; end",
                "foo ||= (false or 3)",
            ),
        ] {
            assert_correction(&OrAssignment, before, after);
        }
    }
    #[test]
    fn comments_heredocs_directives_and_line_literals_withhold_atomic_edits() {
        for source in [
            "def f; foo = if foo; foo; else; return 3; end; end",
            "foo = if foo # important\n foo\nelse\n 3\nend",
            "foo = foo ? foo : <<~TEXT\n  value\nTEXT\n",
            "foo = if foo\n foo\nelse\n __LINE__\nend",
            "foo = if foo\n foo\nelse\n 3\nend\np __LINE__",
            "foo = foo ? foo : 1 # rubocop:disable Lint/Debugger\n",
            "foo = foo ? foo : DATA.pos\n__END__\npayload\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&OrAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn bounded_lazy_rewrites_and_one_cached_scope_analysis() {
        use crate::cops::variables::VISIBLE_LOCAL_BUILDS;
        let source = "foo = foo ? foo : 1\n".repeat(1000);
        REWRITE_BYTES.with(|n| n.set(0));
        VISIBLE_LOCAL_BUILDS.with(|n| n.set(0));
        assert_eq!(
            lint_str(&source)
                .iter()
                .filter(|o| o.cop == OrAssignment.name())
                .count(),
            1000
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 0));
        let found = crate::cops::testing::correction_offenses(&OrAssignment, &source);
        assert_eq!(found.len(), 1000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        REWRITE_BYTES.with(|n| assert!(n.get() <= source.len()));
        VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 1));
        let huge = format!("foo = foo ? foo : '{}'", "x".repeat(20_000));
        REWRITE_BYTES.with(|n| n.set(0));
        assert!(
            crate::cops::testing::correction_offenses(&OrAssignment, &huge)[0]
                .correction
                .is_none()
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
    }
    #[test]
    fn lookalikes_do_not_match() {
        for source in [
            "foo = bar ? foo : 3",
            "foo = foo ? bar : 3",
            "foo = 3 unless bar",
            "unless foo\n bar = 3\nend",
            "foo = if foo\n foo\nelsif bar\n 2\nelse\n 3\nend",
            "CONST = CONST ? CONST : 3",
            "obj.x = obj.x ? obj.x : 3",
            "obj[0] = obj[0] ? obj[0] : 3",
            "a,b = a ? a : 3",
            "foo = foo ? foo : bar ? bar : 3",
            "unless @foo\n @foo = 1\n work\nend",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == OrAssignment.name()),
                "{source}"
            );
        }
    }
}
