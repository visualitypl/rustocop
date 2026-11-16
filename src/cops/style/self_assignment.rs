//! `Style/SelfAssignment` — prefer compound variable assignment.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct SelfAssignment;
impl Cop for SelfAssignment {
    fn name(&self) -> &'static str {
        "Style/SelfAssignment"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Layout/SpaceAroundOperators"]
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((variable, value, equal)) = write(node) else {
            return;
        };
        let value_loc = value.location();
        let Some(rhs) = unparenthesized(value) else {
            return;
        };
        let Some((operator, operand)) = operation(&rhs, variable) else {
            return;
        };
        let correction = cx
            .corrections_requested()
            .then(|| {
                correction(
                    node,
                    Parts {
                        variable,
                        value: value_loc,
                        equal,
                        operator,
                        operand: &operand,
                    },
                    cx,
                    file,
                )
            })
            .flatten();
        super::negated_if::report(
            self,
            node,
            file,
            format!(
                "Use self-assignment shorthand `{}=`.",
                String::from_utf8_lossy(operator)
            ),
            correction,
            out,
        );
    }
}
// No target is moved across a scope. Match lexical depth as well as spelling;
// correction additionally reuses the cached visible-local analysis.
type Variable<'pr> = (u8, &'pr [u8], u32);
fn write<'pr>(node: &Node<'pr>) -> Option<(Variable<'pr>, Node<'pr>, Location<'pr>)> {
    Some(match node {
        Node::LocalVariableWriteNode { .. } => {
            let n = node.as_local_variable_write_node()?;
            (
                (0, n.name().as_slice(), n.depth()),
                n.value(),
                n.operator_loc(),
            )
        }
        Node::InstanceVariableWriteNode { .. } => {
            let n = node.as_instance_variable_write_node()?;
            ((1, n.name().as_slice(), 0), n.value(), n.operator_loc())
        }
        Node::ClassVariableWriteNode { .. } => {
            let n = node.as_class_variable_write_node()?;
            ((2, n.name().as_slice(), 0), n.value(), n.operator_loc())
        }
        _ => return None,
    })
}
fn read<'pr>(node: &Node<'pr>) -> Option<Variable<'pr>> {
    Some(match node {
        Node::LocalVariableReadNode { .. } => {
            let n = node.as_local_variable_read_node()?;
            (0, n.name().as_slice(), n.depth())
        }
        Node::InstanceVariableReadNode { .. } => (
            1,
            node.as_instance_variable_read_node()?.name().as_slice(),
            0,
        ),
        Node::ClassVariableReadNode { .. } => {
            (2, node.as_class_variable_read_node()?.name().as_slice(), 0)
        }
        _ => return None,
    })
}
fn unparenthesized(node: Node<'_>) -> Option<Node<'_>> {
    let mut current = node;
    while let Some(n) = current.as_parentheses_node() {
        let body = n.body()?.as_statements_node()?;
        if body.body().len() != 1 {
            return None;
        }
        current = body.body().first()?;
    }
    Some(current)
}
fn operation<'pr>(node: &Node<'pr>, variable: Variable<'pr>) -> Option<(&'pr [u8], Node<'pr>)> {
    if let Some(call) = node.as_call_node() {
        let op = call.name().as_slice();
        if ![
            b"+".as_slice(),
            b"-",
            b"*",
            b"**",
            b"/",
            b"%",
            b"^",
            b"<<",
            b">>",
            b"|",
            b"&",
        ]
        .contains(&op)
            || call.is_safe_navigation()
            || call.block().is_some_and(|n| n.as_block_node().is_some())
            || read(&call.receiver()?)? != variable
        {
            return None;
        }
        let args = ast::call_arguments(&call);
        return (args.len() == 1).then(|| (op, args.into_iter().next().unwrap()));
    }
    let (left, right, op) = if let Some(n) = node.as_and_node() {
        (n.left(), n.right(), n.operator_loc())
    } else {
        let n = node.as_or_node()?;
        (n.left(), n.right(), n.operator_loc())
    };
    (matches!(op.as_slice(), b"&&" | b"||") && read(&left) == Some(variable))
        .then_some((op.as_slice(), right))
}
struct Parts<'a, 'pr> {
    variable: Variable<'pr>,
    value: Location<'pr>,
    equal: Location<'pr>,
    operator: &'pr [u8],
    operand: &'a Node<'pr>,
}
fn correction(
    node: &Node<'_>,
    Parts {
        variable,
        value,
        equal,
        operator,
        operand,
    }: Parts<'_, '_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    // ||= can suppress an undefined-class-variable exception; both boolean
    // forms can skip a nonlocal write (notably on frozen receivers/classes).
    if loc.end_offset() - loc.start_offset() > 16 * 1024
        || (variable.0 != 0 && matches!(operator, b"||" | b"&&"))
        || cx.ancestors().any(|n| n.as_defined_node().is_some())
        || matches!(
            operand,
            Node::SplatNode { .. }
                | Node::BlockArgumentNode { .. }
                | Node::ForwardingArgumentsNode { .. }
                | Node::KeywordHashNode { .. }
                | Node::ReturnNode { .. }
                | Node::BreakNode { .. }
                | Node::NextNode { .. }
                | Node::RedoNode { .. }
                | Node::RetryNode { .. }
        )
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, loc.start_offset()) {
        return None;
    }
    if variable.0 == 0
        && !file
            .visible_locals(root)
            .declared_before(variable.1, node, cx, loc.start_offset())
    {
        return None;
    }
    let text = std::str::from_utf8(operand.location().as_slice()).ok()?;
    // Preserve precedence, command binding and control flow of arbitrary RHS
    // expressions. Primary operands retain RuboCop's canonical spelling.
    let rhs = if primary(operand) {
        text.to_owned()
    } else {
        format!("({text})")
    };
    let edit = super::correction::combine(
        file,
        vec![
            Correction {
                range: equal.start_offset()..equal.start_offset(),
                replacement: std::str::from_utf8(operator).ok()?.into(),
            },
            Correction {
                range: value.start_offset()..value.end_offset(),
                replacement: rhs,
            },
        ],
    )?;
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + edit.range.len()));
    Some(edit)
}
fn primary(node: &Node<'_>) -> bool {
    ast::is_basic_literal(node)
        || read(node).is_some()
        || matches!(
            node,
            Node::ConstantReadNode { .. }
                | Node::ConstantPathNode { .. }
                | Node::ParenthesesNode { .. }
                | Node::StringNode { .. }
                | Node::ArrayNode { .. }
                | Node::HashNode { .. }
        )
        || node.as_call_node().is_some_and(|n| {
            n.block().is_none()
                && !ast::is_operator_method(n.name().as_slice())
                && (n.opening_loc().is_some() || ast::call_arguments(&n).is_empty())
        })
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/self_assignment_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn variable_kinds_captured_scopes_and_nonmatching_writes() {
        for source in [
            "$x = $x + 1",
            "X = X + 1",
            "Foo::X = Foo::X + 1",
            "obj.x = obj.x + 1",
            "obj[0] = obj[0] + 1",
            "x,y = x+1,y",
            "x = y + x",
            "x = (foo; x + y)",
            "x = (x and y)",
            "x = (x or y)",
            "x = x&.+(y)",
            "x = x.+(y,z)",
            "x = x.+(y) { work }",
            "x = (x) + 1",
            "x = x == y",
            "x += y",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == SelfAssignment.name()),
                "{source}"
            );
        }
        for (before, after) in [
            ("x = 1; proc { x = x + 2 }", "x = 1; proc { x += 2 }"),
            (
                "x = 1; proc { |x| x = x + 2 }",
                "x = 1; proc { |x| x += 2 }",
            ),
            (
                "x = 1; proc { |; x| x = x || 2 }",
                "x = 1; proc { |; x| x ||= 2 }",
            ),
            (
                "def f(x); x = x && false; end",
                "def f(x); x &&= false; end",
            ),
            ("@x = @x.+(work(2))", "@x += work(2)"),
            ("@@x = @@x ** y + z", "@@x = @@x ** y + z"),
        ] {
            if before == after {
                assert!(
                    !lint_str(before)
                        .iter()
                        .any(|o| o.cop == SelfAssignment.name())
                );
            } else {
                assert_correction(&SelfAssignment, before, after);
            }
        }
    }
    #[test]
    fn rhs_precedence_control_flow_and_dispatch_are_retained() {
        for (before, after) in [
            ("x = x.+(y * z)", "x += (y * z)"),
            ("x = x ** y + z", "x = x ** y + z"),
            ("x = x.+((false or y))", "x += (false or y)"),
            ("x = x + (y = 2)", "x += (y = 2)"),
            ("x = x.+(work y)", "x += (work y)"),
        ] {
            if before == after {
                assert!(
                    !lint_str(before)
                        .iter()
                        .any(|o| o.cop == SelfAssignment.name())
                );
            } else {
                assert_correction(&SelfAssignment, before, after);
            }
        }
    }
    #[test]
    fn corrections_are_lazy_bounded_and_scope_analysis_is_shared() {
        use crate::cops::variables::VISIBLE_LOCAL_BUILDS;
        let source = "x = x + 1\n".repeat(1000);
        REWRITE_BYTES.with(|n| n.set(0));
        VISIBLE_LOCAL_BUILDS.with(|n| n.set(0));
        assert_eq!(
            lint_str(&source)
                .iter()
                .filter(|o| o.cop == SelfAssignment.name())
                .count(),
            1000
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 0));
        assert!(
            correction_offenses(&SelfAssignment, &source)
                .iter()
                .all(|o| o.correction.is_some())
        );
        REWRITE_BYTES.with(|n| assert!(n.get() <= source.len()));
        VISIBLE_LOCAL_BUILDS.with(|n| assert_eq!(n.get(), 1));
        let huge = format!("x = x + '{}'", "z".repeat(20_000));
        REWRITE_BYTES.with(|n| n.set(0));
        assert!(
            correction_offenses(&SelfAssignment, &huge)[0]
                .correction
                .is_none()
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
    }
    #[test]
    fn every_pinned_call_has_exact_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/self_assignment_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 118);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == SelfAssignment.name())
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
    fn pinned_corrections_are_canonical_or_nonlocal_boolean_safety_holds() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/self_assignment_pinned.json"
        ))
        .unwrap();
        let (mut fixed, mut held) = (0, 0);
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            let before = c["source"].as_str().unwrap();
            if before.starts_with('@') && (before.contains("||") || before.contains("&&")) {
                let found = correction_offenses(&SelfAssignment, before);
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_none(), "{before}");
                held += 1;
            } else {
                assert_correction(&SelfAssignment, before, after);
                fixed += 1;
            }
        }
        assert_eq!((fixed, held), (76, 6));
    }
}
