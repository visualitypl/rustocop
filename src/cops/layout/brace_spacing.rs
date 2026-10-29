//! Brace offense construction and conservative empty-hash newline corrections.
use std::ops::Range;

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        style::correction::{line_value_boundary, touches_directive},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// Ruby's `\s` includes vertical tabs, unlike Rust's ASCII whitespace predicate.
pub(super) fn is_ruby_whitespace(byte: u8) -> bool {
    byte.is_ascii_whitespace() || byte == 0x0b
}

#[allow(clippy::too_many_arguments)]
pub(super) fn report(
    cop: &dyn Cop,
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    highlight: Range<usize>,
    range: Range<usize>,
    replacement: &str,
    message: &str,
    out: &mut Vec<Offense>,
) {
    let newline = file.bytes[range.clone()].contains(&b'\n');
    let root = cx.ancestors().next_back().unwrap_or(node);
    let correction = (cx.corrections_requested()
        && (!newline
            || (range.start >= line_value_boundary(file)
                // Include the closing brace's line: an interior newline range
                // itself ends before that brace and can miss its directive.
                && !touches_directive(file, range.start..(range.end + 1).min(file.bytes.len()))
                // Removing physical lines can change squiggly-heredoc dedentation.
                && !cx.ancestors().any(|ancestor| {
                    ancestor.as_interpolated_string_node().and_then(|n| n.opening_loc())
                        .or_else(|| ancestor.as_interpolated_x_string_node().map(|n| n.opening_loc()))
                        .is_some_and(|loc| loc.as_slice().starts_with(b"<<"))
                })
                && !file.overlaps_literal(root, &range))))
        .then(|| Correction { range, replacement: replacement.to_owned() });
    let (line, col) = file.line_col(highlight.start);
    out.push(Offense {
        correction,
        cop: cop.name(),
        severity: cop.severity(),
        message: message.to_owned(),
        line,
        end_line: file.line_of(highlight.end.saturating_sub(1).max(highlight.start)),
        col,
    });
}
