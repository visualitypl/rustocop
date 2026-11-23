//! `Lint/DuplicateMagicComment` — repeated encoding/frozen header keys.
use crate::{
    cops::{Cop, NodeContext, style::magic_comment::MagicComment},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct DuplicateMagicComment;
impl Cop for DuplicateMagicComment {
    fn name(&self) -> &'static str {
        "Lint/DuplicateMagicComment"
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
        let correctable = cx.corrections_requested();
        let mut seen = [false; 2];
        for line in 1..=leading_lines(file) {
            let projected = file.match_pattern_source(file.line_text(line));
            // ASCII-8BIT's POSIX alnum is ASCII-only. Keep token boundaries
            // without letting a coincidental UTF-8 byte sequence become a value.
            let binary;
            let projected = if file.match_pattern_binary() && !projected.is_ascii() {
                binary = projected
                    .iter()
                    .map(|&b| if b.is_ascii() { b } else { 0xff })
                    .collect::<Vec<_>>();
                binary.as_slice()
            } else {
                projected.as_ref()
            };
            let Some(group) = magic_group(projected) else {
                continue;
            };
            if !std::mem::replace(&mut seen[group], true) {
                continue;
            }
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col: 1,
                end_line: line,
                message: "Duplicate magic comment detected.".into(),
                correction: correctable.then(|| crate::offense::Correction {
                    range: file.line_start(line)..file.line_start(line + 1),
                    replacement: String::new(),
                }),
            });
        }
    }
}

fn magic_group(text: &[u8]) -> Option<usize> {
    // FrozenStringLiteral passes whole source lines, not just comment
    // locations. SimpleComment permits leading ASCII regexp whitespace;
    // don't strip BOM, NUL, Unicode blanks, or arbitrary comment text.
    let start = text
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'..=b'\r'))
        .unwrap_or(text.len());
    let text = &text[start..];
    // Reuse the existing grammar, but supply its token regex ASCII surrogates
    // for pinned POSIX alnum (Alphabetic OR Decimal_Number, NOT all Number).
    // Use a digit, absent from every grammar key, so a Unicode value remains
    // a token without making an unrelated Unicode key match an ASCII keyword.
    // The grammar's ASCII case folding also needs Ruby's two simple-key folds:
    // long s and the st ligatures. Editor keys remain strictly case-sensitive.
    let emacs = memchr::memmem::find(text, b"-*-").is_some_and(|start| {
        memchr::memmem::rfind(&text[start + 3..], b"-*-").is_some_and(|end| end > 0)
    });
    let mut normalized = Vec::new();
    let text = if !text.is_ascii()
        && let Ok(unicode) = std::str::from_utf8(text)
    {
        for c in unicode.chars() {
            match c {
                'ſ' if !emacs => normalized.push(b's'),
                'ﬅ' | 'ﬆ' if !emacs => normalized.extend_from_slice(b"st"),
                c if c.is_ascii() => normalized.push(c as u8),
                c if crate::source::projected_alphanumeric(c) => normalized.push(b'0'),
                _ => normalized.push(0xff),
            }
        }
        normalized.as_slice()
    } else {
        text
    };
    // Unlike the Style comment-location callers, leading_comment_lines can
    // include embedded-document body text. Vim's selector is unanchored too;
    // only SimpleComment requires the leading '#'. Emacs still wins first.
    let text = if !emacs && !text.starts_with(b"#") {
        static VIM: std::sync::LazyLock<regex::bytes::Regex> =
            std::sync::LazyLock::new(|| regex::bytes::Regex::new(r"(?-u:#\s*vim:\s*.+)").unwrap());
        &text[VIM.find(text)?.start()..]
    } else {
        text
    };
    let magic = MagicComment::parse(text);
    if magic.encoding.is_some() {
        Some(0) // A mixed Emacs pragma belongs ONLY to the encoding group.
    } else if magic.frozen_strings().is_some() {
        Some(1)
    } else {
        None
    }
}

/// FrozenStringLiteral#leading_comment_lines excludes the entire first code
/// token's physical line. Scan the main parse's sorted comment ranges once;
/// embedded documents and escaped newlines stay transparent, literal text not.
fn leading_lines(file: &SourceFile) -> u32 {
    super::magic_comment_header::leading_lines(file)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/duplicate_magic_comment_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == DuplicateMagicComment.name())
            .collect()
    }
    #[test]
    fn all_pinned_source_shared_lifecycle_and_byte_rows() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_magic_comment.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 342);
        for row in rows {
            let found = offenses(row["source"].as_str().unwrap());
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
        }
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_magic_bytes.json"
        ))
        .unwrap();
        for row in rows {
            let source: Vec<u8> = crate::fixture::value(&row["bytes"]);
            let file = SourceFile::new(std::path::PathBuf::from("example.rb"), source);
            let result = crate::parser::parse(&file.bytes);
            let mut found = vec![];
            crate::cops::run_cops(&[&DuplicateMagicComment], &result, &file, &mut found);
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
        }
    }

    #[test]
    fn every_shared_magic_comment_grammar_example() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_magic_shared.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 54);
        for row in rows {
            assert_eq!(
                magic_group(row["text"].as_str().unwrap().as_bytes()),
                row["group"].as_u64().map(|g| g as usize),
                "{row}"
            );
        }
    }

    #[test]
    fn unicode_alphanumeric_values_do_not_create_magic_keys() {
        for key in [
            "frozen_string_literél",
            "frozen_string_literあl",
            "frozen_string_liter١l",
            "frozen_string_literﬅl",
        ] {
            for text in [format!("# {key}: true"), format!("# -*- {key}: true -*-")] {
                let source = format!("{text}\n{text}\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        // Genuine simple-key folds and Unicode value tokens still count.
        for text in [
            "# frozen_ſtring_literal: true",
            "# frozen_ﬅring_literal: true",
            "# frozen_ﬆring_literal: true",
            "# frozen_string_literal: é",
            "# -*- frozen_string_literal: あ -*-",
            "# vim: filetype=ruby, fileencoding=é",
        ] {
            let source = format!("# ordinary\n{text}\n{text}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1), "{source}");
            assert_eq!(found[0].message, "Duplicate magic comment detected.");
        }
    }

    #[test]
    fn pinned_main_specs_detect_keys_not_values() {
        for (source, expected) in [
            (
                "# frozen_string_literal: true\n# frozen_string_literal: true\n",
                vec![2],
            ),
            (
                "# frozen_string_literal: true\n# frozen_string_literal: TRUE\n",
                vec![2],
            ),
            ("# encoding: ascii\n# encoding: ascii\n", vec![2]),
            ("# encoding: ascii\n# encoding: utf-8\n", vec![2]),
            (
                "# encoding: ascii\n# frozen_string_literal: true\n# encoding: ascii\n# frozen_string_literal: true\n",
                vec![3, 4],
            ),
            ("# frozen_string_literal: true\n", vec![]),
            ("# encoding: ascii\n", vec![]),
            ("# encoding: ascii\n# frozen_string_literal: true\n", vec![]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| o.line).collect::<Vec<_>>(),
                expected,
                "{source}"
            );
            for o in found {
                assert_eq!((o.col, o.end_line), (1, o.line));
                assert_eq!(o.message, "Duplicate magic comment detected.");
            }
        }
    }
}
