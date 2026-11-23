//! Consumer-local RuboCop header facts; no change to the Style grammar.
use crate::{cops::style::magic_comment::MagicComment, source::SourceFile};
use std::sync::LazyLock;

pub(crate) struct HeaderMagic {
    pub valid: bool,
    pub encoding: Option<Vec<u8>>,
    pub frozen: bool,
}

/// Whole buffer lines and actual comment ranges have different validity rules:
/// encoding_specified? may be true even when valid? is false (leading spaces,
/// embedded-document prose). Keep those facts separate, as the callers do.
pub(crate) fn parse(file: &SourceFile, text: &[u8]) -> HeaderMagic {
    let projected = file.match_pattern_source(text);
    let starts_hash = projected.starts_with(b"#");
    let emacs = memchr::memmem::find(&projected, b"-*-").is_some_and(|start| {
        memchr::memmem::rfind(&projected[start + 3..], b"-*-").is_some_and(|end| end > 0)
    });
    static VIM: LazyLock<regex::bytes::Regex> =
        LazyLock::new(|| regex::bytes::Regex::new(r"(?-u:#\s*vim:\s*.+)").unwrap());
    let vim = !emacs && VIM.is_match(&projected);
    let key_end = if emacs || vim {
        0
    } else {
        projected.iter().position(|&b| b == b':').unwrap_or(0)
    };
    let mut normalized = Vec::new();
    if file.match_pattern_binary() {
        normalized.extend(
            projected
                .iter()
                .map(|&b| if b.is_ascii() { b } else { 0xff }),
        );
    } else if let Ok(unicode) = std::str::from_utf8(&projected) {
        for (i, c) in unicode.char_indices() {
            match c {
                // Ruby's simple-key regexp folds these, not editor keys or
                // values. Editor value downcasing happens after parsing.
                'ſ' if i < key_end => normalized.push(b's'),
                'ﬅ' | 'ﬆ' if i < key_end => normalized.extend_from_slice(b"st"),
                c if c.is_ascii() => normalized.push(c as u8),
                c if crate::source::projected_alphanumeric(c) => {
                    normalized.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
                _ => normalized.push(0xff),
            }
        }
    } else {
        normalized.extend_from_slice(&projected);
    }
    let start = normalized
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'..=b'\r'))
        .unwrap_or(normalized.len());
    let text = &normalized[start..];
    let text = if vim {
        &text[VIM.find(text).unwrap().start()..]
    } else {
        text
    };
    let magic = MagicComment::parse(text);
    HeaderMagic {
        valid: starts_hash && magic.valid,
        encoding: magic.encoding.map(|value| {
            // Upstream EditorComment#match downcases values before
            // Encoding.find; SimpleComment#encoding preserves their case.
            if (emacs || vim)
                && let Ok(value) = std::str::from_utf8(value)
            {
                value.to_lowercase().into_bytes()
            } else {
                value.to_vec()
            }
        }),
        frozen: magic.frozen_strings().is_some(),
    }
}

/// Main-parse lexical prefix: one scan of sorted Prism comments, including
/// embedded documents and escaped newlines. EOF controls have no code token.
pub(crate) fn first_code_offset(file: &SourceFile) -> Option<usize> {
    let mut offset = usize::from(file.bytes.starts_with(b"\xef\xbb\xbf")) * 3;
    let mut comments = file.comments().iter().peekable();
    while offset < file.bytes.len() {
        if let Some(comment) = comments.peek()
            && comment.start == offset
        {
            offset = comment.end;
            comments.next();
            continue;
        }
        match file.bytes[offset] {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => offset += 1,
            0 | 4 | 0x1a => return None,
            b'\\' if file.bytes.get(offset + 1) == Some(&b'\n') => offset += 2,
            b'\\' if file.bytes.get(offset + 1..offset + 3) == Some(b"\r\n") => offset += 3,
            _ => return Some(offset),
        }
    }
    None
}
pub(super) fn leading_lines(file: &SourceFile) -> u32 {
    first_code_offset(file).map_or(file.line_count() as u32, |offset| {
        file.line_of(offset).saturating_sub(1)
    })
}

// Finite Encoding.find names/aliases from the pinned Ruby reference. Special
// names are recognized even when Encoding.find("internal") returns nil. No
// locale/provider/filesystem lookup is performed by the linter.
pub(super) fn known_encoding(name: &[u8]) -> bool {
    const NAMES: &str = include_str!("magic_encoding_names.txt");
    NAMES
        .split_ascii_whitespace()
        .any(|candidate| candidate.as_bytes().eq_ignore_ascii_case(name))
}

/// The encoding a name stands for, as `Encoding.find` resolves Ruby 3.4's
/// aliases (`binary` is `ASCII-8BIT`), compared case-insensitively. The
/// locale-dependent `locale`, `external` and `filesystem` are left as names.
pub(super) fn canonical_encoding(name: &[u8]) -> Vec<u8> {
    const ALIASES: &str = include_str!("magic_encoding_aliases.txt");
    ALIASES
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(alias, _)| alias.as_bytes().eq_ignore_ascii_case(name))
        .map_or(name, |(_, canonical)| canonical.as_bytes())
        .to_ascii_uppercase()
}

#[cfg(test)]
pub(super) mod tests {
    use crate::{cops::Cop, offense::Offense, source::SourceFile};
    pub fn check_rows(cop: &dyn Cop) {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/magic_comment_positions.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 1040);
        for row in rows {
            let source = row["source"]
                .as_str()
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| crate::fixture::value(&row["bytes"]));
            let file = SourceFile::new("example.rb".into(), source.clone());
            let result = crate::parser::parse(&file.bytes);
            let mut found = vec![];
            crate::cops::run_cops(&[cop], &result, &file, &mut found);
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row[cop.name()]["findings"].clone()).unwrap();
            let facts = |found: &[Offense]| {
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(facts(&found), expected, "{row}");
            if let Some(source) = row["source"].as_str() {
                let pipeline: Vec<_> = crate::pipeline::lint_str(source)
                    .into_iter()
                    .filter(|o| o.cop == cop.name())
                    .collect();
                assert_eq!(facts(&pipeline), expected, "{row}");
            }
        }
    }
}
