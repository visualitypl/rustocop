//! Shared bracket-adjacent gaps, using Prism delimiters rather than a lexer.
use std::ops::Range;

use ruby_prism::{Location, Node};

use crate::{
    cops::{Cop, NodeContext, style::correction::line_value_boundary},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub(super) fn offsets(
    delimiters: (Option<Location<'_>>, Option<Location<'_>>),
) -> Option<(usize, usize)> {
    let (Some(left), Some(right)) = delimiters else {
        return None;
    };
    (left.as_slice() == b"[" && right.as_slice() == b"]")
        .then(|| (left.start_offset(), right.start_offset()))
}

pub(super) enum Kind {
    Array { skip_left: bool, skip_right: bool },
    Reference,
}

pub(super) fn check(
    cop: &dyn Cop,
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    delimiters: Option<(usize, usize)>,
    kind: Kind,
    out: &mut Vec<Offense>,
) {
    let (reference, skip_left, skip_right) = match kind {
        Kind::Array {
            skip_left,
            skip_right,
        } => (false, skip_left, skip_right),
        Kind::Reference => (true, false, false),
    };
    let Some((left, right)) = delimiters else {
        return;
    };
    let start = left + 1;
    let end = right;
    let inside = &file.bytes[start..end];
    let kind = if reference { "reference" } else { "array" };
    let root = cx.ancestors().next_back().unwrap_or(node);
    if empty_gap(inside) {
        if !inside.is_empty() {
            report(
                cop,
                cx,
                file,
                root,
                left..right + 1,
                start..end,
                format!("Do not use space inside empty {kind} brackets."),
                out,
            );
        }
        return;
    }
    // A plain setter's RHS belongs to its send, while compound/multiple
    // assignment RHSs do not. Literal blocks are outside the send as well.
    let send_end = if node.as_call_node().is_some() {
        crate::cops::ast::source_end(node)
    } else {
        right + 1
    };
    if reference
        && file.line_of(node.location().start_offset()) != file.line_of(send_end.saturating_sub(1))
    {
        return;
    }
    let protected = file.literal_token_ranges(root);
    let blank = |i: usize| {
        let after = protected.partition_point(|r| r.start <= i);
        matches!(file.bytes[i], b' ' | b'\t') && !(after > 0 && protected[after - 1].contains(&i))
    };
    let mut first = start;
    while first < end && blank(first) {
        first += 1;
    }
    // Unlike parentheses, array openings still report blanks before a newline.
    // A following comment token alone exempts this side in the pinned cop.
    let next = start + skip_gap(inside);
    let next_is_comment = file
        .comments()
        .binary_search_by_key(&next, |r| r.start)
        .is_ok();
    if !skip_left && first > start && !next_is_comment {
        report(
            cop,
            cx,
            file,
            root,
            start..first,
            start..first,
            format!("Do not use space inside {kind} brackets."),
            out,
        );
    }
    let mut last = end;
    while last > start && blank(last - 1) {
        last -= 1;
    }
    let mut highlight_start = last;
    while highlight_start > start && matches!(file.bytes[highlight_start - 1], b' ' | b'\t') {
        highlight_start -= 1;
    }
    // A closing bracket on its own line belongs to multiline indentation.
    // Detection includes a whitespace percent-literal delimiter, as upstream
    // does; correction never removes that delimiter, even if it parses after.
    if !skip_right
        && highlight_start < end
        && highlight_start > start
        && file.bytes[highlight_start - 1] != b'\n'
    {
        report(
            cop,
            cx,
            file,
            root,
            highlight_start..end,
            last..end,
            format!("Do not use space inside {kind} brackets."),
            out,
        );
    }
}

/// Line-continuation bytes are not tokens between otherwise empty brackets.
fn empty_gap(bytes: &[u8]) -> bool {
    skip_gap(bytes) == bytes.len()
}

fn skip_gap(bytes: &[u8]) -> usize {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
        } else if bytes[i..].starts_with(b"\\\n") {
            i += 2;
        } else if bytes[i..].starts_with(b"\\\r\n") {
            i += 3;
        } else {
            break;
        }
    }
    i
}

#[allow(clippy::too_many_arguments)]
fn report(
    cop: &dyn Cop,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    root: &Node<'_>,
    highlight: Range<usize>,
    range: Range<usize>,
    message: String,
    out: &mut Vec<Offense>,
) {
    let (line, col) = file.line_col(highlight.start);
    let newline = file.bytes[range.clone()].contains(&b'\n');
    let correction = (cx.corrections_requested()
        && !range.is_empty()
        && (!newline
            || (range.start >= line_value_boundary(file)
                // Squiggly-heredoc dedentation depends on physical lines;
                // keep multiline interpolation edits conservative.
                && !cx.ancestors().any(|ancestor| {
                    ancestor.as_interpolated_string_node().and_then(|n| n.opening_loc())
                        .or_else(|| ancestor.as_interpolated_x_string_node().map(|n| n.opening_loc()))
                        .is_some_and(|loc| loc.as_slice().starts_with(b"<<"))
                })
                && !file.overlaps_literal(root, &range))))
    .then(|| Correction {
        range,
        replacement: String::new(),
    });
    out.push(Offense {
        correction,
        cop: cop.name(),
        severity: cop.severity(),
        message,
        line,
        // Parser's `last_line`: a highlight ending in a newline reaches the next line.
        end_line: file.line_of(highlight.end),
        col,
    });
}
