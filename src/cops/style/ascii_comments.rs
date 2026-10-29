//! `Style/AsciiComments` — opt-in ASCII-only comments, allowing ©.
use crate::{cops::Cop, offense::Offense, source::SourceFile};

pub struct AsciiComments;

impl Cop for AsciiComments {
    fn name(&self) -> &'static str {
        "Style/AsciiComments"
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        for range in file.comments() {
            let text = &file.bytes[range.clone()];
            if text.is_ascii() {
                continue;
            }
            // Compare whole characters, not UTF-8-looking bytes inside a
            // declared legacy encoding. Like Ruby, © in a different source
            // encoding is not equal to the UTF-8 fixed AllowedChars entry.
            let mut offsets = file.character_offsets(range.clone()).peekable();
            let mut first = None;
            let mut disallowed = false;
            while let Some(start) = offsets.next() {
                let end = offsets.peek().copied().unwrap_or(range.end);
                let character = &file.bytes[start..end];
                if !character.is_ascii() {
                    first.get_or_insert(start);
                    disallowed |= !file.is_utf8() || character != "©".as_bytes();
                }
            }
            if !disallowed {
                continue;
            }
            let start = first.expect("a disallowed character exists");
            let end = file.bytes[start..range.end]
                .iter()
                .position(u8::is_ascii)
                .map_or(range.end, |offset| start + offset);
            let (line, col) = file.line_col(start);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: "Use only ascii symbols in comments.".into(),
                line,
                end_line: file.line_of(end.saturating_sub(1)),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/ascii_comments_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::{lint_str, lint_str_with};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&AsciiComments], source)
    }

    #[test]
    fn pinned_non_ascii_positions_and_message() {
        for (source, line, col) in [
            ("# 这是什么？", 1, 3),
            ("# foo ∂ bar", 1, 7),
            ("x = 1 # © then 日本語", 1, 9),
            ("=begin\n日本語\n=end\n", 2, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
            assert_eq!(found[0].message, "Use only ascii symbols in comments.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn allows_copyright_ascii_and_non_comment_lookalikes() {
        for source in [
            "# AZaz1@$%~,;*_`|",
            "# © copyright",
            "# ©©",
            "\"# 日本語\"",
            "x = <<~H\n# 日本語\nH\n",
            "__END__\n# 日本語\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert!(lint_str("# 日本語\n").is_empty(), "opt-in by default");
    }

    #[test]
    fn copyright_allowance_obeys_effective_encoding_not_just_character_widths() {
        for encoding in ["ASCII-8BIT", "CESU-8", "UTF8-MAC", "ISO-8859-1"] {
            let source = format!("# coding: {encoding}\n# ©\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{encoding}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
        }
        for source in [
            "# coding: CP65001\n# ©\n",
            "\u{feff}# coding: ASCII-8BIT\n# ©\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn reports_once_per_comment_and_honors_directives() {
        assert_eq!(offenses("# é é\n# ∂\n").len(), 2);
        for prefix in ["rubocop", "rustocop"] {
            let source = format!("# {prefix}:disable Style/AsciiComments\n# 日本語\n");
            assert!(offenses(&source).is_empty());
        }
    }
}
