//! `Layout/IndentationStyle` — spaces, with two spaces per indentation tab.

use std::ops::Range;

use ruby_prism::{Location, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct IndentationStyle;

impl Cop for IndentationStyle {
    fn name(&self) -> &'static str {
        "Layout/IndentationStyle"
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
        // Detection excludes whole str/dstr expressions, even interpolation
        // code. This is intentionally different from correction safety's
        // literal-text-only ranges. Build once, only if a tab is encountered.
        let mut strings = None;
        for line in 1..=file.code_line_count() as u32 {
            let text = file.line_text(line);
            // Ruby's /\s/ is ASCII. The greedy match ends at the final tab,
            // not at any following spaces: /\A\s*\t+/.
            let prefix = text
                .iter()
                .take_while(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c'))
                .count();
            let Some(last_tab) = text[..prefix].iter().rposition(|&b| b == b'\t') else {
                continue;
            };
            let range = file.line_start(line)..file.line_start(line) + last_tab + 1;
            let strings = strings.get_or_insert_with(|| string_ranges(file, node));
            let index = strings.partition_point(|string| string.end <= range.start);
            if strings
                .get(index)
                .is_some_and(|string| string.start <= range.start && string.end >= range.end)
            {
                continue;
            }
            out.push(Offense {
                correction: (cx.corrections_requested() && !file.overlaps_literal(node, &range))
                    .then(|| Correction {
                        replacement: String::from_utf8(text[..=last_tab].to_vec())
                            .expect("ASCII whitespace")
                            .replace('\t', "  "),
                        range,
                    }),
                cop: self.name(),
                severity: self.severity(),
                message: "Tab detected in indentation.".into(),
                line,
                end_line: line,
                col: 1,
            });
        }
    }
}

/// RuboCop's `string_literal_ranges`: only str/dstr nodes with delimiters,
/// not regexps, symbols, command strings or bare percent-array elements.
/// Merge nested ranges so every line query is logarithmic, not lines × nodes.
fn string_ranges(file: &SourceFile, root: &Node<'_>) -> Vec<Range<usize>> {
    struct Strings<'a> {
        file: &'a SourceFile,
        ranges: Vec<Range<usize>>,
    }

    impl Strings<'_> {
        fn add(
            &mut self,
            opening: Option<Location<'_>>,
            closing: Option<Location<'_>>,
            body: Range<usize>,
        ) {
            let (Some(opening), Some(closing)) = (opening, closing) else {
                return;
            };
            let range = if opening.as_slice().starts_with(b"<<") {
                // The pinned translator joins the first and last body parts,
                // not the terminator. A trailing nested heredoc can therefore
                // lie beyond the outer body's exemption.
                self.file.line_start(self.file.line_of(body.start))..body.end
            } else {
                opening.start_offset()..closing.end_offset()
            };
            self.ranges.push(range);
        }
    }

    impl<'pr> NodeVisitor<'pr> for Strings<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            match node {
                Node::StringNode { .. } => {
                    let n = node.as_string_node().unwrap();
                    self.add(
                        n.opening_loc(),
                        n.closing_loc(),
                        n.content_loc().start_offset()..n.content_loc().end_offset(),
                    );
                }
                Node::InterpolatedStringNode { .. } => {
                    let n = node.as_interpolated_string_node().unwrap();
                    let start = n
                        .parts()
                        .iter()
                        .next()
                        .map_or(n.location().end_offset(), |part| {
                            part.location().start_offset()
                        });
                    let end = n
                        .parts()
                        .iter()
                        .last()
                        .map_or(start, |part| part.location().end_offset());
                    self.add(n.opening_loc(), n.closing_loc(), start..end);
                }
                _ => {}
            }
        }
    }

    let mut strings = Strings {
        file,
        ranges: Vec::new(),
    };
    walk(&mut strings, root);
    strings
        .ranges
        .sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in strings.ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else if !range.is_empty() {
            merged.push(range);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/indentation_style_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        offense::Offense,
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
            .filter(|o| o.cop == "Layout/IndentationStyle")
            .collect()
    }

    #[test]
    fn pinned_string_and_heredoc_exemptions_include_interpolation_code() {
        for source in [
            "x = '\n\thello'",
            "x = <<HELLO\n\thello\nHELLO",
            "x = <<HELLO\n\thello\n\t\n\t\t\nhello\nHELLO",
            "x = \"a\n#{\n\t1\n}b\"\n",
            "x = <<~H\n\t#{\n\t1\n\t}tail\nH\n",
            "x = [<<~A, <<~B]\n\ta\nA\n\tb\nB\n",
            "x = <<~A\n\t#{<<~B}\n\tb\nB\n\ta\nA\n",
            "x = %q(a\n\tb)\n",
            "x = %Q(a\n#{\n\t1\n}b)\n",
            "x = <<~'H'\n\t\n\t\t\nH\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn trailing_nested_command_heredoc_is_reported_but_not_corrected() {
        for opening in ["<<H", "<<-H", "<<~H"] {
            let source = format!("x = {opening}\n\t#{{<<`B`}}\n\tb\nB\nH\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), (3, 1));
            assert_eq!(found[0].message, "Tab detected in indentation.");
            let found = correction_offenses(&IndentationStyle, &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
            assert_correction(&IndentationStyle, &source, &source);

            // A later outer part extends its body range over the nested body.
            let with_tail = source.replace("B\nH\n", "B\n\ttail\nH\n");
            assert!(offenses(&with_tail).is_empty(), "{with_tail:?}");
            assert!(correction_offenses(&IndentationStyle, &with_tail).is_empty());

            // A nested ordinary string heredoc is independently exempt.
            let nested_string = source.replace("<<`B`", "<<B");
            assert!(offenses(&nested_string).is_empty(), "{nested_string:?}");
            assert!(correction_offenses(&IndentationStyle, &nested_string).is_empty());
        }
    }

    #[test]
    fn pinned_data_section_is_not_code() {
        let before = "\tx = 0\n__END__\n\tx = 0\n";
        let after = "  x = 0\n__END__\n\tx = 0\n";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_correction(&IndentationStyle, before, after);
        assert!(offenses("__END__\n\tdata\n").is_empty());
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "",
            "\n",
            "  \n",
            "x = 0\n",
            "        x = 0\n",
            "foo \t bar",
            "'foo'\t'bar'",
            "(x = \"\t\")",
            "x = 0 # \t comment\n",
            "# \tcomment\n",
            "\u{a0}\tx = 0\n",
            // The pinned line regexp does not strip a BOM before matching.
            "\u{feff}\tx = 0\r\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn exact_range_stops_at_last_tab_and_preserves_other_whitespace() {
        let before = " \t  \t  x = 0\n";
        let found = correction_offenses(&IndentationStyle, before);
        assert_eq!(found.len(), 1);
        let fix = found[0].correction.as_ref().unwrap();
        assert_eq!(fix.range, 0..5);
        assert_eq!(fix.replacement, "       ");
        for prefix in ["\x0b", "\x0c", "\r"] {
            assert_correction(
                &IndentationStyle,
                &format!("{prefix}\tx = 0\n"),
                &format!("{prefix}  x = 0\n"),
            );
        }
    }

    #[test]
    fn comments_blank_lines_utf8_bom_crlf_and_no_final_newline() {
        let before = "\u{feff}# 日本語\r\n\t# é\r\n \t  \n\tputs \"é\"\r\n\t";
        let after = "\u{feff}# 日本語\r\n  # é\r\n     \n  puts \"é\"\r\n  ";
        let found = offenses(before);
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.end_line, o.col))
                .collect::<Vec<_>>(),
            [(2, 2, 1), (3, 3, 1), (4, 4, 1), (5, 5, 1)]
        );
        assert_correction(&IndentationStyle, before, after);
        assert_correction(
            &IndentationStyle,
            "=begin\n\tdoc\n=end\n",
            "=begin\n  doc\n=end\n",
        );
    }

    #[test]
    fn non_string_literal_tabs_are_reported_but_never_rewritten() {
        for literal in [
            "\"a\n\tb\"",
            "'a\n\tb'",
            "%q(a\n\tb)",
            "%Q(a\n\tb)",
            "/a\n\tb/",
            "%r(a\n\tb)",
            "%w(a\n\tb)",
            "%W(a\n\tb#{1})",
            "%i(a\n\tb)",
            "%I(a\n\tb#{1})",
            "%x(a\n\tb)",
            "`a\n\tb`",
            ":\"a\n\tb\"",
            "%s(a\n\tb)",
        ] {
            let before = format!("x = {literal}\n\tputs x\n");
            let after = format!("x = {literal}\n  puts x\n");
            let string_exempt = literal.starts_with(['\'', '"'])
                || literal.starts_with("%q")
                || literal.starts_with("%Q");
            let found = offenses(&before);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                if string_exempt {
                    vec![(3, 1)]
                } else {
                    vec![(2, 1), (3, 1)]
                },
                "{before:?}"
            );
            assert_correction(&IndentationStyle, &before, &after);
        }
    }

    #[test]
    fn heredoc_bodies_and_terminators_are_safe_even_with_command_strings() {
        for prefix in ["<<-", "<<~"] {
            for quote in ["", "'", "\"", "`"] {
                let source = format!("x = {prefix}{quote}H{quote}\n\ta\n\tH\n");
                let found = offenses(&source);
                assert_eq!(
                    found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                    if quote == "`" {
                        vec![(2, 1), (3, 1)]
                    } else {
                        vec![(3, 1)]
                    },
                    "{source:?}"
                );
                assert_correction(&IndentationStyle, &source, &source);
            }
        }
    }

    #[test]
    fn interpolation_detection_and_fix_safety_are_distinct() {
        for (before, after, positions) in [
            // dstr is excluded in full, not just its literal segments.
            (
                "x = \"#{\n\t1\n}\"\n\tputs x\n",
                "x = \"#{\n\t1\n}\"\n  puts x\n",
                vec![(4, 1)],
            ),
            // Regexp interpolation is ordinary code; its text is not.
            (
                "x = /a\n\t#{\n\t1\n}\n\tb/\n",
                "x = /a\n\t#{\n  1\n}\n\tb/\n",
                vec![(2, 1), (3, 1), (5, 1)],
            ),
            // Percent arrays remain conservatively protected as a whole.
            (
                "x = %W(a#{\n\t1\n})\n",
                "x = %W(a#{\n\t1\n})\n",
                vec![(2, 1)],
            ),
            // Quoted symbols expose interpolation but not nested literal text.
            (
                "x = :\"#{\n\t'a\n\tb'\n}\"\n",
                "x = :\"#{\n  'a\n\tb'\n}\"\n",
                vec![(2, 1)],
            ),
        ] {
            assert_eq!(
                offenses(before)
                    .iter()
                    .map(|o| (o.line, o.col))
                    .collect::<Vec<_>>(),
                positions,
                "{before:?}"
            );
            assert_correction(&IndentationStyle, before, after);
        }
    }

    #[test]
    fn both_directive_prefixes_protect_whole_indentation_ranges() {
        for prefix in ["rustocop", "rubocop"] {
            for target in [
                "Layout/IndentationStyle",
                "IndentationStyle",
                "Layout",
                "all",
            ] {
                let before = format!(
                    "# {prefix}:disable {target}\n\tx = 0\n# {prefix}:enable {target}\n\tx = 0 # {prefix}:disable {target}\n# {prefix}:disable-next {target}\n\tx = 0\n\ty = 0\n"
                );
                let after = before.strip_suffix("\ty = 0\n").unwrap().to_owned() + "  y = 0\n";
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col), (7, 1));
                assert_correction(&IndentationStyle, &before, &after);
            }
        }
    }

    #[test]
    fn many_nested_strings_use_merged_ranges_not_per_line_ast_walks() {
        let source = "x = \"text\n#{\"nested\n\tliteral\"}\n\ttail\"\n\tputs x\n".repeat(2000);
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        let ranges = string_ranges(&file, &parsed.node());
        assert_eq!(ranges.len(), 2000);
        assert_eq!(offenses(&source).len(), 2000);
        let found = correction_offenses(&IndentationStyle, &source);
        assert_eq!(found.len(), 2000);
        assert!(found.iter().all(|o| o.correction.is_some()));
    }

    #[test]
    fn pinned_tabs_and_mixed_indentation() {
        for (before, after) in [
            ("\tx = 0\n", "  x = 0\n"),
            ("\t\t\tx = 0\n", "      x = 0\n"),
            (" \tx = 0\n", "   x = 0\n"),
            ("\t(x = \"\t\")\n", "  (x = \"\t\")\n"),
            (" \t  \t  x = 0\n", "         x = 0\n"),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before:?}");
            assert_eq!((found[0].line, found[0].end_line, found[0].col), (1, 1, 1));
            assert_eq!(found[0].message, "Tab detected in indentation.");
            assert_eq!(found[0].severity, crate::offense::Severity::Convention);
            assert!(found[0].correction.is_none());
            assert_correction(&IndentationStyle, before, after);
            assert!(offenses(after).is_empty());
        }
    }
}
