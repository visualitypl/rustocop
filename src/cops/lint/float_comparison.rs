//! `Lint/FloatComparison` — precise comparisons with syntactically known floats.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

const CASE_MESSAGE: &str =
    "Avoid float literal comparisons in case statements as they are unreliable.";

/// The `Lint/FloatComparison` cop (detection only).
pub struct FloatComparison;

impl Cop for FloatComparison {
    fn name(&self) -> &'static str {
        "Lint/FloatComparison"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some(case) = node.as_case_node() {
            for branch in &case.conditions() {
                let Some(when) = branch.as_when_node() else {
                    continue;
                };
                for condition in &when.conditions() {
                    if float(&condition) && !literal_safe(&condition) {
                        let (line, col) =
                            file.reference_line_col(condition.location().start_offset());
                        out.push(Offense {
                            cop: self.name(),
                            severity: self.severity(),
                            correction: None,
                            message: CASE_MESSAGE.into(),
                            line,
                            col,
                            end_line: file.line_of(ast::source_end(&condition).saturating_sub(1)),
                        });
                    }
                }
            }
            return;
        }
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !matches!(call.name().as_slice(), b"==" | b"!=" | b"eql?" | b"equal?") {
            return;
        }
        let arguments = ast::call_arguments(&call);
        let [rhs] = arguments.as_slice() else { return };
        let lhs = call.receiver();
        if lhs.as_ref().is_some_and(literal_safe) || literal_safe(rhs) {
            return;
        }
        if !lhs.as_ref().is_some_and(float) && !float(rhs) {
            return;
        }
        let kind = if call.name().as_slice() == b"!=" {
            "inequality"
        } else {
            "equality"
        };
        // Project only this report prefix through the existing pinned buffer:
        // its composition/header/BOM rules differ from general diagnostics.
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            message: format!("Avoid {kind} comparisons of floats as they are unreliable."),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
        });
    }
}

// RuboCop's `begin` here means grouping parentheses, not explicit begin/end
// (`kwbegin`), and examines only the first statement, not the eventual value.
fn first_grouped<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let body = node.as_parentheses_node()?.body()?;
    body.as_statements_node()?.body().iter().next()
}

fn literal_float_value(node: &Node<'_>) -> Option<f64> {
    if let Some(float) = node.as_float_node() {
        return Some(float.value());
    }
    ast::negated_numeric_literal(node)?
        .as_float_node()
        .map(|n| -n.value())
}

fn literal_safe(node: &Node<'_>) -> bool {
    let mut current = None;
    loop {
        let node = current.as_ref().unwrap_or(node);
        if let Some(first) = first_grouped(node) {
            current = Some(first);
            continue;
        }
        if let Some(number) = ast::negated_numeric_literal(node) {
            current = Some(number);
            continue;
        }
        return match node {
            Node::NilNode { .. } => true,
            Node::IntegerNode { .. } => node
                .as_integer_node()
                .unwrap()
                .value()
                .to_u32_digits()
                .1
                .iter()
                .all(|&d| d == 0),
            Node::FloatNode { .. } => node.as_float_node().unwrap().value() == 0.0,
            Node::RationalNode { .. } => node
                .as_rational_node()
                .unwrap()
                .numerator()
                .to_u32_digits()
                .1
                .iter()
                .all(|&d| d == 0),
            Node::ImaginaryNode { .. } => {
                current = Some(node.as_imaginary_node().unwrap().numeric());
                continue;
            }
            _ => false,
        };
    }
}

// Pure syntactic classification; never evaluate input or infer local types.
// Iterative traversal is confined to the expression being classified.
fn float(node: &Node<'_>) -> bool {
    let mut pending = Vec::new();
    if float_step(node, &mut pending) {
        return true;
    }
    while let Some(node) = pending.pop() {
        if float_step(&node, &mut pending) {
            return true;
        }
    }
    false
}

fn float_step<'pr>(node: &Node<'pr>, pending: &mut Vec<Node<'pr>>) -> bool {
    if literal_float_value(node).is_some() {
        return true;
    }
    if let Some(first) = first_grouped(node) {
        pending.push(first);
        return false;
    }
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.is_safe_navigation() || call.block().is_some_and(|b| b.as_block_node().is_some()) {
        return false;
    }
    let name = call.name().as_slice();
    let args = ast::call_arguments(&call);
    if matches!(name, b"+" | b"-" | b"*" | b"/" | b"%" | b"**") {
        pending.extend(call.receiver());
        pending.extend(args.into_iter().take(1));
    } else if matches!(name, b"to_f" | b"Float" | b"fdiv") {
        return true;
    } else if let Some(value) = call.receiver().as_ref().and_then(literal_float_value) {
        if matches!(
            name,
            // `@-` (not `-@`) is the exact pinned list; don't broaden it.
            b"@-" | b"abs" | b"magnitude" | b"modulo" | b"next_float" | b"prev_float" | b"quo"
        ) {
            return true;
        }
        if matches!(name, b"angle" | b"arg" | b"phase") && value < 0.0 {
            return true;
        }
        if matches!(name, b"ceil" | b"floor" | b"round" | b"truncate")
            && args.first().is_some_and(|arg| {
                arg.as_integer_node().is_some_and(|int| {
                    let value = int.value();
                    let (negative, digits) = value.to_u32_digits();
                    !negative && digits.iter().any(|&d| d != 0)
                })
            })
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/lint/float_comparison_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/FloatComparison")
            .collect()
    }

    #[test]
    fn case_conditions_report_each_float_but_not_safe_or_nonfloat_values() {
        let found =
            offenses("case value\nwhen 1.0, 2.0\n  foo\nwhen 0.0, 1, 'string'\n  bar\nend\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 6), (2, 11)]
        );
        for o in found {
            assert_eq!(
                o.message,
                "Avoid float literal comparisons in case statements as they are unreliable."
            );
        }
        assert!(offenses("case\nwhen (value - 1.0).abs < Float::EPSILON\nfoo\nend").is_empty());
    }

    #[test]
    fn matches_float_returning_methods_arithmetic_and_precision() {
        for rhs in [
            "Float(1)",
            "'0.1'.to_f",
            "1.fdiv(2)",
            "0.1 + y",
            "y + Float('0.1')",
            "y + z * (foo(arg) + '0.1'.to_f)",
            "0.1.abs",
            "1.1.ceil(1)",
        ] {
            assert_eq!(offenses(&format!("x == {rhs}")).len(), 1, "{rhs}");
        }
    }

    #[test]
    fn safe_literals_and_nonfloat_lookalikes_are_ignored() {
        for source in [
            "x == 0.0",
            "x.to_f == 0",
            "x.to_f.abs == 0.0",
            "x != 0.0",
            "x.to_f != 0",
            "x.to_f.zero?",
            "x.to_f.nonzero?",
            "Float('not_a_float', exception: false) == nil",
            "nil != Float('not_a_float', exception: false)",
            "x == (0.0)",
            "x == ((0))",
            "Float('not_a_float', exception: false) == (nil)",
            "x == 1.1.ceil",
            "(x - 0.1) < epsilon",
            "value == 0.2r",
            "x.==(0.1, 0.2)",
            "x.!=(0.1, 0.2)",
            "x.eql?(0.1, 0.2)",
            "x.equal?(0.1, 0.2)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn grouping_uses_first_child_not_tail_or_explicit_begin() {
        assert_eq!(offenses("x == (0.1; 0)").len(), 1);
        for source in [
            "x == (0; 0.1)",
            "x == begin; 0.1; end",
            "x.to_f == (nil; 1)",
            "x.to_f == -0.0r",
            "x.to_f == -0.0i",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn no_type_flow_or_safe_navigation_return_inference() {
        for source in [
            "value = 0.1; x == value",
            "x == obj&.to_f",
            "x == (1.0).abs",
            "x == 1.0.abs {}",
            "x == 1.0.-@",
            "x == 1.0.ceil(n)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("x&.eql?(1.0.abs)").len(), 1);
    }

    #[test]
    fn upstream_source_conversion_errors_do_not_become_runtime_evaluation_or_internal_errors() {
        // The pin calls Integer('--1') / Integer('-+1') and raises. These
        // non-single-spelling precisions are not spec parity exclusions.
        for source in ["x == 1.1.ceil(--1)", "x == 1.1.ceil(-+1)"] {
            assert!(offenses(source).is_empty());
        }
    }

    #[test]
    fn case_columns_project_only_the_report_prefix_not_grouped_child_ownership() {
        for (header, col) in [
            ("# coding: UTF8-MAC\n", 18),
            ("# coding: UTF8-MAC\r\n", 18),
            ("# coding: UTF-8\n", 19),
            (" # coding: UTF8-MAC\n", 19),
        ] {
            let source = format!("{header}case x\nwhen ('e\u{301}'; 1.0), 0.1\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (3, col, 3)
            );
            assert_eq!(found[0].message, super::CASE_MESSAGE);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn call_columns_use_composed_buffer_and_retain_leading_bom() {
        for (source, position) in [
            ("# coding: UTF8-MAC\n'e\u{301}'; x == 0.1\n", (2, 6)),
            ("\u{feff}'é'; x == 0.1\r\n", (1, 7)),
            ("# coding: UTF-8\n'e\u{301}'; x == 0.1\n", (2, 7)),
            (" # coding: UTF8-MAC\n'e\u{301}'; x != 0.1\n", (2, 7)),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), position, "{source:?}");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn equality_and_inequality_report_exact_positions_and_messages() {
        for (expression, kind) in [
            ("x == 0.1", "equality"),
            ("0.1 == x", "equality"),
            ("x != 0.1", "inequality"),
            ("0.1 != x", "inequality"),
            ("x.eql?(0.1)", "equality"),
            ("0.1.eql?(x)", "equality"),
            ("x&.eql?(0.1)", "equality"),
            ("x&.equal?(0.1)", "equality"),
        ] {
            let found = offenses(&format!("# comparison\n  {expression}\n"));
            assert_eq!(found.len(), 1, "{expression}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("Avoid {kind} comparisons of floats as they are unreliable.")
            );
            assert!(found[0].correction.is_none());
        }
    }
}
