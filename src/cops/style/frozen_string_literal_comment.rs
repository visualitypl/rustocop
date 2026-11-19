//! `Style/FrozenStringLiteralComment` — flags a file whose leading comments
//! have no `# frozen_string_literal: true` (or `false`) magic comment.
//!
//! Ships **off by default**: Ruby 3.4's chilled strings move
//! this to the runtime. Enable it in `rustocop.toml` to get RuboCop's default
//! `always` style: a file needs the comment (set to `true` or `false`) among
//! the comment lines before its first token. Files with no tokens at all are
//! accepted. As in RuboCop, the magic comment may be written Emacs-style
//! (`# -*- frozen_string_literal: true -*-`), is case-insensitive, and
//! accepts `-` for `_`; a value other than `true`/`false` does not count.
//! The offense is reported at the start of the file.

use crate::{cops::Cop, offense::Offense, source::SourceFile};

/// The `Style/FrozenStringLiteralComment` cop.
pub struct FrozenStringLiteralComment;

impl Cop for FrozenStringLiteralComment {
    fn name(&self) -> &'static str {
        "Style/FrozenStringLiteralComment"
    }

    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        let Some(leading_lines) = leading_comment_lines(file) else {
            return;
        };
        let has_comment = (1..=leading_lines).any(|line| {
            let text = file.line_text(line);
            let text = if line == 1 {
                text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text)
            } else {
                text
            };
            is_frozen_string_literal_comment(text)
        });
        if !has_comment {
            out.push(Offense {
                correction: Some(insert_comment(file)),
                cop: self.name(),
                severity: self.severity(),
                message: "Missing frozen string literal comment.".to_string(),
                line: 1,
                end_line: 1,
                col: 1,
            });
        }
    }
}

/// How many lines precede the line of the first token that is not a
/// comment (all of them when there is none). `None` for a file with no
/// tokens at all, comments included.
/// Upstream's `insert_comment`: on a line of its own after a leading shebang
/// and/or encoding comment, else at the top.
fn insert_comment(file: &SourceFile) -> crate::offense::Correction {
    use crate::cops::lint::magic_comment_header::{first_code_offset, parse};
    let bytes = &file.bytes;
    let comments = file.comments();
    let code = first_code_offset(file);
    let token = |index: usize| {
        comments
            .get(index)
            .filter(|c| code.is_none_or(|o| c.start < o))
    };
    let mut special = None;
    let mut next = 0;
    if let Some(first) = token(0)
        && bytes[first.clone()].starts_with(b"#!")
    {
        special = Some(first);
        next = 1;
    }
    if let Some(comment) = token(next)
        && parse(file, &bytes[comment.clone()]).encoding.is_some()
    {
        special = Some(comment);
    }
    match special {
        Some(comment) => {
            let line = file.line_of(comment.start);
            let mut end = if (line as usize) < file.line_count() {
                file.line_start(line + 1)
            } else {
                bytes.len()
            };
            if end > comment.start && bytes.get(end - 1) == Some(&b'\n') {
                end -= 1;
                if bytes.get(end.wrapping_sub(1)) == Some(&b'\r') {
                    end -= 1;
                }
            }
            crate::offense::Correction {
                range: end..end,
                replacement: "\n# frozen_string_literal: true".into(),
            }
        }
        None => {
            let at = if bytes.starts_with(b"\xEF\xBB\xBF") {
                3
            } else {
                0
            };
            crate::offense::Correction {
                range: at..at,
                replacement: "# frozen_string_literal: true\n".into(),
            }
        }
    }
}

fn leading_comment_lines(file: &SourceFile) -> Option<u32> {
    let bytes = &file.bytes;
    let comments = file.comments();
    let mut next_comment = 0;
    let mut offset = if bytes.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    while offset < bytes.len() {
        while comments
            .get(next_comment)
            .is_some_and(|comment| comment.end <= offset)
        {
            next_comment += 1;
        }
        if let Some(comment) = comments.get(next_comment)
            && comment.start <= offset
        {
            offset = comment.end;
            continue;
        }
        match bytes[offset] {
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c' => offset += 1,
            b'\\' if matches!(bytes.get(offset + 1), Some(b'\n')) => offset += 2,
            b'\\' if bytes.get(offset + 1..offset + 3) == Some(b"\r\n") => offset += 3,
            _ => {
                let line = file.line_of(offset);
                // `__END__` ends the code (and RuboCop's lines) without a token.
                let data = file.line_start(line) == offset && file.line_text(line) == b"__END__";
                if !data {
                    return Some(line - 1);
                }
                return (next_comment > 0).then_some(line - 1);
            }
        }
    }
    if comments.is_empty() {
        None
    } else {
        Some(file.line_count() as u32)
    }
}

/// Whether a line is a magic comment setting `frozen_string_literal` to
/// `true` or `false` (RuboCop's `MagicComment#valid_literal_value?`).
fn is_frozen_string_literal_comment(line: &[u8]) -> bool {
    let value = if let Some(tokens) = emacs_tokens(line) {
        tokens
            .split(|&byte| byte == b';')
            .find_map(|token| emacs_value(trim(token)))
    } else if is_vim_comment(line) {
        // Vim modelines cannot set `frozen_string_literal`.
        None
    } else {
        simple_value(line)
    };
    value.is_some_and(|value| {
        value.eq_ignore_ascii_case(b"true") || value.eq_ignore_ascii_case(b"false")
    })
}

/// The text between the first `-*-` and the last `-*-` of an Emacs-style
/// magic comment line.
fn emacs_tokens(line: &[u8]) -> Option<&[u8]> {
    const MARK: &[u8] = b"-*-";
    let start = memchr::memmem::find(line, MARK)? + MARK.len();
    let end = memchr::memmem::rfind(&line[start..], MARK)? + start;
    // `-*-(.+)-*-`: at least one byte between the marks.
    (end > start).then(|| &line[start..end])
}

/// The value of an Emacs-style `frozen_string_literal: value` token
/// (`\Afrozen[_-]string[_-]literal\s*:\s*TOKEN\z`, case-sensitive key).
fn emacs_value(token: &[u8]) -> Option<&[u8]> {
    let rest = strip_keyword(token, false)?;
    let rest = trim_start(rest).strip_prefix(b":")?;
    let value = trim_start(rest);
    is_token(value).then_some(value)
}

/// `#\s*vim:\s*.+` anywhere in the line.
fn is_vim_comment(line: &[u8]) -> bool {
    line.iter().enumerate().any(|(index, &byte)| {
        if byte != b'#' {
            return false;
        }
        let rest = trim_start(&line[index + 1..]);
        rest.strip_prefix(b"vim:")
            .is_some_and(|rest| !trim_start(rest).is_empty())
    })
}

/// The value of a plain `# frozen_string_literal: value` comment
/// (`\A\s*#\s*frozen[_-]string[_-]literal:\s*TOKEN\s*\z`, case-insensitive).
fn simple_value(line: &[u8]) -> Option<&[u8]> {
    let rest = trim_start(line).strip_prefix(b"#")?;
    let rest = strip_keyword(trim_start(rest), true)?;
    let value = trim(rest.strip_prefix(b":")?);
    is_token(value).then_some(value)
}

/// `rest` after a leading `frozen[_-]string[_-]literal`.
fn strip_keyword(text: &[u8], ignore_case: bool) -> Option<&[u8]> {
    const WORDS: [&[u8]; 3] = [b"frozen", b"string", b"literal"];
    let mut rest = text;
    for (index, word) in WORDS.iter().enumerate() {
        if index > 0 {
            rest = rest
                .strip_prefix(b"_")
                .or_else(|| rest.strip_prefix(b"-"))?;
        }
        let head = rest.get(..word.len())?;
        let matches = if ignore_case {
            head.eq_ignore_ascii_case(word)
        } else {
            head == *word
        };
        if !matches {
            return None;
        }
        rest = &rest[word.len()..];
    }
    Some(rest)
}

/// RuboCop's magic comment value token: `[[:alnum:]\-_]+`.
fn is_token(value: &[u8]) -> bool {
    !value.is_empty()
        && String::from_utf8_lossy(value)
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Ruby's `\s`: space, tab, line feed, carriage return, form feed, vertical tab.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x0b')
}

fn trim_start(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|&byte| !is_space(byte))
        .unwrap_or(text.len());
    &text[start..]
}

fn trim(text: &[u8]) -> &[u8] {
    let text = trim_start(text);
    let end = text
        .iter()
        .rposition(|&byte| !is_space(byte) && byte != 0)
        .map_or(0, |index| index + 1);
    &text[..end]
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/frozen_string_literal_comment_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the
    //! `always` style.

    use super::*;
    use crate::pipeline::{lint_str, lint_str_with};

    fn offenses(source: &str) -> Vec<(u32, u32)> {
        lint_str_with(&[&FrozenStringLiteralComment], source)
            .iter()
            .map(|o| (o.line, o.col))
            .collect()
    }

    fn assert_flags(source: &str) {
        assert_eq!(offenses(source), [(1, 1)], "should flag {source:?}");
    }

    fn assert_accepts(source: &str) {
        assert!(offenses(source).is_empty(), "should accept {source:?}");
    }

    #[test]
    fn is_off_by_default() {
        assert!(!FrozenStringLiteralComment.default_enabled());
        assert!(
            lint_str("puts 1\n")
                .iter()
                .all(|o| o.cop != "Style/FrozenStringLiteralComment")
        );
    }

    #[test]
    fn flags_missing_comment_with_message() {
        let found = lint_str_with(&[&FrozenStringLiteralComment], "puts 1\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(found[0].message, "Missing frozen string literal comment.");
        assert_eq!(found[0].cop, "Style/FrozenStringLiteralComment");
        assert!(!FrozenStringLiteralComment.safe_fixes());
        let requested =
            crate::cops::testing::correction_offenses(&FrozenStringLiteralComment, "puts 1\n");
        assert_eq!(requested.len(), 1);
        assert!(requested[0].correction.is_some());
    }

    #[test]
    fn accepts_sources_without_tokens() {
        assert_accepts("");
        assert_accepts(" ");
        assert_accepts("\n\n");
        assert_accepts("__END__\n");
        assert_accepts("\n__END__\nputs 1\n");
    }

    #[test]
    fn stops_at_the_data_section() {
        assert_flags("# c\n__END__\n# frozen_string_literal: true\n");
        assert_flags("x\n__END__\n");
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "# frozen_string_literal: true\nputs 1\n",
            "# frozen_string_literal: false\nputs 1\n",
            "#!/usr/bin/env ruby\n# frozen_string_literal: true\nputs 1\n",
            "#!/usr/bin/env ruby\n# frozen_string_literal: false\nputs 1\n",
            "# encoding: utf-8\n# frozen_string_literal: true\nputs 1\n",
            "# encoding: utf-8\n# frozen_string_literal: false\nputs 1\n",
            "#!/usr/bin/env ruby\n# encoding: utf-8\n# frozen_string_literal: true\nputs 1\n",
            "#!/usr/bin/env ruby\n\n# encoding: utf-8\n\n# frozen_string_literal: true\nputs 1\n",
            "#!/usr/bin/env ruby\n# encoding: utf-8\n# frozen_string_literal: false\nputs 1\n",
            "#!/usr/bin/env ruby\n# frozen_string_literal: true\n# encoding: utf-8\nputs 1\n",
            "#!/usr/bin/env ruby\n# frozen_string_literal: false\n# encoding: utf-8\nputs 1\n",
            "#!/usr/bin/env ruby\n# -*- encoding: UTF-8; frozen_string_literal: true -*-\n\
             # encoding: utf-8\nputs 1\n",
            "#!/usr/bin/env ruby\n# encoding: utf-8\n#\n# These are more comments\n\
             # frozen_string_literal: false\nputs 1\n",
        ] {
            assert_accepts(source);
        }
    }

    #[test]
    fn accepts_any_case_of_true_and_false() {
        for value in ["TRUE", "FALSE", "True", "False", "truE", "falsE"] {
            assert_accepts(&format!("# frozen_string_literal: {value}\nputs 1\n"));
        }
    }

    #[test]
    fn flags_rubocop_spec_examples() {
        for source in [
            "# frozen_string_literal: token\nputs 1\n",
            "puts 1\n",
            "#!/usr/bin/env ruby\nputs 1\n",
            "#!/usr/bin/env ruby\n",
            "# encoding: utf-8\nputs 1\n",
            "# encoding: BINARY\nputs 1\n",
            "# -*- coding: binary -*-\nputs 1\n",
            "# encoding: utf-8\n\nputs 1\n",
            "#!/usr/bin/env ruby\n# encoding: utf-8\nputs 1\n",
            "#!/usr/bin/env ruby\n# encoding: utf-8\n\nputs 1\n",
            "# encoding: utf-8\n\nputs 1\n# frozen_string_literal: true\n",
            "\nputs 1\n",
        ] {
            assert_flags(source);
        }
    }

    #[test]
    fn accepts_spelling_variants() {
        for source in [
            "# Frozen-String-Literal: true\nputs 1\n",
            "#frozen_string_literal:true\nputs 1\n",
            "  # frozen_string_literal: true  \nputs 1\n",
            "# frozen_string_literal: true\r\nputs 1\r\n",
            "# -*- frozen_string_literal: true -*-\nputs 1\n",
            "# -*- mode: ruby; frozen-string-literal : false -*-\nputs 1\n",
            "=begin\n=end\n# frozen_string_literal: true\nputs 1\n",
            "\u{feff}# frozen_string_literal: true\nputs 1\n",
            "# frozen_string_literal: true\n",
            "# frozen_string_literal: true\n__END__\n",
        ] {
            assert_accepts(source);
        }
    }

    #[test]
    fn flags_lookalikes() {
        for source in [
            // Comments after the first token do not count.
            "puts 1 # frozen_string_literal: true\n",
            "x = 1\n# frozen_string_literal: true\n",
            // Not a magic comment.
            "# frozen_string_literal: yes\nputs 1\n",
            "# frozen_string_literal: true, really\nputs 1\n",
            "# frozen_string_literal = true\nputs 1\n",
            "# frozen_string_literals: true\nputs 1\n",
            "# frozen_string_literal : true\nputs 1\n",
            "# -*- Frozen_String_Literal: true -*-\nputs 1\n",
            "# vim: ft=ruby # frozen_string_literal: true\nputs 1\n",
            "# the frozen_string_literal: true comment\nputs 1\n",
        ] {
            assert_flags(source);
        }
    }

    #[test]
    fn honors_directives() {
        assert!(
            lint_str_with(
                &[&FrozenStringLiteralComment],
                "# rubocop:disable Style/FrozenStringLiteralComment\nputs 1\n"
            )
            .is_empty()
        );
    }
}
