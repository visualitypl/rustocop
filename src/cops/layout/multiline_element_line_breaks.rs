//! A narrow port of RuboCop's `MultilineElementLineBreaks` mixin (97f30c4,
//! MIT) for the fixed `AllowMultilineFinalElement: false`, shared by the
//! array, hash key, method argument and method parameter line-break cops.
//!
//! `check_line_breaks`: unless the first child's first line equals the last
//! child's last line, every child starting on or before the last line of the
//! previous child that did not offend is reported, except one that a heredoc
//! opened in an earlier child on its own line precedes
//! (`heredoc_opened_before?`). The correction is upstream's
//! `EmptyLineCorrector.insert_before(child, "\n")`, offered through the
//! accepted first-element seam (`first_element_line_break::report`): one
//! line break before the child, nothing else moved, even in CRLF files,
//! withheld for the listed source observers, with line-number
//! observers after the insertion and heredoc bodies that would start at the
//! inserted break held by the shared `run_cops` boundary.
use ruby_prism::Node;

use super::first_element_line_break::report;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// Report every child of a multi-line collection that starts on a line an
/// earlier child already occupies. `children` are in source order.
pub(crate) fn check_line_breaks(
    cop: &dyn Cop,
    message: &str,
    children: &[Node<'_>],
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let (Some(first), Some(last)) = (children.first(), children.last()) else {
        return;
    };
    // `all_on_same_line?`: `nodes.first.first_line == nodes.last.last_line`.
    if file.line_of(first.location().start_offset()) == file.last_line_of(last.location()) {
        return;
    }
    let mut last_seen_line = None;
    for (index, child) in children.iter().enumerate() {
        let line = file.line_of(child.location().start_offset());
        if last_seen_line.is_some_and(|seen| seen >= line) {
            if !heredoc_opened_before(file, cx, &children[..index], line) {
                report(cop, message, child, cx, file, out);
            }
        } else {
            last_seen_line = Some(file.last_line_of(child.location()));
        }
    }
}

/// `heredoc_opened_before?`: some heredoc string inside an earlier child
/// (the child itself included) opens on `line`. The earlier children are
/// disjoint and in source order, so only those reaching `line` can hold such
/// an opener; the shared main-parse openings answer each range exactly.
fn heredoc_opened_before(
    file: &SourceFile,
    cx: &NodeContext<'_, '_>,
    previous: &[Node<'_>],
    line: u32,
) -> bool {
    let Some(root) = cx.ancestors().next_back() else {
        return false;
    };
    let line_start = file.line_start(line);
    let line_end = file.line_start(line + 1);
    previous
        .iter()
        .rev()
        .map(|child| child.location())
        .take_while(|location| location.end_offset() > line_start)
        .any(|location| {
            let start = location.start_offset().max(line_start);
            let end = location.end_offset().min(line_end);
            start < end && file.heredoc_opening_in(root, start..end)
        })
}
