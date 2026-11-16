//! `Lint/ImplicitStringConcatenation` — adjacent literals on the same line.
use super::adjacent_strings;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ImplicitStringConcatenation;
impl Cop for ImplicitStringConcatenation {
    fn name(&self) -> &'static str {
        "Lint/ImplicitStringConcatenation"
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
        let Some(parts) = adjacent_strings::children(node) else {
            return;
        };
        // ArgumentsNode is transparent in RuboCop; receiver and setter calls
        // are send parents as well. Literal blocks instead own their bodies.
        let parent = cx.ancestors().find(|p| p.as_arguments_node().is_none());
        let suffix = if parent.is_some_and(|p| p.as_array_node().is_some()) {
            " Or, if they were intended to be separate array elements, separate them with a comma."
        } else if parent.is_some_and(|p| send_parent(p, node)) {
            " Or, if they were intended to be separate method arguments, separate them with a comma."
        } else {
            ""
        };
        for pair in parts.windows(2) {
            let lhs = &pair[0];
            let rhs = &pair[1];
            let raw = lhs.location().as_slice();
            // Opening-less word-array strings also contain literal fragments
            // and embedded statements. Quotes in a fragment are payload, not
            // delimiters; pinned each_bad_cons requires two str/dstr children.
            if !matches!(
                lhs,
                Node::StringNode { .. } | Node::InterpolatedStringNode { .. }
            ) || !matches!(
                rhs,
                Node::StringNode { .. } | Node::InterpolatedStringNode { .. }
            ) || !matches!(raw.first(), Some(b'\'' | b'"'))
                || raw.first() != raw.last()
                || file.line_of(lhs.location().end_offset().saturating_sub(1))
                    != file.line_of(rhs.location().start_offset())
            {
                continue;
            }
            let (line, col) = super::array_literal_in_regexp::reference_line_col(
                lhs.location().start_offset(),
                file,
            );
            // RuboCop removes an empty string, or puts ` + ` between the two.
            let range = |n: &Node<'_>| n.location().start_offset()..n.location().end_offset();
            let (removed, replacement) = if adjacent_strings::empty(lhs) {
                (range(lhs), "")
            } else if adjacent_strings::empty(rhs) {
                (range(rhs), "")
            } else {
                (
                    lhs.location().end_offset()..rhs.location().start_offset(),
                    " + ",
                )
            };
            out.push(Offense {
                cop: self.name(), severity: self.severity(), line, col,
                end_line: file.last_line_of(rhs.location()),
                message: format!("Combine {} and {} into a single string literal, rather than using implicit string concatenation.{suffix}", adjacent_strings::display(lhs, file), adjacent_strings::display(rhs, file)),
                correction: cx.corrections_requested().then(|| crate::offense::Correction { range: removed, replacement: replacement.into() }),
            });
        }
    }
}

// Compound and multiple-assignment receivers/indices belong to a synthetic
// send in the pinned AST. Their RHS belongs to the assignment, not that send.
fn send_parent(parent: &Node<'_>, child: &Node<'_>) -> bool {
    let (value, safe) = match parent {
        Node::CallNode { .. } => return !parent.as_call_node().unwrap().is_safe_navigation(),
        Node::CallTargetNode { .. } => {
            return !parent.as_call_target_node().unwrap().is_safe_navigation();
        }
        Node::IndexTargetNode { .. } => return true,
        Node::CallAndWriteNode { .. } => {
            let n = parent.as_call_and_write_node().unwrap();
            (n.value(), n.is_safe_navigation())
        }
        Node::CallOrWriteNode { .. } => {
            let n = parent.as_call_or_write_node().unwrap();
            (n.value(), n.is_safe_navigation())
        }
        Node::CallOperatorWriteNode { .. } => {
            let n = parent.as_call_operator_write_node().unwrap();
            (n.value(), n.is_safe_navigation())
        }
        Node::IndexAndWriteNode { .. } => {
            (parent.as_index_and_write_node().unwrap().value(), false)
        }
        Node::IndexOrWriteNode { .. } => (parent.as_index_or_write_node().unwrap().value(), false),
        Node::IndexOperatorWriteNode { .. } => (
            parent.as_index_operator_write_node().unwrap().value(),
            false,
        ),
        _ => return false,
    };
    !safe && child.location().end_offset() <= value.location().start_offset()
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/implicit_string_concatenation_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ImplicitStringConcatenation")
            .collect()
    }
    #[test]
    fn pinned_same_line_pairs_messages_and_contexts() {
        for (source, line, col, lhs, rhs, suffix) in [
            (
                "class A; \"abc\" \"def\"; end",
                1,
                10,
                "\"abc\"",
                "\"def\"",
                "",
            ),
            ("class B; 'ghi' 'jkl'; end", 1, 10, "'ghi'", "'jkl'", ""),
            (
                "\"string#{interpolation}\" \"def\"",
                1,
                1,
                "\"string#{interpolation}\"",
                "\"def\"",
                "",
            ),
            (
                "'single-quoted string'\"string\n#{interpolation}\"",
                1,
                1,
                "'single-quoted string'",
                "\"string\\n\"",
                "",
            ),
            (
                "def method\n  \"ab\nc\" \"de\nf\"\nend",
                2,
                3,
                "\"ab\\nc\"",
                "\"de\\nf\"",
                "",
            ),
            (
                "array = [\"abc\" \"def\"]",
                1,
                10,
                "\"abc\"",
                "\"def\"",
                " Or, if they were intended to be separate array elements, separate them with a comma.",
            ),
            (
                "method(\"abc\" \"def\")",
                1,
                8,
                "\"abc\"",
                "\"def\"",
                " Or, if they were intended to be separate method arguments, separate them with a comma.",
            ),
        ] {
            let got = offenses(source);
            assert_eq!(got.len(), 1, "{source:?}");
            assert_eq!((got[0].line, got[0].col), (line, col));
            assert_eq!(
                got[0].message,
                format!(
                    "Combine {lhs} and {rhs} into a single string literal, rather than using implicit string concatenation.{suffix}"
                )
            );
        }
    }
    #[test]
    fn every_pair_and_triple_quotes() {
        for (source, cols) in [
            ("\"foo\"\"string#{interpolation}\"\"bar\"", vec![1, 6]),
            ("\"\"\"string\"\"\"", vec![1, 3]),
        ] {
            assert_eq!(
                offenses(source).iter().map(|o| o.col).collect::<Vec<_>>(),
                cols
            );
        }
    }
    #[test]
    fn word_array_quoted_fragments_are_not_literal_pairs() {
        for source in [
            r##"%W["a"#{value}]"##,
            r##"%W['a'#{value}]"##,
            r##"%W[""#{value}]"##,
            r##"%W["a"#{"b"}]"##,
            r##"%W["a"#{value}"b"]"##,
            r##"%W[#{value}"a"]"##,
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
            assert!(
                crate::cops::testing::correction_offenses(
                    &super::ImplicitStringConcatenation,
                    source
                )
                .is_empty(),
                "{source:?}"
            );
        }
    }
    #[test]
    fn word_array_interpolation_bodies_still_report_literal_pairs() {
        for source in [r##"%W[#{"b" "c"}]"##, r##"%W["a"#{"b" "c"}]"##] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, source.find("\"b\"").unwrap() as u32 + 1)
            );
            assert_eq!(
                found[0].message,
                "Combine \"b\" and \"c\" into a single string literal, rather than using implicit string concatenation."
            );
        }
    }
    #[test]
    fn no_offense_lookalikes_and_different_physical_lines() {
        for source in [
            "abc",
            "'abc'",
            "array = ['abc'\\\n'def']",
            "'abc\ndef'",
            "array = [\"abc#{something}def#{something_else}\"]",
            "\"a\nb\"",
            "<<~DOC\ntext\nDOC\n",
            "%Q[a#{x}b]",
            "\"a\" + \"b\"",
            "# 'a' 'b'\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }
}
