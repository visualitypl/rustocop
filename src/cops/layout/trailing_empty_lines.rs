//! `Layout/TrailingEmptyLines` — fixed `final_newline` policy.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct TrailingEmptyLines;

impl Cop for TrailingEmptyLines {
    fn name(&self) -> &'static str {
        "Layout/TrailingEmptyLines"
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
        // One bounded source scan at the root, reusing the pipeline's parse.
        // Preserve the pinned cop's broad __END__ and percent-blank exceptions.
        let bytes = &file.bytes;
        if node.as_program_node().is_none()
            || bytes.is_empty()
            || file.contains(b"__END__")
            || [b"%\n\n".as_slice(), b"%\r\n\r\n", b"%\r\n\n", b"%\n\r\n"]
                .iter()
                .any(|ending| bytes.ends_with(ending))
        {
            return;
        }
        // Ruby's /\s/ is ASCII whitespace, not Unicode's whitespace class.
        let start = bytes
            .iter()
            .rposition(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c))
            .map_or(0, |i| i + 1);
        let suffix = &bytes[start..];
        let newlines = suffix.iter().filter(|&&b| b == b'\n').count();
        if newlines == 1 {
            return;
        }
        // RuboCop reports just after the first whitespace character. Its
        // buffer normalizes CRLF; translate that position back to raw bytes.
        let report_start = start
            + if suffix.starts_with(b"\r\n") {
                2
            } else {
                usize::from(!suffix.is_empty())
            };
        let (line, col) = file.line_col(report_start);
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| correction(file, node, start))
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: if newlines == 0 {
                "Final newline missing.".into()
            } else {
                format!("{} trailing blank lines detected.", newlines - 1)
            },
            line,
            end_line: file.line_of(bytes.len()),
            col,
        });
    }
}

fn correction(file: &SourceFile, root: &Node<'_>, start: usize) -> Option<Correction> {
    let suffix = &file.bytes[start..];
    // Keep the final code line's existing terminator. For a missing one,
    // inherit the nearest preceding terminator, defaulting to LF in one-line
    // files. Mixed endings elsewhere are never normalized.
    let newline_offset = suffix
        .iter()
        .position(|&b| b == b'\n')
        .map(|i| start + i)
        .or_else(|| file.bytes[..start].iter().rposition(|&b| b == b'\n'));
    let newline = if newline_offset.is_some_and(|i| i > 0 && file.bytes[i - 1] == b'\r') {
        "\r\n"
    } else {
        "\n"
    };
    // Retain any already-canonical prefix, especially a heredoc terminator's
    // newline: deleting only the excess blanks need not touch literal bytes.
    let common = suffix
        .iter()
        .zip(newline.as_bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let range = start + common..file.bytes.len();
    if file.overlaps_literal(root, &range) {
        return None;
    }
    Some(Correction {
        range,
        replacement: newline[common..].into(),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/trailing_empty_lines_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::assert_correction,
        offense::{Offense, Severity},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/TrailingEmptyLines")
            .collect()
    }

    #[test]
    fn pinned_offense_ranges_and_messages() {
        for (source, line, col, end_line, message) in [
            ("x = 0", 1, 6, 1, "Final newline missing."),
            ("x = 0\n\n\n\n", 2, 1, 5, "3 trailing blank lines detected."),
            ("\n\n\n\n", 2, 1, 5, "3 trailing blank lines detected."),
            (
                "x = 0\n\n \n\n\n",
                2,
                1,
                6,
                "4 trailing blank lines detected.",
            ),
            ("x = 0 \n\n", 1, 7, 3, "1 trailing blank lines detected."),
            ("  ", 1, 2, 1, "Final newline missing."),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line),
                "{source:?}"
            );
            assert_eq!(found[0].message, message);
            assert_eq!(found[0].severity, Severity::Convention);
            assert!(
                found[0].correction.is_none(),
                "ordinary lint does not build corrections"
            );
        }
    }

    #[test]
    fn newline_conventions_and_preceding_bytes_are_preserved() {
        for (before, after) in [
            ("x = 0\r\n\r\n\r\n", "x = 0\r\n"),
            ("\r\n\r\n", "\r\n"),
            ("x = 0 \t\r\n \t\r\n", "x = 0\r\n"),
            ("# earlier\r\nx = 0", "# earlier\r\nx = 0\r\n"),
            (
                "# earlier\r\n# nearest\nx = 0",
                "# earlier\r\n# nearest\nx = 0\n",
            ),
            ("# earlier\nx = 0\r\n\n", "# earlier\nx = 0\r\n"),
            ("# earlier\r\nx = 0\n\r\n", "# earlier\r\nx = 0\n"),
            (
                "\u{feff}#!/usr/bin/ruby\r\n# coding: UTF-8\r\nputs \"日本語\"",
                "\u{feff}#!/usr/bin/ruby\r\n# coding: UTF-8\r\nputs \"日本語\"\r\n",
            ),
            ("\u{feff}", "\u{feff}\n"),
            ("é = 1", "é = 1\n"),
        ] {
            assert_correction(&TrailingEmptyLines, before, after);
            assert!(offenses(after).is_empty(), "{after:?}");
        }
        for (source, line, col) in [
            ("x = 0\r\n\r\n", 2, 1),
            ("é = 1", 1, 6),
            ("\u{feff}é = 1", 1, 6),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (line, col));
        }
        for source in ["x = 0\r\n", "\r\n", "%\r\n\r\n", "x = %\r\n\r\n"] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn corrections_touch_only_the_required_eof_bytes() {
        use crate::{cops::testing::correction_offenses, offense::Correction};
        for (source, range, replacement) in [
            ("x = 0", 5..5, "\n"),
            ("x = 0\n\n\n", 6..8, ""),
            ("x = 0\r\n\r\n", 7..9, ""),
            ("x = 0 \n\n", 5..8, "\n"),
            ("x = <<H\nbody\nH\n\n", 15..16, ""),
        ] {
            let found = correction_offenses(&TrailingEmptyLines, source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(
                found[0].correction,
                Some(Correction {
                    range,
                    replacement: replacement.into()
                }),
                "{source:?}"
            );
        }
    }

    #[test]
    fn pinned_no_offense_examples() {
        for source in [
            "",
            "\n",
            "x = 0\n",
            "x = 0\n  ",
            "x = 0\n\n__END__\n\n",
            "__END__\n\n\n",
            "%\n\n",
            "x = %\n\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn multiline_literals_and_heredoc_terminators_are_preserved() {
        for literal in [
            "'a\n\nb'",
            "\"a\n#{1}\nb\"",
            "/a\n\nb/",
            "%q(a\n\nb)",
            "%w(a\n\nb)",
        ] {
            let code = format!("x = {literal}");
            assert_correction(&TrailingEmptyLines, &code, &format!("{code}\n"));
            assert_correction(
                &TrailingEmptyLines,
                &format!("{code}\n\n\n"),
                &format!("{code}\n"),
            );
        }
        for opener in ["<<H", "<<-H", "<<~H", "<<'H'", "<<~\"H\""] {
            for newline in ["\n", "\r\n"] {
                let code = format!("x = {opener}{newline}  text  {newline} {newline}H");
                assert_correction(&TrailingEmptyLines, &code, &format!("{code}{newline}"));
                assert_correction(
                    &TrailingEmptyLines,
                    &format!("{code}{newline}{newline} \t{newline}"),
                    &format!("{code}{newline}"),
                );
            }
        }
    }

    #[test]
    fn whitespace_tokens_are_reported_but_unsafe_rewrites_are_withheld() {
        for source in [
            "x = %q a ",
            "x = %Q a#{1} ",
            "x = %r\ta\t",
            "x = ?\\ ",
            "x = %\n\n\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source:?}");
            assert_correction(&TrailingEmptyLines, source, source);
        }
    }

    #[test]
    fn data_and_pinned_end_lookalikes_are_exempt() {
        for source in [
            "__END__",
            "__END__\nraw",
            "puts 1\n\n__END__\r\nraw \t\r\n\r\n",
            "# __END__ lookalike",
            "x = '__END__'\n\n",
            "x = <<H\n__END__\nH\n\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
            assert!(
                crate::cops::testing::correction_offenses(&TrailingEmptyLines, source).is_empty()
            );
        }
    }

    #[test]
    fn ascii_suffix_blanks_are_not_unicode_blanks() {
        for (before, after) in [
            ("x = 0\t \n \t\x0b\x0c\n", "x = 0\n"),
            ("# é\u{a0}", "# é\u{a0}\n"),
            ("# é\u{3000}\n\n", "# é\u{3000}\n"),
        ] {
            assert_correction(&TrailingEmptyLines, before, after);
        }
    }

    #[test]
    fn directives_filter_detection_and_corrections() {
        for prefix in ["rustocop", "rubocop"] {
            for target in [
                "Layout/TrailingEmptyLines",
                "TrailingEmptyLines",
                "Layout",
                "all",
            ] {
                for before in [
                    format!("# {prefix}:disable {target}\nputs 1\n\n"),
                    format!("puts 1 # {prefix}:disable {target}"),
                    format!("# {prefix}:disable-next {target}\nputs 1"),
                    format!("# {prefix}:todo {target}\nputs 1\n\n"),
                ] {
                    assert!(offenses(&before).is_empty(), "{before}");
                    assert!(
                        crate::cops::testing::correction_offenses(&TrailingEmptyLines, &before)
                            .is_empty(),
                        "{before}"
                    );
                }
                let before = format!(
                    "# {prefix}:disable {target}\nputs 1\n# {prefix}:enable {target}\nputs 2\n\n"
                );
                let after = before.strip_suffix('\n').unwrap();
                assert_correction(&TrailingEmptyLines, &before, after);
                // Removing excess EOF lines must not move this directive or
                // delete the newline terminating its single-line scope.
                let before = format!("puts 1 # {prefix}:disable {target}\n\n");
                assert_correction(
                    &TrailingEmptyLines,
                    &before,
                    before.strip_suffix('\n').unwrap(),
                );
            }
        }
    }

    #[test]
    fn mixed_trailing_whitespace_edits_converge_without_changing_other_bytes() {
        use crate::cops::layout::TrailingWhitespace;
        for (before, after) in [
            ("puts \"é\" \t\r\n \t\r\n\r\n", "puts \"é\"\r\n"),
            ("puts 1\n \t", "puts 1\n"),
            (" \t", "\n"),
        ] {
            let mut source = before.as_bytes().to_vec();
            for _ in 0..3 {
                let text = std::str::from_utf8(&source).unwrap();
                let found = crate::pipeline::lint_str_with_corrections(
                    &[&TrailingEmptyLines, &TrailingWhitespace],
                    text,
                );
                source = crate::corrections::apply(&source, &found).unwrap().bytes;
            }
            assert_eq!(source, after.as_bytes(), "{before:?}");
            assert!(offenses(after).is_empty());
        }
    }

    #[test]
    fn pinned_corrections() {
        for (before, after) in [
            ("x = 0", "x = 0\n"),
            ("x = 0\n\n\n\n", "x = 0\n"),
            ("\n\n\n\n", "\n"),
            ("x = 0\n\n \n\n\n", "x = 0\n"),
            (
                "puts 'testing rubocop when final new line is missing\n                          after block comments'\n\n=begin\nfirst line\nsecond line\nthird line\n=end",
                "puts 'testing rubocop when final new line is missing\n                          after block comments'\n\n=begin\nfirst line\nsecond line\nthird line\n=end\n",
            ),
        ] {
            assert_correction(&TrailingEmptyLines, before, after);
        }
    }
}
