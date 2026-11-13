//! `Layout/TrailingWhitespace` — pinned `AllowInHeredoc: false` detection.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct TrailingWhitespace;

impl Cop for TrailingWhitespace {
    fn name(&self) -> &'static str {
        "Layout/TrailingWhitespace"
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
        // One line scan per file, never one scan per AST node.
        if node.as_program_node().is_none() {
            return;
        }
        for line in 1..=file.code_line_count() as u32 {
            let text = file.line_text(line);
            let start = trailing_start(text, file.encoding());
            if start == text.len() {
                continue;
            }
            let range = file.line_start(line) + start..file.line_start(line) + text.len();
            let (_, col) = file.line_col(range.start);
            out.push(Offense {
                correction: (cx.corrections_requested()
                    && !file.overlaps_literal(node, &range)
                    // A parseable edit must not turn an identifier into the
                    // __END__ marker, silently moving later code into data.
                    && text[..start].strip_prefix(b"\xef\xbb\xbf").unwrap_or(&text[..start]) != b"__END__"
                    // Non-ASCII blanks are valid Ruby identifier bytes, not
                    // lexical whitespace. Only remove them inside comments.
                    && (text[start..].is_ascii() || in_comment(file, &range)))
                .then(|| Correction {
                    range,
                    replacement: String::new(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: "Trailing whitespace detected.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

fn in_comment(file: &SourceFile, range: &std::ops::Range<usize>) -> bool {
    let comments = file.comments();
    let index = comments.partition_point(|comment| comment.start <= range.start);
    index > 0 && comments[index - 1].end >= range.end
}

// Ruby/Onigmo's [[:blank:]], not Rust's broader Unicode whitespace class.
fn blank(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn trailing_start(text: &[u8], encoding: crate::source::SourceEncoding) -> usize {
    use crate::source::SourceEncoding;

    if encoding == SourceEncoding::Utf8 {
        // Decode only the suffix: CESU-8 may have surrogate bytes earlier in
        // the line, but all of Onigmo's blanks have the same bytes as UTF-8.
        let mut end = text.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && text[start] & 0xc0 == 0x80 {
                start -= 1;
            }
            let Ok(character) = std::str::from_utf8(&text[start..end]) else {
                break;
            };
            if !character.chars().next().is_some_and(blank) {
                break;
            }
            end = start;
        }
        return end;
    }
    text.iter()
        .rposition(|&b| {
            !matches!(b, b' ' | b'\t') && encoding != SourceEncoding::SingleByteBlank(b)
        })
        .map_or(0, |i| i + 1)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/trailing_whitespace_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/TrailingWhitespace")
            .collect()
    }

    #[test]
    fn pinned_code_and_blank_lines() {
        for (source, col) in [
            ("x = 0 \n", 6),
            ("  \n", 1),
            ("x = 0\t\n", 6),
            ("x = :a\u{3000}\n", 7),
            ("# comment \n", 10),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(
                (found[0].line, found[0].end_line, found[0].col),
                (1, 1, col)
            );
            assert_eq!(found[0].message, "Trailing whitespace detected.");
            assert_eq!(found[0].severity, crate::offense::Severity::Convention);
        }
    }

    #[test]
    fn pinned_removals() {
        for (before, after) in [
            ("x = 0 \n", "x = 0\n"),
            ("  \n", "\n"),
            ("x = 0\t\n", "x = 0\n"),
            ("# comment \n", "# comment\n"),
        ] {
            assert_correction(&TrailingWhitespace, before, after);
        }
    }

    #[test]
    fn literal_values_are_reported_but_never_trimmed() {
        for source in [
            "x = 'a  \nb' \n",
            "x = \"a  \nb\" \n",
            "x = /a  \nb/ \n",
            "x = %q(a  \nb) \n",
            "x = %Q(a  \nb) \n",
            "x = %r(a  \nb) \n",
            "x = %w(a\\  \nb) \n",
            "x = %W(a\\  \nb) \n",
            "x = %i(a\\  \nb) \n",
            "x = %I(a\\  \nb) \n",
            "x = %x(a  \nb) \n",
            "x = :\"a  \nb\" \n",
        ] {
            assert_eq!(offenses(source).len(), 2, "{source:?}");
            assert_correction(
                &TrailingWhitespace,
                source,
                &format!("{}\n", source.trim_end()),
            );
        }
    }

    #[test]
    fn whitespace_percent_delimiters_are_never_trimmed() {
        for delimiter in [" ", "\t"] {
            for kind in ["q", "Q", "r", "s", "x", ""] {
                for content in ["a", "a#{1}"] {
                    let literal = format!("x = %{kind}{delimiter}\n{content}{delimiter}\n");
                    let before = format!("{literal}puts :ok \n");
                    let after = format!("{literal}puts :ok\n");
                    assert_eq!(offenses(&before).len(), 3, "{before:?}");
                    assert_correction(&TrailingWhitespace, &before, &after);
                }
            }
            let source = format!("if %r{delimiter}\na{delimiter}\n  puts :ok\nend\n");
            assert_correction(&TrailingWhitespace, &source, &source);
        }
    }

    #[test]
    fn all_heredoc_forms_retain_body_bytes() {
        for prefix in ["<<", "<<-", "<<~"] {
            for quote in ["", "'", "\"", "`"] {
                let before = format!("x = {prefix}{quote}H{quote} \n  Hi   \n \t\nH\nx = 0\t\n");
                let after = format!("x = {prefix}{quote}H{quote}\n  Hi   \n \t\nH\nx = 0\n");
                let found = offenses(&before);
                assert_eq!(
                    found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                    [
                        (1, (before.lines().next().unwrap().len()) as u32),
                        (2, 5),
                        (3, 1),
                        (5, 6)
                    ]
                );
                assert_correction(&TrailingWhitespace, &before, &after);
            }
        }
    }

    #[test]
    fn interpolation_code_is_fixable_but_literal_text_is_not() {
        for (before, after) in [
            (
                "x = \"text  \n#{  \n  1  \n}tail  \n\" \n",
                "x = \"text  \n#{\n  1\n}tail  \n\"\n",
            ),
            (
                "x = <<~H \n  text  \n  #{  \n    \"nested  \ntext\"  \n  }tail  \nH\n",
                "x = <<~H\n  text  \n  #{\n    \"nested  \ntext\"\n  }tail  \nH\n",
            ),
        ] {
            assert_correction(&TrailingWhitespace, before, after);
        }
    }

    #[test]
    fn unicode_blanks_in_tokens_are_not_safe_deletions() {
        for source in [
            "x = :a\u{3000}\n",
            "identifier\u{a0}\n",
            "x = :a\u{3000} \n",
        ] {
            assert_eq!(offenses(source).len(), 1);
            assert_correction(&TrailingWhitespace, source, source);
        }
        assert_correction(&TrailingWhitespace, "# comment\u{3000}\n", "# comment\n");
    }

    #[test]
    fn preserves_end_data_and_handles_documentation_lookalikes() {
        for (before, after, positions) in [
            (
                "x = 0\t\n \n__END__\nx = 0\t\n",
                "x = 0\n\n__END__\nx = 0\t\n",
                vec![(1, 6), (2, 1)],
            ),
            (
                "x = 0\t\n=begin\n__END__\n=end\nx = 0\t\n",
                "x = 0\n=begin\n__END__\n=end\nx = 0\n",
                vec![(1, 6), (5, 6)],
            ),
            (
                "x1 = <<H \n__END__\nx2 = 0\t\nH\nx3 = 0\t\n",
                "x1 = <<H\n__END__\nx2 = 0\t\nH\nx3 = 0\n",
                vec![(1, 9), (3, 7), (5, 7)],
            ),
            (
                "x1 = <<H \n=begin  \n__END__\n=end\nx2 = 0\t\nH\nx3 = 0\t\n",
                "x1 = <<H\n=begin  \n__END__\n=end\nx2 = 0\t\nH\nx3 = 0\n",
                vec![(1, 9), (2, 7), (5, 7), (7, 7)],
            ),
        ] {
            assert_eq!(
                offenses(before)
                    .iter()
                    .map(|o| (o.line, o.col))
                    .collect::<Vec<_>>(),
                positions
            );
            assert_correction(&TrailingWhitespace, before, after);
        }
        assert!(offenses("__END__\n \t\n").is_empty());
    }

    #[test]
    fn encoding_line_endings_and_no_final_newline() {
        for (before, after, positions) in [
            (
                "\u{feff}é = 1 \t\r\n# 日本語 \n \t\r\nlast \t",
                "\u{feff}é = 1\r\n# 日本語\n\r\nlast",
                vec![(1, 6), (2, 6), (3, 1), (4, 5)],
            ),
            ("\u{feff} \t", "\u{feff}", vec![(1, 1)]),
        ] {
            assert_eq!(
                offenses(before)
                    .iter()
                    .map(|o| (o.line, o.col))
                    .collect::<Vec<_>>(),
                positions
            );
            assert_correction(&TrailingWhitespace, before, after);
        }
    }

    #[test]
    fn pinned_blank_character_set() {
        for c in [
            '\t', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}',
            '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}',
            '\u{202f}', '\u{205f}', '\u{3000}',
        ] {
            let source = format!("# é{c}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{c:?}");
            assert_eq!((found[0].line, found[0].col), (1, 4));
            assert_correction(&TrailingWhitespace, &source, "# é\n");
        }
        for c in [
            '\u{b}', '\u{c}', '\u{85}', '\u{200b}', '\u{2028}', '\u{2029}', '\u{feff}',
        ] {
            assert!(offenses(&format!("# comment{c}\n")).is_empty(), "{c:?}");
        }
    }

    #[test]
    fn encoded_blanks_keep_original_byte_offsets() {
        for (encoding, prefix, suffix, blank) in [
            (
                "ASCII-8BIT",
                b"# comment".as_slice(),
                b"\xe3\x80\x80".as_slice(),
                false,
            ),
            (
                "ISO-8859-1",
                b"# comment".as_slice(),
                b"\xa0".as_slice(),
                true,
            ),
            ("CP1252", b"# comment".as_slice(), b"\xa0".as_slice(), true),
            ("CP1251", b"# comment".as_slice(), b"\xa0".as_slice(), false),
            ("KOI8-R", b"# comment".as_slice(), b"\x9a".as_slice(), true),
            (
                "Shift_JIS",
                b"# comment".as_slice(),
                b"\x81\x40".as_slice(),
                false,
            ),
            (
                "UTF8-DoCoMo",
                b"# comment".as_slice(),
                b"\xe3\x80\x80".as_slice(),
                true,
            ),
            (
                "CESU-8",
                b"# \xed\xa0\xbd\xed\xb8\x80".as_slice(),
                b"\xc2\xa0".as_slice(),
                true,
            ),
        ] {
            let source = [
                format!("# coding: {encoding}\n").as_bytes(),
                prefix,
                suffix,
                b"\n",
            ]
            .concat();
            let file = SourceFile::new("test.rb".into(), source);
            let parsed = ruby_prism::parse(&file.bytes);
            assert_eq!(parsed.errors().count(), 0, "{encoding}");
            file.record_encoding(&parsed);
            assert_eq!(
                trailing_start(file.line_text(2), file.encoding()),
                prefix.len() + if blank { 0 } else { suffix.len() },
                "{encoding}"
            );
        }
    }

    #[test]
    fn both_directive_prefixes_suppress_detection_and_fixes() {
        for prefix in ["rustocop", "rubocop"] {
            for target in [
                "Layout/TrailingWhitespace",
                "TrailingWhitespace",
                "Layout",
                "all",
            ] {
                let before = format!(
                    "# {prefix}:disable {target} \nx = 0 \n# {prefix}:enable {target}\ny = 0 # {prefix}:disable {target} \n# {prefix}:disable-next {target}\nx = 0 \nz = 0 \n"
                );
                let after = before.strip_suffix(" \n").unwrap().to_owned() + "\n";
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col), (7, 6));
                assert_correction(&TrailingWhitespace, &before, &after);
            }
        }
    }

    #[test]
    fn multiple_and_nested_heredocs_keep_their_values() {
        for (before, after) in [
            (
                "x = [<<~A, <<~B] \n  a  \nA\n  b  \nB\n",
                "x = [<<~A, <<~B]\n  a  \nA\n  b  \nB\n",
            ),
            (
                "x = <<~A \n  #{<<~B}  \n    b  \n  B\n  a  \nA\n",
                "x = <<~A\n  #{<<~B}  \n    b  \n  B\n  a  \nA\n",
            ),
        ] {
            assert_correction(&TrailingWhitespace, before, after);
        }
    }

    #[test]
    fn never_creates_a_data_marker() {
        for source in [
            "__END__ \nputs :ok\n",
            "\u{feff}__END__\t\r\nputs :ok\r\n",
            "__END__ \t",
        ] {
            assert_eq!(offenses(source).len(), 1);
            assert_correction(&TrailingWhitespace, source, source);
        }
        // An indented identifier is not a data marker.
        assert_correction(&TrailingWhitespace, "  __END__ \n", "  __END__\n");
    }

    #[test]
    fn clean_lookalikes() {
        for source in [
            "",
            "x = 0",
            "x = 0\n",
            "# comment\n",
            "\n",
            "x = ' ' \n".trim_end(),
            "x = 0\r\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }
}
