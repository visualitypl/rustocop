//! `Style/CommentedKeyword` — detection only: moving/deleting comments is unsafe.
use std::sync::LazyLock;

use regex::bytes::Regex;

use crate::{cops::Cop, offense::Offense, source::SourceFile};

// Ruby's \s and \w are ASCII here. Use byte patterns so legacy comment
// encodings need no transcoding. Directive boundaries instead use Ruby's
// Unicode word set: Alphabetic, Mark, Number and Connector_Punctuation (not
// Join_Control as in Rust's \w). All scans are linear in source size.
static ALLOWED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?-u:#\s*(?::nodoc:|:yields:))|",
        r"(?-u:#\s*(?:rubocop|rustocop)\s*:\s*",
        r"(?:disable|enable|todo|push|pop|disable-next|todo-next|enable-next|next))",
        r"(?:[^\p{Alphabetic}\p{M}\p{N}\p{Pc}]|$)"
    ))
    .expect("fixed allowed comments")
});
static SUBCLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:^\s*class\s+[\w:]+\s*<\s*[\w:]+)").expect("fixed subclass prefix")
});
static GENERICS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:^#\[.+\])").expect("fixed RBS annotation"));
static STEEP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:#\ssteep:ignore(?:\s|$))").expect("fixed Steep annotation"));

pub struct CommentedKeyword;

impl Cop for CommentedKeyword {
    fn name(&self) -> &'static str {
        "Style/CommentedKeyword"
    }

    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        for comment in file.comments() {
            let line = file.line_of(comment.start);
            let source = file.line_text(line);
            let code = &source[source
                .iter()
                .take_while(|&&b| super::is_ruby_whitespace(b))
                .count()..];
            let Some(end) = code.iter().position(|&b| super::is_ruby_whitespace(b)) else {
                continue;
            };
            let keyword = &code[..end];
            if !matches!(keyword, b"begin" | b"class" | b"def" | b"end" | b"module") {
                continue;
            }
            let text = &file.bytes[comment.clone()];
            let rbs = if SUBCLASS.is_match(source) {
                GENERICS.is_match(text)
            } else {
                matches!(keyword, b"def" | b"end") && text.starts_with(b"#:")
            };
            if rbs || STEEP.is_match(text) || ALLOWED.is_match(source) {
                continue;
            }
            let (line, col) = file.line_col(comment.start);
            let keyword = std::str::from_utf8(keyword).expect("ASCII keyword");
            // The comment (with the blanks around it) goes; but for `end`, it
            // moves to a line of its own before the keyword's.
            let correction = {
                let bytes = &file.bytes;
                let mut start = comment.start;
                while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
                    start -= 1;
                }
                let end = crate::offense::space_right_blank(bytes, comment.end);
                let mut edits = vec![(start..end, String::new())];
                if keyword != "end" {
                    let at = file.line_start(line);
                    edits.push((at..at, format!("{}\n", String::from_utf8_lossy(text))));
                }
                crate::offense::Correction::from_edits(bytes, edits)
            };
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Do not place comments on the same line as the `{keyword}` keyword."
                ),
                line,
                end_line: file.line_of(comment.end.saturating_sub(1)),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/commented_keyword_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/CommentedKeyword")
            .collect()
    }

    #[test]
    fn pinned_keyword_positions_messages_and_unsafe_corrections() {
        for (source, line, col, keyword) in [
            ("if x\n  y\nend # comment\n", 3, 5, "end"),
            ("begin # comment\n  y\nend\n", 1, 7, "begin"),
            ("class X # comment\nend\n", 1, 9, "class"),
            ("module X # comment\nend\n", 1, 10, "module"),
            ("def x # comment\nend\n", 1, 7, "def"),
            (
                "module X\n  class Y # comment\n  end\nend\n",
                2,
                11,
                "class",
            ),
            ("def x(a, b) # comment\nend\n", 1, 13, "def"),
            ("def x; end # comment\n", 1, 12, "def"),
            ("class << self # comment\nend\n", 1, 15, "class"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(
                found[0].message,
                format!("Do not place comments on the same line as the `{keyword}` keyword.")
            );
            let found = correction_offenses(&CommentedKeyword, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_some());
        }
    }

    #[test]
    fn comments_literals_and_keyword_lookalikes_are_not_trailing_keyword_comments() {
        for source in [
            "if x\n y\nend\n",
            "class X\nend\n",
            "begin\nend\n",
            "def x\nend\n",
            "module X\nend\n",
            "# module Y # trap comment\n",
            "'end' # comment\n",
            "x = <<-HEREDOC\n  def # not a comment\nHEREDOC\n",
            "def x(y = \"#value\")\n y\nend\n",
            "def x(y: \"#value\")\n y\nend\n",
            "options = { end_buttons: true } # comment\n",
            "defined?(SomeModule).should be_nil # comment\n",
            "foo = beginning_statement # comment\n",
            "begin; end # comment\n",
            "x = 1; begin # comment\nend\n",
            "=begin\ndef x # comment\n=end\n",
            "__END__\ndef x # comment\n",
            "x = %q{\ndef x # comment\n}\n",
            "x = /\nend # comment\n/\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_documentation_and_directive_exemptions() {
        for comment in [
            ":nodoc:",
            ":yields:",
            "rubocop:disable Metrics/MethodLength",
            "rubocop: disable Metrics/MethodLength",
            "rubocop :todo Metrics/MethodLength",
            "rubocop:  todo Metrics/MethodLength",
            "rubocop:enable all",
            "rubocop:push",
            "rustocop:disable Lint/Debugger",
        ] {
            let source = format!("def x # {comment}\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for comment in [
            ":no_doc:",
            "rubocop:disabled",
            "rubocop:disableé",
            "steep",
            "steep:ignoreMethodBodyTypeMismatch",
        ] {
            let source = format!("def x # {comment}\nend\n");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }

    #[test]
    fn ruby_vertical_tab_whitespace_before_and_after_keywords() {
        for (source, col) in [
            ("\x0bclass X # comment\nend\n", 10),
            ("class\x0bX # comment\nend\n", 9),
            (" \t\x0bclass\x0bX # comment\nend\n", 12),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(
                found[0].message,
                "Do not place comments on the same line as the `class` keyword."
            );
        }
        assert!(offenses("\x0bclass X < Y #[String]\nend\n").is_empty());
    }

    #[test]
    fn directive_boundaries_use_ruby_unicode_word_semantics() {
        for prefix in ["rubocop", "rustocop"] {
            for suffix in ["\u{200c}", "\u{200d}", "!", "😀", " "] {
                let source = format!("class X # {prefix}:pop{suffix}\nend\n");
                assert!(offenses(&source).is_empty(), "{source:?}");
            }
            for suffix in ["é", "\u{0301}", "²", "Ⅰ", "Ⓐ", "_", "a", "0"] {
                let source = format!("class X # {prefix}:pop{suffix}\nend\n");
                assert_eq!(offenses(&source).len(), 1, "{source:?}");
            }
        }
    }

    #[test]
    fn pinned_rbs_and_steep_annotations() {
        for namespace in [
            "X < Y",
            "A < B::C",
            "A < B::C::D",
            "A::B < C",
            "A::B::C < D",
        ] {
            assert!(offenses(&format!("class {namespace} #[String]\nend\n")).is_empty());
        }
        for source in [
            "def x #: String\nend\n",
            "def x\nend #: String\n",
            "class X\nend #: String\n",
            "module X\nend #: String\n",
            "begin\nend #: String\n",
            "each do\nend #: Array[Integer]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (open, keyword) in [
            ("def x", "def"),
            ("class X", "class"),
            ("module X", "module"),
            ("begin", "begin"),
        ] {
            for comment in [
                "# steep:ignore",
                "# steep:ignore SomeError",
                "#\tsteep:ignore",
            ] {
                assert!(offenses(&format!("{open} {comment}\nend {comment}\n")).is_empty());
            }
            for comment in ["# steep", "#steep:ignore", "# steep:ignoreError"] {
                let found = offenses(&format!("{open} {comment}\nend {comment}\n"));
                assert_eq!(found.len(), 2);
                assert!(found[0].message.contains(&format!("`{keyword}`")));
                assert!(found[1].message.contains("`end`"));
            }
        }
        for source in [
            "class X #[String]\nend\n",
            "class X #: String\nend\n",
            "module Y #: String\nend\n",
            "begin #: String\nend\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for comment in [
            "#[String",
            "#String]",
            "# String]",
            "#String ]",
            "#[]",
            "# [String]",
        ] {
            assert_eq!(
                offenses(&format!("class X < Y {comment}\nend\n")).len(),
                1,
                "{comment}"
            );
        }
    }

    #[test]
    fn many_long_comments_are_bounded_by_source_size() {
        let source = format!(
            "begin\n# {}\nend # {}\n",
            "a".repeat(100_000),
            "b".repeat(100_000)
        )
        .repeat(100);
        let file = SourceFile::new("bounded.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        file.record_comments(&parsed);
        let mut found = Vec::new();
        CommentedKeyword.check_source(&file, &mut found);
        assert_eq!(found.len(), 100);
    }
}
