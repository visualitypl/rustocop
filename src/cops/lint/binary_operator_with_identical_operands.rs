//! `Lint/BinaryOperatorWithIdenticalOperands` — flags a comparison, bitwise
//! or boolean operator whose two operands are identical (`x.top >= x.top`,
//! `a && a`).
//!
//! Matches RuboCop's semantics: operands are compared structurally; covered
//! operators are `== != === <=> =~ > >= < <= | ^` plus `&&`/`||`/`and`/`or`.
//! Arithmetic (`+ - * / ** << >>`) is allowed. Like RuboCop's (unsafe) cop,
//! it does not consider side effects of the repeated calls. `=~` on a
//! non-interpolated regexp literal (`/x/ =~ x`) is `match_with_lvasgn`, not a
//! `send`, in RuboCop's AST and is skipped.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast::same_node},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/BinaryOperatorWithIdenticalOperands` cop.
pub struct BinaryOperatorWithIdenticalOperands;

const OPERATORS: [&[u8]; 11] = [
    b"==", b"!=", b"===", b"<=>", b"=~", b">", b">=", b"<", b"<=", b"|", b"^",
];

impl BinaryOperatorWithIdenticalOperands {
    fn report(&self, node: &Node<'_>, operator: &[u8], file: &SourceFile, out: &mut Vec<Offense>) {
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Binary operator `{}` has identical operands.",
                String::from_utf8_lossy(operator)
            ),
            line,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            col,
        });
    }
}

impl Cop for BinaryOperatorWithIdenticalOperands {
    fn name(&self) -> &'static str {
        "Lint/BinaryOperatorWithIdenticalOperands"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some(call) = node.as_call_node() {
            let name = call.name().as_slice();
            // RuboCop handles `send` only: `a&.==(a)` is not checked.
            if !OPERATORS.contains(&name) || call.is_safe_navigation() {
                return;
            }
            let (Some(receiver), Some(arguments)) = (call.receiver(), call.arguments()) else {
                return;
            };
            // The parser gem builds `match_with_lvasgn`, not `send`, for `=~`
            // on a non-interpolated regexp literal, so RuboCop never checks
            // it. Prism's `RegularExpressionNode` is exactly that literal.
            if name == b"=~" && receiver.as_regular_expression_node().is_some() {
                return;
            }
            let Some(first) = arguments.arguments().iter().next() else {
                return;
            };
            if same_node(&receiver, &first) {
                self.report(node, name, file, out);
            }
        } else if let Some(and) = node.as_and_node() {
            if same_node(&and.left(), &and.right()) {
                self.report(node, and.operator_loc().as_slice(), file, out);
            }
        } else if let Some(or) = node.as_or_node()
            && same_node(&or.left(), &or.right())
        {
            self.report(node, or.operator_loc().as_slice(), file, out);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/binary_operator_with_identical_operands_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/BinaryOperatorWithIdenticalOperands")
            .collect()
    }

    #[test]
    fn flags_each_covered_operator_with_duplicate_operands() {
        for operator in [
            "==", "!=", "===", "<=>", "=~", "&&", "||", ">", ">=", "<", "<=", "|", "^",
        ] {
            let found = offenses(&format!("y = a.x(arg) {operator} a.x(arg)\n"));
            assert_eq!(found.len(), 1, "should flag {operator}");
            assert_eq!((found[0].line, found[0].col), (1, 5));
            assert_eq!(
                found[0].message,
                format!("Binary operator `{operator}` has identical operands.")
            );
        }
    }

    #[test]
    fn accepts_arithmetic_operators_with_duplicate_operands() {
        for operator in ["-", "+", "*", "/", "**", "<<", ">>"] {
            let source = format!("y = a.x(arg) {operator} a.x(arg)\n");
            assert!(offenses(&source).is_empty(), "should not flag {operator}");
        }
    }

    #[test]
    fn accepts_different_operands() {
        assert!(
            offenses("x == y\ny = x && z\ny = a.x + b.x\na.x(arg) > b.x(arg)\na.(x) > b.(x)\n")
                .is_empty()
        );
    }

    #[test]
    fn accepts_arithmetic_with_numerics() {
        assert!(offenses("x = 2 + 2\nx = 1 << 1\n").is_empty());
    }

    #[test]
    fn does_not_crash_on_operator_without_argument() {
        assert!(offenses("foo.*\n").is_empty());
        assert!(offenses("foo.==\n").is_empty());
    }

    #[test]
    fn flags_keyword_operators_and_layout_variants() {
        let found = offenses("if left_child or left_child\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 4));
        assert_eq!(
            found[0].message,
            "Binary operator `or` has identical operands."
        );
        let found = offenses("x = (a.x != 0 &&\n     a.x != 0)\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 6));
        assert_eq!(
            found[0].message,
            "Binary operator `&&` has identical operands."
        );
        assert_eq!(offenses("foo(1, 'a') == foo(1, \"a\")\n").len(), 1);
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "a.x == a.y\n",
            "a.x == b.x\n",
            "a && b\n",
            "x = 1 == 1.0\n",
            "a.foo(1) == a.foo(2)\n",
            "!a == a\n",
            "a & a\n",
            // RuboCop only checks `send`, not safe navigation (`csend`).
            "a&.==(a)\n",
            // `=~` with a non-interpolated regexp literal receiver is
            // `match_with_lvasgn` in the parser gem, not `send`, with or
            // without named captures.
            "/(?<a>x)/ =~ /(?<a>x)/\n",
            "/(?'a'x)/ =~ /(?'a'x)/\n",
            "/(?<A>x)/ =~ /(?<A>x)/\n",
            "/x/ =~ /x/\n",
            "/x/i =~ /x/i\n",
            "/(x)/ =~ /(x)/\n",
            "/(?<=x)/ =~ /(?<=x)/\n",
            "/(?<!x)/ =~ /(?<!x)/\n",
            "/[(?<a>x)]/ =~ /[(?<a>x)]/\n",
            "/\\(?<a>x\\)/ =~ /\\(?<a>x\\)/\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_match_operator_without_static_regexp_receiver() {
        // Interpolated or parenthesized regexps stay `send` in the parser gem.
        for source in [
            "/#{a}(?<a>x)/ =~ /#{a}(?<a>x)/\n",
            "(/x/) =~ (/x/)\n",
            "y =~ y\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "should flag {source:?}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
    }
}
