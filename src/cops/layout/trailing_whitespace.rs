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
        let heredocs = if cx.corrections_requested() {
            heredocs(node)
        } else {
            Vec::new()
        };
        for line in 1..=file.code_line_count() as u32 {
            let text = file.line_text(line);
            let start = trailing_start(text, file.encoding());
            if start == text.len() {
                continue;
            }
            let range = file.line_start(line) + start..file.line_start(line) + text.len();
            let (_, col) = file.line_col(range.start);
            out.push(Offense {
                correction: cx
                    .corrections_requested()
                    .then(|| correction(file, line, range, &heredocs))
                    .flatten(),
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

/// A heredoc's body (from its first byte to the terminator) and whether it
/// is static (`<<~'X'`).
struct Heredoc {
    body: std::ops::Range<usize>,
    lines: std::ops::Range<u32>,
    stat: bool,
}

fn heredocs(root: &Node<'_>) -> Vec<(std::ops::Range<usize>, bool)> {
    struct Found(Vec<(std::ops::Range<usize>, bool)>);
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Found {
        fn enter(&mut self, n: Node<'pr>) {
            let first = |parts: ruby_prism::NodeList<'pr>| {
                parts.iter().next().map(|p| p.location().start_offset())
            };
            let (opening, body, closing) = match &n {
                Node::StringNode { .. } => {
                    let s = n.as_string_node().unwrap();
                    (
                        s.opening_loc(),
                        Some(s.content_loc().start_offset()),
                        s.closing_loc(),
                    )
                }
                Node::InterpolatedStringNode { .. } => {
                    let s = n.as_interpolated_string_node().unwrap();
                    (s.opening_loc(), first(s.parts()), s.closing_loc())
                }
                Node::XStringNode { .. } => {
                    let s = n.as_x_string_node().unwrap();
                    (
                        Some(s.opening_loc()),
                        Some(s.content_loc().start_offset()),
                        Some(s.closing_loc()),
                    )
                }
                Node::InterpolatedXStringNode { .. } => {
                    let s = n.as_interpolated_x_string_node().unwrap();
                    (
                        Some(s.opening_loc()),
                        first(s.parts()),
                        Some(s.closing_loc()),
                    )
                }
                _ => return,
            };
            if let (Some(opening), Some(closing)) = (opening, closing)
                && opening.as_slice().starts_with(b"<<")
            {
                let end = closing.start_offset();
                let body = body.unwrap_or(end).min(end);
                self.0.push((body..end, opening.as_slice().ends_with(b"'")));
            }
        }
    }
    let mut found = Found(Vec::new());
    crate::cops::walk::walk(&mut found, root);
    found.0
}

/// RuboCop's corrector: remove the blanks, except inside a heredoc body.
fn correction(
    file: &SourceFile,
    line: u32,
    range: std::ops::Range<usize>,
    heredocs: &[(std::ops::Range<usize>, bool)],
) -> Option<Correction> {
    let remove = |range| {
        Some(Correction {
            range,
            replacement: String::new(),
        })
    };
    let heredoc = heredocs
        .iter()
        .map(|(body, stat)| Heredoc {
            body: body.clone(),
            lines: file.line_of(body.start)..file.line_of(body.end),
            stat: *stat,
        })
        .find(|h| h.lines.contains(&line));
    let Some(heredoc) = heredoc else {
        return remove(range);
    };
    let body = std::str::from_utf8(&file.bytes[heredoc.body.clone()]).ok()?;
    // RuboCop's `indent_level`.
    let level = body
        .split_inclusive('\n')
        .filter_map(|l| {
            let indent = l.len() - l.trim_start().len();
            (indent < l.len()).then(|| l[..indent].chars().count())
        })
        .min()
        .unwrap_or(0);
    let line_start = file.line_start(line);
    let whitespace_only = range.start == line_start
        && line_start > 0
        && file.bytes[line_start - 1] == b'\n'
        && file.bytes.get(range.end) == Some(&b'\n');
    let blanks = std::str::from_utf8(&file.bytes[range.clone()]).ok()?;
    if whitespace_only && blanks.chars().count() <= level {
        return remove(range);
    }
    if heredoc.stat {
        return None;
    }
    let start = if whitespace_only {
        range.start
            + blanks
                .chars()
                .take(level)
                .map(char::len_utf8)
                .sum::<usize>()
    } else {
        range.start
    };
    Some(Correction {
        range: start..range.end,
        replacement: format!(
            "#{{'{}'}}",
            std::str::from_utf8(&file.bytes[start..range.end]).ok()?
        ),
    })
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
