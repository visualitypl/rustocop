//! Fixed RuboCop magic-comment grammar shared by the header cops.
//! Byte matching keeps legacy source bytes intact. Editor keys are case-sensitive
//! for validity, while simple keys and the formatting range search are not.
use crate::source::SourceFile;
use regex::bytes::Regex;
use std::{ops::Range, sync::LazyLock};

const KEYS: &str = r"(?:en)?coding|frozen[_-]string[_-]literal|rbs_inline|warn[_-]indent|shareable[_-]constant[_-]value|typed";
static DIRECTIVES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("(?i-u:{KEYS})")).unwrap());
static SIMPLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i-u:^#\s*({KEYS}):\s*((?u:[\p{{Alphabetic}}\p{{N}}_-]+))\s*$)"
    ))
    .unwrap()
});
static ENCODING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i-u:^#\s*(?:frozen_string_literal:\s*(?:true|false))?\s*((?:en)?coding): ((?u:[\p{Alphabetic}\p{N}_-]+)))").unwrap()
});
static EDITOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?-u:^({KEYS})\s*:\s*((?u:[\p{{Alphabetic}}\p{{N}}_-]+))$)"
    ))
    .unwrap()
});
static VIM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u:#\s*vim:\s*(.+))").unwrap());
static VIM_ENCODING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:^fileencoding\s*=\s*((?u:[\p{Alphabetic}\p{N}_-]+))$)").unwrap()
});

pub(crate) struct MagicComment<'a> {
    pub encoding: Option<&'a [u8]>,
    pub valid: bool,
    kind: Kind<'a>,
    rewrite_safe: bool,
}
enum Kind<'a> {
    Simple(&'a [u8]),
    Emacs(Vec<&'a [u8]>),
    Vim(Vec<&'a [u8]>),
}

impl<'a> MagicComment<'a> {
    pub fn parse(text: &'a [u8]) -> Self {
        let mut comment = Self {
            encoding: None,
            valid: false,
            kind: Kind::Simple(text),
            rewrite_safe: true,
        };
        // Upstream's Emacs form is unanchored, including when the caller
        // supplies a whole source line. Simple forms still require a leading #.
        if let Some(start) = memchr::memmem::find(text, b"-*-")
            && let Some(end) = memchr::memmem::rfind(&text[start + 3..], b"-*-")
            && end > 0
        {
            let tokens: Vec<_> = text[start + 3..start + 3 + end]
                .split(|&b| b == b';')
                .map(trim)
                .collect();
            for token in &tokens {
                if let Some(parts) = EDITOR.captures(token) {
                    let key = parts.get(1).unwrap().as_bytes();
                    // Emacs supports only encoding and Ruby's three other keys.
                    if matches!(key, b"typed" | b"rbs_inline") {
                        continue;
                    }
                    comment.valid = true;
                    if matches!(key, b"encoding" | b"coding") && comment.encoding.is_none() {
                        comment.encoding = Some(parts.get(2).unwrap().as_bytes());
                    }
                }
            }
            // Ruby String#strip also strips NUL; rebuilding must not turn
            // an inert NUL-prefixed setting into an effective Ruby directive.
            // Detection uses the greedy upstream editor grammar, but Ruby can
            // stop at an interior closing delimiter. Never rebuild that case:
            // removing a token could activate settings beyond Ruby's delimiter.
            comment.rewrite_safe = !text.contains(&0)
                && memchr::memmem::find(&text[start + 3..start + 3 + end], b"-*-").is_none()
                && trim(&text[..start]) == b"#"
                && trim(&text[start + 3 + end + 3..]).is_empty();
            comment.kind = Kind::Emacs(tokens);
        } else if !text.starts_with(b"#") {
            return comment;
        } else if let Some(parts) = VIM.captures(text) {
            let body = parts.get(1).unwrap().as_bytes();
            // Vim's separator is the exact string ", ", not a comma alone.
            let tokens = split_vim(body);
            if tokens.len() > 1 {
                comment.encoding = tokens.iter().find_map(|token| {
                    VIM_ENCODING
                        .captures(token)
                        .map(|p| p.get(1).unwrap().as_bytes())
                });
            }
            comment.valid = comment.encoding.is_some();
            comment.rewrite_safe = !text.contains(&0);
            comment.kind = Kind::Vim(tokens);
        } else {
            if let Some(parts) = ENCODING.captures(text) {
                comment.encoding = Some(parts.get(2).unwrap().as_bytes());
            }
            comment.valid = comment.encoding.is_some()
                || SIMPLE.captures(text).is_some_and(|parts| {
                    let key = parts.get(1).unwrap().as_bytes();
                    !key.eq_ignore_ascii_case(b"encoding")
                        && !key.eq_ignore_ascii_case(b"coding")
                        && (!key.eq_ignore_ascii_case(b"rbs_inline")
                            || matches!(parts.get(2).unwrap().as_bytes(), b"enabled" | b"disabled"))
                });
        }
        comment
    }

    /// First setting token, with editor keys case-sensitive as in RuboCop.
    fn setting(&self, key: &[u8]) -> Option<&'a [u8]> {
        let key_matches = |candidate: &[u8], insensitive: bool| {
            candidate.len() == key.len()
                && candidate.iter().zip(key).all(|(&a, &b)| {
                    a == b
                        || (a == b'-' && b == b'_')
                        || (insensitive && a.eq_ignore_ascii_case(&b))
                })
        };
        match &self.kind {
            Kind::Simple(text) => SIMPLE.captures(text).and_then(|parts| {
                key_matches(&parts[1], true).then(|| parts.get(2).unwrap().as_bytes())
            }),
            Kind::Emacs(tokens) => tokens.iter().find_map(|token| {
                EDITOR.captures(token).and_then(|parts| {
                    key_matches(&parts[1], false).then(|| parts.get(2).unwrap().as_bytes())
                })
            }),
            Kind::Vim(_) => None,
        }
    }

    /// Invalid but specified values deliberately stop the header search too.
    pub fn frozen_strings(&self) -> Option<bool> {
        self.setting(b"frozen_string_literal")
            .map(|value| value.eq_ignore_ascii_case(b"true"))
    }

    /// Only valid shareable values change the current state.
    pub fn shareable_constants(&self) -> Option<bool> {
        let value = self.setting(b"shareable_constant_value")?;
        let equals = |expected: &[u8]| {
            if matches!(self.kind, Kind::Emacs(_)) {
                value.eq_ignore_ascii_case(expected)
            } else {
                value == expected
            }
        };
        if equals(b"none") {
            Some(false)
        } else if [
            b"literal".as_slice(),
            b"experimental_everything",
            b"experimental_copy",
        ]
        .into_iter()
        .any(equals)
        {
            Some(true)
        } else {
            None
        }
    }

    pub fn without_encoding(&self) -> Option<String> {
        if !self.rewrite_safe {
            return None;
        }
        let bytes = match &self.kind {
            Kind::Simple(text) => {
                if ENCODING
                    .captures(text)
                    .is_some_and(|p| trim(&text[1..]).starts_with(p.get(1).unwrap().as_bytes()))
                {
                    if directives(text).count() > 1 {
                        return None;
                    }
                    Vec::new()
                } else {
                    text.to_vec()
                }
            }
            Kind::Emacs(tokens) => {
                editor_without(tokens, b"coding", b"encoding", "# -*- ", ";", " -*-")
            }
            Kind::Vim(tokens) => editor_without(
                tokens,
                b"fileencoding",
                b"fileencoding",
                "# vim: ",
                ", ",
                "",
            ),
        };
        String::from_utf8(bytes).ok()
    }
}

fn editor_without(
    tokens: &[&[u8]],
    key: &[u8],
    other: &[u8],
    prefix: &str,
    separator: &str,
    suffix: &str,
) -> Vec<u8> {
    let remaining: Vec<_> = tokens
        .iter()
        .filter(|t| {
            ![key, other].iter().any(|k| {
                t.strip_prefix(*k).is_some_and(|rest| {
                    rest.first()
                        .is_some_and(|b| *b == b':' || *b == b'=' || super::is_ruby_whitespace(*b))
                })
            })
        })
        .copied()
        .collect();
    if remaining.is_empty() {
        return Vec::new();
    }
    let mut bytes = prefix.as_bytes().to_vec();
    for (i, token) in remaining.iter().enumerate() {
        if i > 0 {
            bytes.extend_from_slice(separator.as_bytes());
        }
        bytes.extend_from_slice(token);
    }
    bytes.extend_from_slice(suffix.as_bytes());
    bytes
}
fn split_vim(mut text: &[u8]) -> Vec<&[u8]> {
    let mut tokens = Vec::new();
    while let Some(i) = memchr::memmem::find(text, b", ") {
        tokens.push(&text[..i]);
        text = &text[i + 2..];
    }
    tokens.push(text);
    // String#split drops trailing empty fields before String#strip runs.
    // A whitespace-only field is not empty until after that split step.
    while tokens.last().is_some_and(|token| token.is_empty()) {
        tokens.pop();
    }
    tokens.into_iter().map(trim).collect()
}
fn trim(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|b| !super::is_ruby_whitespace(*b) && *b != 0)
        .unwrap_or(text.len());
    trim_end(&text[start..])
}

pub(super) fn trim_end(text: &[u8]) -> &[u8] {
    let end = text
        .iter()
        .rposition(|b| !super::is_ruby_whitespace(*b) && *b != 0)
        .map_or(0, |i| i + 1);
    &text[..end]
}

pub(super) fn directives(text: &[u8]) -> impl Iterator<Item = Range<usize>> + '_ {
    DIRECTIVES.find_iter(text).map(|m| m.range())
}

/// End of the first code-token line (upstream includes comments on that line).
/// One forward scan of bytes and sorted Prism comments; never lex literal text.
pub(super) fn leading_end(file: &SourceFile) -> usize {
    let mut offset = if file.bytes.starts_with(b"\xef\xbb\xbf") {
        3
    } else {
        0
    };
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
            b if super::is_ruby_whitespace(b) => offset += 1,
            b'\\' if file.bytes.get(offset + 1) == Some(&b'\n') => offset += 2,
            b'\\' if file.bytes.get(offset + 1..offset + 3) == Some(b"\r\n") => offset += 3,
            _ => return file.line_start(file.line_of(offset) + 1),
        }
    }
    offset
}
