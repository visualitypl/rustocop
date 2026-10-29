//! `Style/NumericPredicate` — prefer numeric predicates (unsafe, detection only).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;

/// Default configured BitwisePredicate is enabled even under `--only`.
pub struct NumericPredicate;
struct ConfiguredNumericPredicate {
    bitwise_enabled: bool,
}
impl Cop for NumericPredicate {
    fn name(&self) -> &'static str {
        "Style/NumericPredicate"
    }
    fn with_bitwise_predicate(&self, enabled: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(ConfiguredNumericPredicate {
            bitwise_enabled: enabled,
        }))
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        check(node, file, out, true);
    }
}
impl Cop for ConfiguredNumericPredicate {
    fn name(&self) -> &'static str {
        "Style/NumericPredicate"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        check(node, file, out, self.bitwise_enabled);
    }
}

fn check(node: &Node<'_>, file: &SourceFile, out: &mut Vec<Offense>, bitwise_enabled: bool) {
    let Some(call) = node.as_call_node() else {
        return;
    };
    if call.is_safe_navigation() {
        return;
    }
    let op = call.name();
    let op = op.as_slice();
    if !matches!(op, b"==" | b">" | b"<") {
        return;
    }
    let Some(receiver) = call.receiver() else {
        return;
    };
    let args = ast::call_arguments(&call);
    let [argument] = args.as_slice() else { return };
    let zero = |n: &Node<'_>| super::even_odd::integer(n, Some(file)) == Some(0);
    let (numeric, method) = if zero(argument) && receiver.as_global_variable_read_node().is_none() {
        (
            &receiver,
            match op {
                b">" => "positive?",
                b"<" => "negative?",
                _ => "zero?",
            },
        )
    } else if zero(&receiver) && argument.as_global_variable_read_node().is_none() {
        (
            argument,
            match op {
                b">" => "negative?",
                b"<" => "positive?",
                _ => "zero?",
            },
        )
    } else {
        return;
    };
    // Upstream delegates only forward ==/> comparisons on one grouped or
    // ungrouped bitwise send. Inversions and < remain this cop's responsibility.
    // The fixed Ruby 3.4 target supports BitwisePredicate (introduced in 2.5).
    if bitwise_enabled
        && matches!(op, b"==" | b">")
        && zero(argument)
        && super::even_odd::single_group(call.receiver().unwrap())
            .and_then(|n| n.as_call_node())
            .is_some_and(|n| {
                n.name().as_slice() == b"&"
                    && !n.is_safe_navigation()
                    && n.receiver().is_some()
                    && n.block().is_none()
                    && ast::call_arguments(&n).len() == 1
            })
    {
        return;
    }
    let preferred = single_line(numeric.location().as_slice());
    let parentheses = numeric.as_call_node().is_some_and(|n| {
        !n.is_safe_navigation()
            && !n
                .block()
                .is_some_and(|block| block.as_block_node().is_some())
            && ast::is_operator_method(n.name().as_slice())
            && !n.closing_loc().is_some_and(|loc| loc.as_slice() == b")")
            && ast::negated_numeric_literal(numeric).is_none()
    });
    let preferred = if parentheses {
        format!("({preferred})")
    } else {
        preferred
    };
    let start = node.location().start_offset();
    let end = ast::source_end(node);
    let current = single_line(&file.bytes[start..end]);
    let (line, col) = file.line_col(start);
    out.push(Offense {
        cop: "Style/NumericPredicate",
        severity: crate::offense::Severity::Convention,
        line,
        col,
        end_line: file.line_of(end.saturating_sub(1)),
        message: format!("Use `{preferred}.{method}` instead of `{current}`."),
        correction: None,
    });
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop style/numeric_predicate_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/NumericPredicate")
            .collect()
    }
    #[test]
    fn pinned_comparisons_in_both_directions() {
        for (op, inverse, method) in [
            ("==", "==", "zero"),
            (">", "<", "positive"),
            ("<", ">", "negative"),
        ] {
            for (operand, preferred) in [
                ("number", "number"),
                ("foo - 1", "(foo - 1)"),
                ("-foo", "(-foo)"),
                ("~foo", "(~foo)"),
            ] {
                for expression in [
                    format!("{operand} {op} 0"),
                    format!("0 {inverse} {operand}"),
                ] {
                    let found = offenses(&format!("# header\n  {expression}\n"));
                    assert_eq!(found.len(), 1, "{expression}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert_eq!(
                        found[0].message,
                        format!("Use `{preferred}.{method}?` instead of `{expression}`.")
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn literal_zero_values_and_unary_folding() {
        for zero in [
            "0", "00", "0x0", "0b0", "0o0", "0d0", "0_0", "+0", "-0", "- 0", "-\n0", "- -0", "- +0",
        ] {
            for s in [format!("number == {zero}"), format!("{zero} < number")] {
                assert_eq!(offenses(&s).len(), 1, "{s}");
            }
        }
        for value in [
            "1",
            "-1",
            "0x10",
            "999999999999999999999999999",
            "0.0",
            "-0.0",
            "0r",
            "0i",
            "0.0r",
            "0ri",
            "(0)",
            "+ 0",
            "- - 0",
            "0.-@",
            "__LINE__",
        ] {
            for s in [format!("number == {value}"), format!("{value} < number")] {
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
    }
    #[test]
    fn no_type_assumptions_or_nonzero_predicate() {
        for receiver in [
            "object", "@value", "@@value", "NUMBER", "self", "nil", "false", "1.0", "1r", "1i",
            "-1", "value[0]", "foo&.bar",
        ] {
            assert_eq!(offenses(&format!("{receiver} == 0")).len(), 1, "{receiver}");
        }
        for s in [
            "$CHILD_STATUS == 0",
            "0 == $CHILD_STATUS",
            "$x < 0",
            "0 > $x",
            "number != 0",
            "0 != number",
            "foo - 1 != 0",
            "number >= 0",
            "number <= 0",
            "number.zero?",
            "number.positive?",
            "number.negative?",
            "number.nonzero?",
            "foo&.==(0)",
            "foo.==(0, 1)",
            "foo == [0]",
            "foo === 0",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn scopes_and_default_empty_allowlists() {
        for (s, line, col) in [
            ("def m(foo)\n  foo == 0\nend", 2, 3),
            ("def m(foo)\n  foo - 1 == 0\nend", 2, 3),
            ("exclude(number > 0)", 1, 9),
            ("exclude { number < 0 }", 1, 11),
            ("where { _1 > 0 }", 1, 9),
            ("where { it < 0 }", 1, 9),
            ("order(Sequel[:number] > 0)", 1, 7),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{s}");
        }
    }
    #[test]
    fn source_aware_operator_receiver_suggestions() {
        for (s, preferred) in [
            ("(a + b) == 0", "(a + b).zero?"),
            ("a.+(b) == 0", "a.+(b).zero?"),
            ("a[0] == 0", "(a[0]).zero?"),
            ("-1 == 0", "-1.zero?"),
            ("__LINE__ > 0", "__LINE__.positive?"),
            ("-__LINE__ == 0", "(-__LINE__).zero?"),
            ("- __LINE__ == 0", "(- __LINE__).zero?"),
            ("foo.! == 0", "(foo.!).zero?"),
            ("(foo; bar) > 0", "(foo; bar).positive?"),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(
                found[0].message,
                format!("Use `{preferred}` instead of `{s}`.")
            );
        }
    }
    #[test]
    fn literal_blocks_and_multiline_sends_keep_source_boundaries() {
        for (s, preferred, current, end_line) in [
            ("foo.==(0) { work }", "foo.zero?", "foo.==(0)", 1),
            ("foo.==(0, &block)", "", "", 1),
            (
                "a.&(b) { c } == 0",
                "a.&(b) { c }.zero?",
                "a.&(b) { c } == 0",
                1,
            ),
            (
                "foo - #keep\n1 == 0",
                "(foo - #keep 1).zero?",
                "foo - #keep 1 == 0",
                2,
            ),
        ] {
            let found = offenses(s);
            if preferred.is_empty() {
                assert!(found.is_empty(), "{s}");
            } else {
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, 1, end_line)
                );
                assert_eq!(
                    found[0].message,
                    format!("Use `{preferred}` instead of `{current}`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn literal_block_is_not_a_bitwise_send_receiver() {
        // A literal block wraps the send in RuboCop's AST; delegation must
        // not silently discard this NumericPredicate finding.
        for op in ["==", ">"] {
            let expression = format!("(a.&(b) {{ c }}) {op} 0");
            let found = offenses(&expression);
            assert_eq!(found.len(), 1, "{expression}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "Use `(a.&(b) {{ c }}).{}?` instead of `{expression}`.",
                    if op == "==" { "zero" } else { "positive" }
                )
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn bitwise_delegation_is_directional_and_one_group_only() {
        for s in ["x & flags == 0", "(x & flags) > 0", "(x & flags).zero?"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "0 == x & flags",
            "0 < (x & flags)",
            "(x & flags) < 0",
            "((x & flags)) == 0",
            "(x | flags) == 0",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
}
