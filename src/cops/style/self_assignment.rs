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
            .then(|| correction(&value_loc, &equal, operator, &operand, file))
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
/// RuboCop's corrector: the operator before `=`, the operand's source after.
fn correction(
    value: &Location<'_>,
    equal: &Location<'_>,
    operator: &[u8],
    operand: &Node<'_>,
    file: &SourceFile,
) -> Option<Correction> {
    let rhs = std::str::from_utf8(operand.location().as_slice())
        .ok()?
        .to_owned();
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
    Some(edit)
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/self_assignment_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
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
}
