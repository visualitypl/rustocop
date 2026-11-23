//! `Layout/EmptyLines` — no consecutive empty code lines.

use std::ops::Range;

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, style::correction::line_value_boundary},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct EmptyLines;

impl Cop for EmptyLines {
    fn name(&self) -> &'static str {
        "Layout/EmptyLines"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
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
        // The pinned prefilter requires three consecutive normalized newlines.
        // Whitespace-only lines are TrailingWhitespace's responsibility.
        let mut consecutive = 0;
        let possible = (1..=file.code_line_count() as u32).any(|line| {
            consecutive = if file.line_text(line).is_empty() {
                consecutive + 1
            } else {
                0
            };
            consecutive >= if line == consecutive { 3 } else { 2 }
        });
        if !possible {
            return;
        }
        let literals = file.literal_token_ranges(node);
        let lines = token_lines(file, literals, file.percent_array_separators(node));
        let boundary = cx
            .corrections_requested()
            .then(|| line_value_boundary(file));
        for gap in lines.windows(2).filter(|gap| gap[1] - gap[0] > 2) {
            for line in gap[0] + 1..gap[1] {
                if !file.line_text(line - 1).is_empty() || !file.line_text(line).is_empty() {
                    continue;
                }
                let range = file.line_start(line)..file.line_start(line + 1);
                if contains(literals, range.start) {
                    continue;
                }
                out.push(Offense {
                    correction: boundary
                        .filter(|&boundary| range.start >= boundary)
                        .filter(|_| !file.overlaps_literal(node, &range))
                        .map(|_| Correction {
                            range,
                            replacement: String::new(),
                        }),
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Extra blank line detected.".into(),
                    line,
                    end_line: line,
                    col: 1,
                });
            }
        }
    }
}

fn contains(ranges: &[Range<usize>], offset: usize) -> bool {
    let index = ranges.partition_point(|range| range.end <= offset);
    ranges
        .get(index)
        .is_some_and(|range| range.contains(&offset))
}

/// Lines bounding potential token gaps. Literal bytes protect internal blank
/// lines; percent-array separators can start on an otherwise empty line after
/// an escaped newline. Comments count at their opening, not at the end of a
/// block comment. Stop at Ruby's lexical EOF markers, never inspecting data.
/// One bounded scan using cached AST locations, no reparse.
fn token_lines(file: &SourceFile, literals: &[Range<usize>], separators: &[usize]) -> Vec<u32> {
    // The pinned cop starts its first gap at line 1, even without a token there.
    let mut lines = vec![1];
    let mut separator = 0;
    let mut offset = usize::from(file.bytes.starts_with(b"\xef\xbb\xbf")) * 3;
    let mut comment = 0;
    let mut line = 1;
    let end = file.data_start().unwrap_or(file.bytes.len());
    while offset < end {
        while file
            .comments()
            .get(comment)
            .is_some_and(|range| range.end <= offset)
        {
            comment += 1;
        }
        if let Some(range) = file.comments().get(comment)
            && range.start == offset
        {
            if lines.last() != Some(&line) {
                lines.push(line);
            }
            line += memchr::memchr_iter(b'\n', &file.bytes[range.clone()]).count() as u32;
            offset = range.end;
            continue;
        }
        let byte = file.bytes[offset];
        let literal = contains(literals, offset);
        if byte == b'\\' && !literal {
            let width = if file.bytes[offset..end].starts_with(b"\\\r\n") {
                3
            } else if file.bytes[offset..end].starts_with(b"\\\n") {
                2
            } else {
                0
            };
            if width > 0 {
                line += 1;
                offset += width;
                continue;
            }
        }
        if matches!(byte, 0 | 4 | 26) && !literal {
            break;
        }
        while separators
            .get(separator)
            .is_some_and(|&start| start < offset)
        {
            separator += 1;
        }
        let separator_start = separators.get(separator) == Some(&offset);
        if ((!byte.is_ascii_whitespace() && byte != b'\x0b') || literal || separator_start)
            && lines.last() != Some(&line)
        {
            lines.push(line);
        }
        line += u32::from(byte == b'\n');
        offset += 1;
    }
    lines
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/empty_lines_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::{
        cops::testing::assert_correction,
        offense::{Offense, Severity},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/EmptyLines")
            .collect()
    }

    #[test]
    fn pinned_offenses_and_corrections_preserve_one_empty_line() {
        for newline in ["\n", "\r\n"] {
            let before = format!("test = 5{newline}{newline}{newline}top{newline}");
            let after = format!("test = 5{newline}{newline}top{newline}");
            let found = offenses(&before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
            assert_eq!(found[0].message, "Extra blank line detected.");
            assert_eq!(found[0].severity, Severity::Convention);
            assert!(found[0].correction.is_none());
            assert_correction(&EmptyLines, &before, &after);
            assert!(offenses(&after).is_empty());
        }
    }

    #[test]
    fn corrections_delete_only_excess_line_terminators() {
        use crate::cops::testing::correction_offenses;
        for (before, range) in [("x\n\n\ny\n", 3..4), ("x\r\n\r\n\r\ny\r\n", 5..7)] {
            let found = correction_offenses(&EmptyLines, before);
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].correction,
                Some(Correction {
                    range,
                    replacement: String::new()
                })
            );
        }
    }

    #[test]
    fn multiple_runs_leading_and_trailing_boundaries() {
        for (before, after, lines) in [
            (
                "x\n\n\n\ny\n\n\nz\n\n\n",
                "x\n\ny\n\nz\n\n\n",
                vec![3, 4, 7],
            ),
            ("\n\n\nx\n", "\nx\n", vec![2, 3]),
            ("# comment\n\n\nx\n", "# comment\n\nx\n", vec![3]),
            ("x\n\n\n# comment\n", "x\n\n# comment\n", vec![3]),
            (
                "x\n=begin\n\n\n=end\ny\n",
                "x\n=begin\n\n=end\ny\n",
                vec![4],
            ),
            ("é = 1\r\n\n\r\nputs é\n", "é = 1\r\n\nputs é\n", vec![3]),
        ] {
            assert_eq!(
                offenses(before).iter().map(|o| o.line).collect::<Vec<_>>(),
                lines,
                "{before:?}"
            );
            assert_correction(&EmptyLines, before, after);
            assert!(offenses(after).is_empty(), "{after:?}");
        }
    }

    #[test]
    fn continued_newlines_are_not_tokens_at_the_file_boundaries() {
        for source in [
            "x\n\n\n\\\n",
            "# c\n\n\n \\\n",
            "\n\n\n\\\n",
            "\\\n\n\n\\\r\n",
            "=begin\n\n\n=end\n\n\n\\\n",
            "\\\n\nx\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        assert_correction(&EmptyLines, "\\\n\n\n\nx\n", "\\\n\nx\n");
    }

    #[test]
    fn vertical_tabs_are_whitespace_at_token_boundaries() {
        for (source, expected) in [
            ("x\n\n\n\u{b}\n", vec![]),
            ("\n\n\u{b}\n", vec![]),
            ("\u{b}\n\n\nx\n", vec![3]),
            ("\n\n\u{b}\nx\n", vec![]),
            ("\n\n\u{b}\nx\n\n\ny\n", vec![2, 6]),
        ] {
            assert_eq!(
                offenses(source).iter().map(|o| o.line).collect::<Vec<_>>(),
                expected,
                "{source:?}"
            );
        }
        assert_correction(&EmptyLines, "\u{b}\n\n\nx\n", "\u{b}\n\nx\n");
        assert_correction(&EmptyLines, "\n\n\u{b}\nx\n\n\ny\n", "\n\u{b}\nx\n\ny\n");
        assert!(
            crate::cops::testing::correction_offenses(&EmptyLines, "x\n\n\n\u{b}\n").is_empty()
        );
    }

    #[test]
    fn whitespace_only_lines_are_not_empty_lines() {
        for before in ["x\n \n\ny\n", "x\n\n\t\ny\n", "x\n\u{a0}\n\ny\n"] {
            // A non-ASCII blank would not parse in code; use it in a comment.
            let before = before.replace('\u{a0}', "#\u{a0}");
            assert!(offenses(&before).is_empty(), "{before:?}");
        }
        assert_correction(&EmptyLines, "x\n\n\n \ny\n", "x\n\n \ny\n");
    }

    #[test]
    fn literal_text_is_exempt_but_interpolation_code_is_checked() {
        for literal in [
            "'a\n\n\nb'",
            "\"a\n\n\nb\"",
            "/a\n\n\nb/",
            "%r(a\n\n\nb)",
            "%q(a\n\n\nb)",
            "%Q(a\n\n\nb)",
            ":\"a\n\n\nb\"",
            "`a\n\n\nb`",
            "<<H\n\n\n\nb\nH",
            "<<-'H'\na\n\n\nb\nH",
            "<<~H\n  a\n\n\n  b\nH",
            "[<<A, <<B]\n\n\n\na\nA\n\n\n\nb\nB",
        ] {
            let source = format!("x = {literal}\n");
            assert!(offenses(&source).is_empty(), "{source:?}");
        }
        for (before, after) in [
            ("x = \"#{1\n\n\n2}tail\"\n", "x = \"#{1\n\n2}tail\"\n"),
            (
                "x = <<~H\n  #{1\n\n\n2}tail\nH\n",
                "x = <<~H\n  #{1\n\n2}tail\nH\n",
            ),
        ] {
            assert_eq!(offenses(before).len(), 1);
            assert_correction(&EmptyLines, before, after);
        }
    }

    #[test]
    fn percent_array_separators_are_reported_but_not_rewritten() {
        for literal in [
            "%w(a\n\n\nb)",
            "%W(a\n\n\nb)",
            "%i(a\n\n\nb)",
            "%I(a\n\n\nb)",
            "%W(a#{1\n\n\n2})",
        ] {
            let before = format!("x = {literal}\n");
            assert_eq!(offenses(&before).len(), 1, "{before:?}");
            assert_correction(&EmptyLines, &before, &before);
        }
    }

    #[test]
    fn percent_array_separator_token_starts_bound_empty_line_gaps() {
        for kind in ["w", "W", "i", "I"] {
            for newline in ["\n", "\r\n"] {
                for tail in ["b", ""] {
                    let source = format!("%{kind}(a\\{newline}{newline}{newline}{tail}){newline}");
                    assert!(offenses(&source).is_empty(), "{source:?}");
                    assert!(
                        crate::cops::testing::correction_offenses(&EmptyLines, &source).is_empty()
                    );
                    let longer =
                        format!("%{kind}(a\\{newline}{newline}{newline}{newline}{tail}){newline}");
                    let found = offenses(&longer);
                    assert_eq!(
                        found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                        vec![(3, 1), (4, 1)],
                        "{longer:?}"
                    );
                    assert_correction(&EmptyLines, &longer, &longer);
                }
            }
        }
        for kind in ["W", "I"] {
            let source = format!("%{kind}(a#{{1}}\\\n\n\nb)\n");
            assert!(offenses(&source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn later_line_values_withhold_corrections_across_scopes() {
        for tail in [
            "p __LINE__\n",
            "def line; __LINE__; end\n",
            "class C\nLINE = __LINE__\nend\n",
            "x = \"#{__LINE__}\"\n",
            "# __LINE__ lookalike\n",
        ] {
            let before = format!("x\n\n\ny\n{tail}");
            assert_eq!(offenses(&before).len(), 1);
            assert_correction(&EmptyLines, &before, &before);
        }
        assert_correction(
            &EmptyLines,
            "p __LINE__\nx\n\n\ny\n",
            "p __LINE__\nx\n\ny\n",
        );
    }

    #[test]
    fn directives_filter_offenses_but_not_adjacent_newline_movement() {
        for prefix in ["rubocop", "rustocop"] {
            for target in ["Layout/EmptyLines", "EmptyLines", "Layout", "all"] {
                for action in ["disable", "todo"] {
                    let before = format!("# {prefix}:{action} {target}\nx\n\n\ny\n");
                    assert!(offenses(&before).is_empty(), "{before:?}");
                    assert!(
                        crate::cops::testing::correction_offenses(&EmptyLines, &before).is_empty()
                    );
                }
            }
            for directive in [
                "disable-next Lint/Debugger",
                "disable Lint/Debugger",
                "enable Lint/Debugger",
            ] {
                let before = format!("x\n\n\n# {prefix}:{directive}\ny\n");
                assert_eq!(offenses(&before).len(), 1);
                assert_correction(&EmptyLines, &before, &before.replacen("\n\n\n", "\n\n", 1));
            }
            let before = format!("# {prefix}:disable-next Layout/EmptyLines\n\n\nx\n");
            // The disabled next line is the retained first blank, not the
            // extra blank we remove. Its scope is unchanged.
            assert_correction(&EmptyLines, &before, &before.replacen("\n\n\n", "\n\n", 1));
        }
    }

    #[test]
    fn data_eof_markers_and_comment_only_boundaries() {
        for source in [
            "# comment\n\n\n",
            "=begin\n\n\n=end\n",
            "x\n\n\n__END__\n\n\nraw",
            "x\n\n\n\x04\ny\n",
            "x\n\n\n\x1a\ny\n",
            "x\n\n\n\0\ny\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        assert_correction(
            &EmptyLines,
            "x\n\n\ny\n__END__\n\n\nraw",
            "x\n\ny\n__END__\n\n\nraw",
        );
    }

    #[test]
    fn mixed_trailing_whitespace_fixes_converge() {
        use crate::cops::layout::TrailingWhitespace;
        let mut source = b"x\r\n \t\r\n\r\n\r\ny\r\n".to_vec();
        for _ in 0..3 {
            let found = crate::pipeline::lint_str_with_corrections(
                &[&EmptyLines, &TrailingWhitespace],
                std::str::from_utf8(&source).unwrap(),
            );
            source = crate::corrections::apply(&source, &found).unwrap().bytes;
        }
        assert_eq!(source, b"x\r\n\r\ny\r\n");
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "",
            "\n\n\n",
            "#comment",
            "test\n\n#comment\ntop\n",
            "result = \"test\n\n\n\n                                  string\"\n",
            "str = <<-TEXT\nline 1\n\n\nline 2\nTEXT\nputs str\n",
            "test\n\n\n",
            "test\n \n\ntop\n",
            "test\n\n\t\ntop\n",
            "\n\ntest\n",
            "__END__\n\n\nraw\n",
            "test\n\n\n__END__\nraw\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }
}
