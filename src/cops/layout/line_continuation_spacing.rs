//! `Layout/LineContinuationSpacing` — one space before the backslash of a
//! line continuation.
//!
//! A port of RuboCop's `on_new_investigation` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: space`. Every raw source line up to the line of the last
//! parser token (so never the `__END__` data) is matched against
//! `/((?<!\s)|\s{2,})\\$/`: a backslash right before the line's `\n` with
//! no blank before it, or with two or more blanks (Ruby's `\s`) before it.
//! The match (the blanks and the backslash) is reported unless an ignored
//! range contains it: a `str`/`dstr` with an opening delimiter (quoted,
//! percent and `?` literals), a `str` inside a regexp or backtick literal, a
//! percent array, a heredoc body, a comment, or the `$\` global variable. As
//! upstream, a backslash before `\r\n` never matches `\\$`, so CRLF lines are
//! not checked, and symbols are not ignored.
//!
//! The correction replaces the match with one space and the backslash, a
//! canonical edit of blanks before a continuation: Ruby's lexer already
//! treats the continuation as a blank, so only blanks between two tokens
//! change. It is withheld when
//!
//! - the match overlaps literal text (a backslash-newline inside a symbol is
//!   an escape, and its blanks are part of the value),
//! - a space would be inserted after a byte whose following byte the lexer
//!   inspects (`foo -\` passes `-1`, `foo - \` subtracts): only identifier
//!   and number characters, closing brackets and quotes, `,`, `=` and `|`
//!   may precede the inserted space, and non-ASCII bytes only in UTF-8
//!   source (a legacy double-byte character can end in the `\` byte),
//! - the file has one of the listed source observers.
use std::ops::Range;

use ruby_prism::{Node, ParseResult};

use crate::{
    cops::{
        Cop,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct LineContinuationSpacing;

const MSG: &str = "Use one space in front of backslash.";

/// Ruby's regexp `\s`.
fn blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// The upstream match on one line (without its `\n`): the byte range of the
/// offending blanks and backslash, relative to the line start.
fn offensive_spacing(text: &[u8]) -> Option<Range<usize>> {
    let backslash = text.len().checked_sub(1)?;
    if text[backslash] != b'\\' {
        return None;
    }
    let blanks = text[..backslash]
        .iter()
        .rev()
        .take_while(|b| blank(**b))
        .count();
    match blanks {
        0 => Some(backslash..text.len()),
        1 => None,
        _ => Some(backslash - blanks..text.len()),
    }
}

/// Whether the line's final `\` byte is a character of its own. Upstream
/// matches characters in the source encoding, where the second byte of a
/// legacy double-byte character (Shift_JIS, Big5, GBK) can be `0x5C`.
fn backslash_character(file: &SourceFile, line_start: usize, body: &[u8]) -> bool {
    let backslash = line_start + body.len() - 1;
    file.is_utf8()
        || body.is_ascii()
        || file.character_offsets(line_start..backslash + 1).last() == Some(backslash)
}

/// Bytes after which inserting a blank cannot change how the lexer reads
/// the following continuation (see the module documentation).
fn insertion_safe(file: &SourceFile, byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'_' | b')' | b']' | b'}' | b'\'' | b'"' | b'`' | b',' | b'=' | b'|'
        )
        || (byte >= 0x80 && file.is_utf8())
}

impl Cop for LineContinuationSpacing {
    fn name(&self) -> &'static str {
        "Layout/LineContinuationSpacing"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_parse(&self, result: &ParseResult<'_>, file: &SourceFile, out: &mut Vec<Offense>) {
        if !file.contains(b"\\\n") {
            return;
        }
        let mut candidates = Vec::new();
        let mut line_start = 0;
        for text in file.bytes.split_inclusive(|b| *b == b'\n') {
            if let Some(body) = text.strip_suffix(b"\n")
                && let Some(range) = offensive_spacing(body)
                && backslash_character(file, line_start, body)
            {
                candidates.push(line_start + range.start..line_start + range.end);
            }
            line_start += text.len();
        }
        if candidates.is_empty() {
            return;
        }
        let last_line = file
            .parser_tokens()
            .last()
            .map_or(file.line_count() as u32, |token| token.line);
        let ignored = ignored_ranges(file, &result.node());
        for range in candidates {
            let (line, col) = file.line_col(range.start);
            if line > last_line {
                break;
            }
            if ignored.contains(&range) {
                continue;
            }
            let insertion = range.len() == 1;
            let correction = (!(insertion
                && (range.start == file.line_start(line)
                    || !insertion_safe(file, file.bytes[range.start - 1])))
                && !file.overlaps_literal(&result.node(), &range))
            .then(|| Correction {
                range: range.clone(),
                replacement: " \\".into(),
            });
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: MSG.into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

/// Upstream's `ignored_ranges`: literal ranges, comments and `$\`, sorted
/// by start with the running maximum end, so that containment is one binary
/// search.
struct IgnoredRanges {
    starts: Vec<usize>,
    max_ends: Vec<usize>,
}

impl IgnoredRanges {
    /// Whether an ignored range contains `range`.
    fn contains(&self, range: &Range<usize>) -> bool {
        let index = self.starts.partition_point(|&start| start <= range.start);
        index > 0 && self.max_ends[index - 1] >= range.end
    }
}

fn ignored_ranges(file: &SourceFile, root: &Node<'_>) -> IgnoredRanges {
    let mut ranges = Ignored(Vec::new());
    walk(&mut ranges, root);
    let mut ranges = ranges.0;
    ranges.extend(file.comments().iter().cloned());
    ranges.sort_by_key(|r| r.start);
    let mut max_end = 0;
    IgnoredRanges {
        starts: ranges.iter().map(|r| r.start).collect(),
        max_ends: ranges
            .iter()
            .map(|r| {
                max_end = max_end.max(r.end);
                max_end
            })
            .collect(),
    }
}

struct Ignored(Vec<Range<usize>>);

fn span(location: &ruby_prism::Location<'_>) -> Range<usize> {
    location.start_offset()..location.end_offset()
}

impl Ignored {
    /// A heredoc's body (upstream `loc.heredoc_body`): from its first part to
    /// the terminator line.
    fn heredoc_body(&mut self, first: Option<usize>, closing: Option<ruby_prism::Location<'_>>) {
        if let (Some(start), Some(closing)) = (first, closing) {
            self.0.push(start..closing.start_offset());
        }
    }

    /// The `str` parts of a regexp or backtick literal.
    fn string_parts(&mut self, parts: ruby_prism::NodeList<'_>) {
        for part in parts.iter() {
            if let Some(string) = part.as_string_node() {
                self.0.push(span(&string.location()));
            }
        }
    }
}

fn heredoc(opening: Option<ruby_prism::Location<'_>>) -> bool {
    opening.is_some_and(|o| o.as_slice().starts_with(b"<<"))
}

impl<'pr> NodeVisitor<'pr> for Ignored {
    fn enter(&mut self, node: Node<'pr>) {
        match &node {
            Node::StringNode { .. } => {
                let string = node.as_string_node().unwrap();
                if heredoc(string.opening_loc()) {
                    self.heredoc_body(
                        Some(string.content_loc().start_offset()),
                        string.closing_loc(),
                    );
                } else if string.opening_loc().is_some() {
                    self.0.push(span(&string.location()));
                }
            }
            Node::InterpolatedStringNode { .. } => {
                let string = node.as_interpolated_string_node().unwrap();
                if heredoc(string.opening_loc()) {
                    let first = string.parts().iter().next();
                    self.heredoc_body(
                        first.map(|p| p.location().start_offset()),
                        string.closing_loc(),
                    );
                } else if string.opening_loc().is_some() {
                    self.0.push(span(&string.location()));
                }
            }
            Node::XStringNode { .. } => {
                self.0
                    .push(span(&node.as_x_string_node().unwrap().content_loc()));
            }
            Node::InterpolatedXStringNode { .. } => {
                self.string_parts(node.as_interpolated_x_string_node().unwrap().parts());
            }
            Node::RegularExpressionNode { .. } => {
                let regexp = node.as_regular_expression_node().unwrap();
                self.0.push(span(&regexp.content_loc()));
            }
            Node::MatchLastLineNode { .. } => {
                let regexp = node.as_match_last_line_node().unwrap();
                self.0.push(span(&regexp.content_loc()));
            }
            Node::InterpolatedRegularExpressionNode { .. } => {
                self.string_parts(
                    node.as_interpolated_regular_expression_node()
                        .unwrap()
                        .parts(),
                );
            }
            Node::InterpolatedMatchLastLineNode { .. } => {
                self.string_parts(node.as_interpolated_match_last_line_node().unwrap().parts());
            }
            Node::ArrayNode { .. } => {
                let array = node.as_array_node().unwrap();
                if array
                    .opening_loc()
                    .is_some_and(|o| o.as_slice().starts_with(b"%"))
                {
                    self.0.push(span(&array.location()));
                }
            }
            Node::GlobalVariableReadNode { .. } => {
                let gvar = node.as_global_variable_read_node().unwrap();
                if gvar.name().as_slice() == b"$\\" {
                    self.0.push(span(&gvar.location()));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
