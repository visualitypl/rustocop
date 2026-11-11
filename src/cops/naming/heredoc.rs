//! Main-AST locations, never physical containment guesses for queued owners.
use crate::source::SourceFile;
use ruby_prism::{Location, Node, NodeList};
use std::ops::Range;
pub(crate) struct Heredoc<'pr> {
    pub opening: Location<'pr>,
    pub closing: Location<'pr>,
    pub body: Range<usize>,
    pub empty: bool,
}
impl<'pr> Heredoc<'pr> {
    pub fn of(node: &Node<'pr>, file: &SourceFile) -> Option<Self> {
        let (opening, closing, body, empty) = match node {
            Node::StringNode { .. } => {
                let s = node.as_string_node()?;
                (
                    s.opening_loc(),
                    s.closing_loc(),
                    Some(s.content_loc().start_offset()..s.content_loc().end_offset()),
                    s.unescaped().is_empty(),
                )
            }
            Node::InterpolatedStringNode { .. } => {
                let s = node.as_interpolated_string_node()?;
                let opening = s
                    .opening_loc()
                    .filter(|l| l.as_slice().starts_with(b"<<"))?;
                let body = parts_body(s.parts());
                (Some(opening), s.closing_loc(), body.clone(), body.is_none())
            }
            Node::XStringNode { .. } => {
                let s = node.as_x_string_node()?;
                (
                    Some(s.opening_loc()),
                    Some(s.closing_loc()),
                    Some(s.content_loc().start_offset()..s.content_loc().end_offset()),
                    s.unescaped().is_empty(),
                )
            }
            Node::InterpolatedXStringNode { .. } => {
                let s = node.as_interpolated_x_string_node()?;
                let opening = s.opening_loc();
                if !opening.as_slice().starts_with(b"<<") {
                    return None;
                }
                let body = parts_body(s.parts());
                (
                    Some(opening),
                    Some(s.closing_loc()),
                    body.clone(),
                    body.is_none(),
                )
            }
            _ => return None,
        };
        let opening = opening.filter(|l| l.as_slice().starts_with(b"<<"))?;
        let closing = closing?;
        let end = file.line_start(file.line_of(closing.start_offset()));
        let body = body.map_or(end..end, |range| {
            file.line_start(file.line_of(range.start))..range.end
        });
        Some(Self {
            opening,
            closing,
            body,
            empty,
        })
    }
    /// Exact pinned mixin OPENING_DELIMITER capture, including its unanchored
    /// search and optional-modifier backtracking. This is NOT rewrite authority.
    pub fn delimiter_range(&self, file: &SourceFile) -> Range<usize> {
        let opening_end = self.opening.end_offset();
        for head in file.character_offsets(self.opening.start_offset()..opening_end) {
            if !file.bytes[head..opening_end].starts_with(b"<<") {
                continue;
            }
            let base = head + 2;
            let modifier = usize::from(matches!(file.bytes.get(base), Some(b'~' | b'-')));
            for skip in (0..=modifier).rev() {
                let mut start = base + skip;
                if matches!(file.bytes.get(start), Some(b'\'' | b'"' | b'`')) {
                    start += 1;
                }
                let end = file
                    .character_offsets(start..opening_end)
                    .find(|&i| matches!(file.bytes[i], b'\'' | b'"' | b'`'))
                    .unwrap_or(opening_end);
                if start < end {
                    return start..end;
                }
            }
        }
        opening_end..opening_end
    }
}
/// The pinned Prism translation's heredoc_body spans composed child locations,
/// not all bytes before the terminator. Deferred nested bodies after the last
/// child therefore do not participate in its collision predicate. Correction
/// safety independently uses the accepted *physical* boundary index.
fn parts_body(parts: NodeList<'_>) -> Option<Range<usize>> {
    let mut parts = parts
        .iter()
        .filter(|p| !p.as_string_node().is_some_and(|s| s.unescaped().is_empty()));
    let first = parts.next()?;
    let end = parts.last().map_or_else(
        || first.location().end_offset(),
        |p| p.location().end_offset(),
    );
    Some(first.location().start_offset()..end)
}
/// Ruby String#strip's ASCII/NUL set, not Unicode whitespace (Rust excludes VT).
pub(crate) fn strip(bytes: &[u8]) -> &[u8] {
    let blank = |b: &u8| matches!(b, b' ' | b'\t'..=b'\r' | 0);
    let start = bytes.iter().position(|b| !blank(b)).unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !blank(b))
        .map_or(start, |n| n + 1);
    &bytes[start..end]
}
