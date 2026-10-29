//! `Style/MagicCommentFormat` — snake_case, lowercase directives; values unchanged.
use super::magic_comment::{MagicComment, directives, leading_end, trim_end};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MagicCommentFormat;
impl Cop for MagicCommentFormat {
    fn name(&self) -> &'static str {
        "Style/MagicCommentFormat"
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
        if node.as_program_node().is_none() {
            return;
        }
        // RuboCop requires an actual AST, not a comment-only source.
        if node
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .is_empty()
        {
            return;
        }
        let end = leading_end(file);
        let mut issues = Vec::new();
        for comment in file.comments().iter().take_while(|c| c.start < end) {
            let text = &file.bytes[comment.clone()];
            if !MagicComment::parse(text).valid {
                continue;
            }
            for range in directives(text) {
                if text[range.clone()]
                    .iter()
                    .any(|b| *b == b'-' || b.is_ascii_uppercase())
                {
                    // Upstream's unanchored range regex can match inside a
                    // value or editor metadata. Detect it, but only normalize
                    // actual directive positions, never arbitrary value bytes.
                    let prefix = trim_end(&text[..range.start]);
                    let key_position =
                        prefix == b"#" || prefix.ends_with(b"-*-") || prefix.ends_with(b";");
                    let suffix = &text[range.end..];
                    let spaces = suffix
                        .iter()
                        .take_while(|b| super::is_ruby_whitespace(**b))
                        .count();
                    let key_position = key_position && suffix[spaces..].starts_with(b":");
                    let correction =
                        (cx.corrections_requested() && key_position).then(|| Correction {
                            range: comment.start + range.start..comment.start + range.end,
                            replacement: text[range.clone()]
                                .iter()
                                .map(|b| {
                                    if *b == b'-' {
                                        '_'
                                    } else {
                                        b.to_ascii_lowercase() as char
                                    }
                                })
                                .collect(),
                        });
                    issues.push((comment.start + range.start, correction));
                }
            }
        }
        let positions = file.line_cols(issues.iter().map(|(start, _)| *start));
        for ((_, correction), (line, col)) in issues.into_iter().zip(positions) {
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: "Prefer lower snake case for magic comments.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/magic_comment_format_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov), at fixed snake/lowercase settings.
    use super::*;
    use crate::pipeline::lint_str_with;
    #[test]
    fn pinned_directive_ranges_and_message() {
        for (source, expected) in [
            (
                "# frozen-string-literal: true\n# encoding: utf-8\n# shareable-constant-value: literal\n# typed: ignore\nputs 1\n",
                vec![(1, 3), (3, 3)],
            ),
            ("# warn-indent: true\nputs 1\n", vec![(1, 3)]),
            (
                "# -*- encoding: ASCII-8BIT; frozen-string-literal: true -*-\nputs 1\n",
                vec![(1, 29)],
            ),
            ("# frozen-string_literal: true\nputs 1\n", vec![(1, 3)]),
            ("# eNcOdInG: utf-8\nputs 1\n", vec![(1, 3)]),
        ] {
            let found = lint_str_with(&[&MagicCommentFormat], source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                expected,
                "{source}"
            );
            for o in found {
                assert_eq!(o.message, "Prefer lower snake case for magic comments.");
            }
        }
    }
    #[test]
    fn pinned_corrections_and_values_are_unchanged() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "# frozen-string-literal: true\n# shareable-constant-value: literal\nputs 1\n",
                "# frozen_string_literal: true\n# shareable_constant_value: literal\nputs 1\n",
            ),
            (
                "# warn-indent: true\nputs 1\n",
                "# warn_indent: true\nputs 1\n",
            ),
            (
                "# -*- encoding: ASCII-8BIT; frozen-string-literal: true -*-\nputs 1\n",
                "# -*- encoding: ASCII-8BIT; frozen_string_literal: true -*-\nputs 1\n",
            ),
            (
                "# frozen-string_literal: true\nputs 1\n",
                "# frozen_string_literal: true\nputs 1\n",
            ),
            ("# eNcOdInG: UtF-8\nputs 1\n", "# encoding: UtF-8\nputs 1\n"),
            (
                "\u{feff}# FROZEN-STRING-LITERAL: TRUE\r\np __LINE__\r\n__END__\r\n# FROZEN-STRING-LITERAL: true\r\n",
                "\u{feff}# frozen_string_literal: TRUE\r\np __LINE__\r\n__END__\r\n# FROZEN-STRING-LITERAL: true\r\n",
            ),
        ] {
            assert_correction(&MagicCommentFormat, before, after);
        }
    }

    #[test]
    fn unanchored_upstream_ranges_in_values_are_reported_but_not_rewritten() {
        let source = "# typed: FROZEN-STRING-LITERAL\nx\n";
        let found = crate::cops::testing::correction_offenses(&MagicCommentFormat, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 10));
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn editor_metadata_key_lookalikes_are_detected_but_not_corrected() {
        for source in [
            "# -*- encoding: UTF-8; FROZEN-STRING-LITERAL -*-\nx\n",
            "# -*- encoding: UTF-8; FROZEN-STRING-LITERALcustom: true -*-\nx\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&MagicCommentFormat, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn many_ranges_on_one_line_use_batched_positions_and_linear_prefix_checks() {
        let source =
            "# -*- ".to_owned() + &"frozen-string-literal: true; ".repeat(5000) + "-*-\nx\n";
        let found = crate::cops::testing::correction_offenses(&MagicCommentFormat, &source);
        assert_eq!(found.len(), 5000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(found[4999].col, 7 + 29 * 4999);
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        assert_eq!(
            applied.bytes,
            source
                .replace("frozen-string-literal", "frozen_string_literal")
                .as_bytes()
        );
    }

    #[test]
    fn vertical_tab_keys_and_first_code_line_comments() {
        for (before, after) in [
            (
                "#\u{b}FROZEN-STRING-LITERAL: true\nx\n",
                "#\u{b}frozen_string_literal: true\nx\n",
            ),
            (
                "# -*- \u{b}frozen-string-literal: true\u{b} -*-\nx\n",
                "# -*- \u{b}frozen_string_literal: true\u{b} -*-\nx\n",
            ),
            (
                "x # FROZEN-STRING-LITERAL: true\n",
                "x # frozen_string_literal: true\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&MagicCommentFormat, before, after);
        }
    }

    #[test]
    fn exact_validity_and_header_scope_not_keyword_substrings() {
        for source in [
            "# FROZEN-STRING-LITERAL: true\n", // no AST
            "# ENCODING:utf-8\nx\n",           // encoding requires colon-space
            "# ENCODING:  utf-8\nx\n",
            "# -*- FROZEN-STRING-LITERAL: true -*-\nx\n", // editor validity is case-sensitive
            "# rbs_inline: ENABLED\nx\n",
            "# vim: fileencoding=utf-8\nx\n", // Vim requires two tokens
        ] {
            assert!(
                lint_str_with(&[&MagicCommentFormat], source).is_empty(),
                "{source}"
            );
        }
        let found = lint_str_with(
            &[&MagicCommentFormat],
            "# ordinary\n\n  # FROZEN-STRING-LITERAL: true\nx\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 5));
    }

    #[test]
    fn pinned_accepted_comments_and_lookalikes() {
        for source in [
            "",
            " ",
            "# frozen_string_literal: true\n# encoding: utf-8\n# shareable_constant_value: literal\n# typed: ignore\nputs 1\n",
            "# -*- encoding: ASCII-8BIT; frozen_string_literal: true -*-\nputs 1\n",
            "# foo-bar-baz ",
            "puts 1\n# frozen-string-literal: true\n",
            "# encoding: UtF-8\nputs 1\n",
            "x = <<~H\n# frozen-string-literal: true\nH\n",
            "__END__\n# frozen-string-literal: true\n",
        ] {
            assert!(
                lint_str_with(&[&MagicCommentFormat], source).is_empty(),
                "{source}"
            );
        }
    }
}
