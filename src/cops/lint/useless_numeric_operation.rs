//! `Lint/UselessNumericOperation` — numeric identity syntax on variables.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct UselessNumericOperation;
impl Cop for UselessNumericOperation {
    fn name(&self) -> &'static str {
        "Lint/UselessNumericOperation"
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
        let matches = if let Some(call) = node.as_call_node() {
            let Some(receiver) = call.receiver() else {
                return;
            };
            if !variable(&receiver) {
                return;
            }
            let args = ast::call_arguments(&call);
            let [rhs] = args.as_slice() else { return };
            identity(call.name().as_slice(), rhs, file)
        } else {
            // Prism fuses the target read, binary send and write. Only actual
            // variable/constant targets match op-asgn, never attribute/index writes.
            macro_rules! write {
                ($access:ident) => {{
                    let n = node.$access().unwrap();
                    identity(n.binary_operator().as_slice(), &n.value(), file)
                }};
            }
            match node {
                Node::LocalVariableOperatorWriteNode { .. } => {
                    write!(as_local_variable_operator_write_node)
                }
                Node::InstanceVariableOperatorWriteNode { .. } => {
                    write!(as_instance_variable_operator_write_node)
                }
                Node::ClassVariableOperatorWriteNode { .. } => {
                    write!(as_class_variable_operator_write_node)
                }
                Node::GlobalVariableOperatorWriteNode { .. } => {
                    write!(as_global_variable_operator_write_node)
                }
                Node::ConstantOperatorWriteNode { .. } => write!(as_constant_operator_write_node),
                Node::ConstantPathOperatorWriteNode { .. } => {
                    write!(as_constant_path_operator_write_node)
                }
                _ => return,
            }
        };
        if !matches {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: "Do not apply inconsequential numeric operations to variables.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, start..end))
                .flatten(),
        });
    }
}
fn variable(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableReadNode { .. }
            | Node::ItLocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::ClassVariableReadNode { .. }
            | Node::GlobalVariableReadNode { .. }
            | Node::ConstantReadNode { .. }
            | Node::ConstantPathNode { .. }
    ) || node
        .as_call_node()
        .is_some_and(|c| c.receiver().is_none() && c.arguments().is_none() && c.block().is_none())
}
fn identity(op: &[u8], node: &Node<'_>, file: &SourceFile) -> bool {
    let negative = ast::negated_numeric_literal(node);
    let node = negative.as_ref().unwrap_or(node);
    let value: Option<i32> = if node.as_source_line_node().is_some() {
        file.line_of(node.location().start_offset()).try_into().ok()
    } else {
        node.as_integer_node()
            .and_then(|n| n.value().try_into().ok())
    };
    // The adapter folds one prefix minus on a literal, including an already
    // negative IntegerNode. Do not fold arbitrary unary-call chains.
    let value = if negative.is_some() {
        value.and_then(i32::checked_neg)
    } else {
        value
    };
    matches!(
        (op, value),
        (b"+" | b"-", Some(0)) | (b"*" | b"/" | b"**", Some(1))
    )
}
/// RuboCop replaces an operation with its receiver, and an abbreviated
/// assignment with a plain one (`x = x`).
fn correction(
    node: &Node<'_>,
    range: std::ops::Range<usize>,
) -> Option<crate::offense::Correction> {
    let source = |bytes: &[u8]| std::str::from_utf8(bytes).ok().map(str::to_owned);
    let replacement = if let Some(call) = node.as_call_node() {
        source(call.receiver()?.location().as_slice())?
    } else {
        let target = match node {
            Node::LocalVariableOperatorWriteNode { .. } => {
                node.as_local_variable_operator_write_node()?.name_loc()
            }
            Node::InstanceVariableOperatorWriteNode { .. } => {
                node.as_instance_variable_operator_write_node()?.name_loc()
            }
            Node::ClassVariableOperatorWriteNode { .. } => {
                node.as_class_variable_operator_write_node()?.name_loc()
            }
            Node::GlobalVariableOperatorWriteNode { .. } => {
                node.as_global_variable_operator_write_node()?.name_loc()
            }
            Node::ConstantOperatorWriteNode { .. } => {
                node.as_constant_operator_write_node()?.name_loc()
            }
            _ => node
                .as_constant_path_operator_write_node()?
                .target()
                .location(),
        };
        let target = source(target.as_slice())?;
        format!("{target} = {target}")
    };
    Some(crate::offense::Correction { range, replacement })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_numeric_operation_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessNumericOperation")
            .collect()
    }
    #[test]
    fn pinned_identity_operations_calls_and_all_variable_targets() {
        for (op, number) in [("+", 0), ("-", 0), ("*", 1), ("/", 1), ("**", 1)] {
            for receiver in ["x", "@x", "@@x", "$x", "CONST", "Other::CONST", "::CONST"] {
                for expression in [
                    format!("{receiver} {op} {number}"),
                    format!("{receiver} {op}= {number}"),
                    format!("{receiver}.{op}({number})"),
                    format!("{receiver}&.{op}({number})"),
                ] {
                    let source = format!("# example\n  {expression}\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col, found[0].end_line),
                        (2, 3, 2),
                        "{source}"
                    );
                    assert_eq!(
                        found[0].message,
                        "Do not apply inconsequential numeric operations to variables."
                    );
                }
            }
        }
        for source in ["x = 1\nx + 0", "x.+(0).bar", "x().+(0)"] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn lookalikes_arity_grouping_blocks_and_literal_values() {
        for source in [
            "x + 1",
            "x - 1",
            "x * 0",
            "x / 0",
            "x ** 0",
            "x + 0.0",
            "x + 0r",
            "x * 1i",
            "0 + x",
            "1 * 1",
            "self + 0",
            "self.x + 0",
            "x(1) + 0",
            "x(&b) + 0",
            "x {} + 0",
            "x.+(0, 1)",
            "x.+(0, &b)",
            "(x) + 0",
            "x + (0)",
            "x[0] += 0",
            "x.value += 0",
            "x&.value += 0",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "x + -0",
            "x - 0x0",
            "x * 0b1",
            "x / 01",
            "x ** +1",
            "x.+(0) { work }",
            "x * __LINE__",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn implicit_it_receivers_ordinary_and_safe_navigation() {
        for (op, number) in [("+", 0), ("-", 0), ("*", 1), ("/", 1), ("**", 1)] {
            for expression in [
                format!("it {op} {number}"),
                format!("it.{op}({number})"),
                format!("it&.{op}({number})"),
            ] {
                let source = format!("items.map {{ {expression} }}");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 13), "{source}");
                assert_eq!(
                    found[0].message,
                    "Do not apply inconsequential numeric operations to variables."
                );
                assert!(found[0].correction.is_none());
            }
        }
        for source in ["items.map { it + 1 }", "items.map { it * 0 }"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn negated_negative_one_literal_values_without_arbitrary_folding() {
        for op in ["*", "/", "**"] {
            for number in ["--1", "- -1", "- -0x1"] {
                for expression in [
                    format!("x {op} {number}"),
                    format!("x {op}= {number}"),
                    format!("x.{op}({number})"),
                    format!("x&.{op}({number})"),
                ] {
                    let source = format!("# example\n  {expression}\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (2, 3), "{source}");
                    assert_eq!(
                        found[0].message,
                        "Do not apply inconsequential numeric operations to variables."
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
        for source in [
            "x * -1",
            "x * ---1",
            "x * -(-1)",
            "x * -value",
            "x * (-1).-@",
            "x * - -1.0",
            "x *= - -1r",
            "x / - -1i",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn canonical_inactive_edit_and_ordinary_holds() {
        crate::cops::testing::assert_correction(
            &super::UselessNumericOperation,
            "if false\n  x + 0\nend\n",
            "if false\n  x\nend\n",
        );
        assert!(offenses("x + 0")[0].correction.is_none());
    }
}
