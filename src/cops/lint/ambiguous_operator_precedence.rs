//! `Lint/AmbiguousOperatorPrecedence` — make mixed operator precedence explicit.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AmbiguousOperatorPrecedence;
impl Cop for AmbiguousOperatorPrecedence {
    fn name(&self) -> &'static str {
        "Lint/AmbiguousOperatorPrecedence"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        // Explicit operator arguments, e.g. a.+(b * c), need grouping
        // according to this cop, but RedundantParentheses removes that same
        // grouping. Use the existing selected-peer guard to prevent cycles,
        // retaining both diagnostics and standalone canonical corrections.
        &["Style/RedundantParentheses"]
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // ArgumentsNode is transparent in RuboCop; ParenthesesNode and
        // StatementsNode are not. Never skip a grouping to find an operator.
        let Some(parent) = cx.ancestors().find(|n| n.as_arguments_node().is_none()) else {
            return;
        };
        let ambiguous = if node.as_and_node().is_some() {
            // Upstream on_and checks node kinds, including English and/or.
            parent.as_or_node().is_some()
        } else if let Some(call) = node.as_call_node() {
            call.opening_loc().is_none()
                && call.call_operator_loc().is_none()
                && !call.block().is_some_and(|b| b.as_block_node().is_some())
                && precedence(node)
                    .zip(precedence(parent))
                    .is_some_and(|(child, parent)| child < parent)
        } else {
            false
        };
        if !ambiguous {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            // An atomic wrap keeps every original interior byte/newline and
            // adds no evaluation, lookup or allocation. The shared main-parse
            // guards subsequently withhold source/DATA/code/heap/heredoc risks.
            // Non-UTF-8 spans remain diagnosed without a lossy replacement.
            correction: cx
                .corrections_requested()
                .then(|| {
                    std::str::from_utf8(loc.as_slice())
                        .ok()
                        .map(|source| Correction {
                            range: loc.start_offset()..loc.end_offset(),
                            replacement: format!("({source})"),
                        })
                })
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message:
                "Wrap expressions with varying precedence with parentheses to avoid ambiguity."
                    .into(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
        });
    }
}

fn precedence(node: &Node<'_>) -> Option<u8> {
    if let Some(call) = node.as_call_node() {
        if call.is_safe_navigation() {
            return None;
        }
        return match call.name().as_slice() {
            b"**" => Some(0),
            b"*" | b"/" | b"%" => Some(1),
            b"+" | b"-" => Some(2),
            b"<<" | b">>" => Some(3),
            b"&" => Some(4),
            b"|" | b"^" => Some(5),
            _ => None,
        };
    }
    if node
        .as_and_node()
        .is_some_and(|n| n.operator_loc().as_slice() == b"&&")
    {
        return Some(6);
    }
    if node
        .as_or_node()
        .is_some_and(|n| n.operator_loc().as_slice() == b"||")
    {
        return Some(7);
    }
    None
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's ambiguous_operator_precedence_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/AmbiguousOperatorPrecedence")
            .collect()
    }
    #[test]
    fn pinned_offense_positions_and_messages() {
        for (source, positions) in [
            ("a + b * c", vec![5]),
            ("a && b || c\na || b && c", vec![1, 6]),
            ("a && b * c\na * b && c", vec![6, 1]),
            ("(a + b * c ** d)", vec![6, 10]),
            (
                "a ** b * c / d % e + f - g << h >> i & j | k ^ l && m || n",
                vec![1; 7],
            ),
            (
                "a || b && c | d ^ e & f << g >> h + i - j * k / l % n ** m",
                vec![6, 11, 19, 23, 33, 41, 53],
            ),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
            for o in &found {
                assert_eq!(
                    o.message,
                    "Wrap expressions with varying precedence with parentheses to avoid ambiguity."
                );
            }
            if source.contains('\n') {
                assert_eq!(found.iter().map(|o| o.line).collect::<Vec<_>>(), [1, 2]);
            }
        }
    }
    #[test]
    fn canonical_wraps_preserve_interior_bytes() {
        for (before, after) in [
            ("a + b * c\n", "a + (b * c)\n"),
            (
                "a && b || c\na || b && c\n",
                "(a && b) || c\na || (b && c)\n",
            ),
            ("a && b * c\na * b && c\n", "a && (b * c)\n(a * b) && c\n"),
            ("a ** b + c", "(a ** b) + c"),
            ("a.+(b * c)", "a.+((b * c))"),
            ("a and b or c", "(a and b) or c"),
            (
                "a + b * # keep\n c # tail\n",
                "a + (b * # keep\n c) # tail\n",
            ),
            (
                "\u{feff}# encoding: utf-8\r\na + b * c\r\np __LINE__\r\n__END__\r\ndata\x00\r\n",
                "\u{feff}# encoding: utf-8\r\na + (b * c)\r\np __LINE__\r\n__END__\r\ndata\x00\r\n",
            ),
        ] {
            crate::cops::testing::assert_correction(
                &super::AmbiguousOperatorPrecedence,
                before,
                after,
            );
        }
    }
    #[test]
    fn pinned_no_offense_examples() {
        for source in [
            "a + b",
            "a + b + c",
            "a * b / c % d",
            "a && b && c",
            "1 + CONST.*",
            "CONST.* + 1",
            "a + (b * c)",
            "(a ** b) + c",
            "(a + b) * c",
            "a ** (b + c)",
            "(a && b) || c",
            "a || (b & c)",
            "array << i and return",
            "array << i or return",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn transparent_arguments_but_not_explicit_grouping_or_other_nodes() {
        for (source, count) in [
            ("a.+(b * c)", 1),
            ("a + b.*(c)", 0),
            ("a.+((b * c))", 0),
            ("a || b & c", 1),
            ("a and b or c", 1),
            ("a && b or c", 1),
            ("a || b and c", 0),
            ("a < b + c", 0),
            ("a + -b", 0),
            ("a + b ** c", 1),
            ("a + [b * c]", 0),
            ("a + foo(b * c)", 0),
            ("a.+(b * c, d)", 1),
            ("a&.+(b * c)", 0),
            ("a.+(b&.*(c))", 0),
            ("begin; a && b; end || c", 0),
        ] {
            assert_eq!(offenses(source).len(), count, "{source}");
        }
    }
}
