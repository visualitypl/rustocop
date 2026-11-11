//! `Layout/DotPosition` — the dot of a multi-line method call goes on the
//! line of the method name (fixed `leading` style).
//!
//! Port of RuboCop's `on_send`/`on_csend`: every parser-gem `send`/`csend`
//! whose call operator is `.` or `&.` (never `::`). Under the Prism
//! translation those are calls, attribute writes, the call targets of
//! multiple assignments and the call side of `a.b += 1`/`||=`/`&&=`.
use std::ops::Range;

use ruby_prism::{Location, Node};

use super::heredoc::{call_type, has_literal_block};
use crate::{
    cops::{Cop, NodeContext, naming::heredoc::Heredoc},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct DotPosition;

const LEADING: &str = "Place the %s on the next line, together with the method name.";

impl Cop for DotPosition {
    fn name(&self) -> &'static str {
        "Layout/DotPosition"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = DottedCall::of(node) else {
            return;
        };
        let dot = call.dot.as_slice();
        if dot != b"." && dot != b"&." {
            return;
        }
        if proper_dot_position(&call, file) {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().last().unwrap_or(node);
                if file
                    .class_method_bindings(root)
                    .has_listed_source_observer(file.data_start().is_some())
                {
                    return None;
                }
                move_dot_before_selector(&call, file)
            })
            .flatten();
        let dot_start = call.dot.start_offset();
        let (line, col) = file.line_col(dot_start);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: LEADING.replace("%s", &String::from_utf8_lossy(dot)),
            line,
            end_line: line,
            col,
        });
    }
}

/// The parts of a parser-gem `send`/`csend` with a call operator.
struct DottedCall<'pr> {
    receiver: Node<'pr>,
    dot: Location<'pr>,
    /// `loc.selector || loc.begin`: `l.(1)` has no selector, so its opening
    /// parenthesis stands in for it.
    selector: Location<'pr>,
}

impl<'pr> DottedCall<'pr> {
    fn of(node: &Node<'pr>) -> Option<Self> {
        let (receiver, dot, selector) = match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node()?;
                (
                    call.receiver(),
                    call.call_operator_loc(),
                    call.message_loc().or_else(|| call.opening_loc()),
                )
            }
            Node::CallOperatorWriteNode { .. } => {
                let call = node.as_call_operator_write_node()?;
                (
                    call.receiver(),
                    call.call_operator_loc(),
                    call.message_loc(),
                )
            }
            Node::CallOrWriteNode { .. } => {
                let call = node.as_call_or_write_node()?;
                (
                    call.receiver(),
                    call.call_operator_loc(),
                    call.message_loc(),
                )
            }
            Node::CallAndWriteNode { .. } => {
                let call = node.as_call_and_write_node()?;
                (
                    call.receiver(),
                    call.call_operator_loc(),
                    call.message_loc(),
                )
            }
            Node::CallTargetNode { .. } => {
                let call = node.as_call_target_node()?;
                (
                    Some(call.receiver()),
                    Some(call.call_operator_loc()),
                    Some(call.message_loc()),
                )
            }
            _ => return None,
        };
        Some(Self {
            receiver: receiver?,
            dot: dot?,
            selector: selector?,
        })
    }
}

/// `proper_dot_position?` at the fixed `leading` style.
fn proper_dot_position(call: &DottedCall<'_>, file: &SourceFile) -> bool {
    let selector_line = file.line_of(call.selector.start_offset());
    if selector_line == file.line_of(call.receiver.location().end_offset()) {
        return true;
    }
    let receiver_line = receiver_end_line(&call.receiver, file);
    let dot_line = file.line_of(call.dot.start_offset());
    // A comment or blank line between the dot and the selector (after a
    // receiver heredoc's body, which ends after the dot) keeps the call.
    if selector_line.saturating_sub(receiver_line.max(dot_line)) > 1 {
        return true;
    }
    dot_line == selector_line
}

/// `receiver_end_line`: the last heredoc terminator line of a heredoc
/// receiver or of a receiver call's direct heredoc arguments, otherwise the
/// receiver's last line.
fn receiver_end_line(receiver: &Node<'_>, file: &SourceFile) -> u32 {
    let heredoc_end = |node: &Node<'_>| {
        Heredoc::of(node, file).map(|heredoc| file.line_of(heredoc.closing.start_offset()))
    };
    let last_heredoc_line = if call_type(receiver).is_some() && !has_literal_block(receiver) {
        receiver
            .as_call_node()
            .and_then(|call| call.arguments())
            .and_then(|args| {
                args.arguments()
                    .iter()
                    .filter_map(|a| heredoc_end(&a))
                    .max()
            })
    } else {
        heredoc_end(receiver)
    };
    last_heredoc_line.unwrap_or_else(|| file.line_of(receiver.location().end_offset()))
}

/// The upstream leading-style correction: the dot (with its whole line when
/// it stands alone there) and the gap up to the selector are replaced by the
/// gap followed by the dot, as one atomic edit.
fn move_dot_before_selector(call: &DottedCall<'_>, file: &SourceFile) -> Option<Correction> {
    let dot = call.dot.start_offset()..call.dot.end_offset();
    let dot_range = removed_dot_range(dot.clone(), file);
    let gap = dot_range.end..call.selector.start_offset();
    if gap.start > gap.end {
        return None;
    }
    let mut replacement = file.bytes[gap.clone()].to_vec();
    replacement.extend_from_slice(&file.bytes[dot]);
    Some(Correction {
        range: dot_range.start..gap.end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

/// `removed_dot_range`: the whole line, final newline included, when the
/// stripped line is just `.`.
fn removed_dot_range(dot: Range<usize>, file: &SourceFile) -> Range<usize> {
    let line = file.line_of(dot.start);
    if ruby_strip(file.line_text(line)) != b"." {
        return dot;
    }
    file.line_start(line)..file.line_start(line + 1)
}

/// Ruby's `String#strip`: leading and trailing whitespace and NUL.
fn ruby_strip(text: &[u8]) -> &[u8] {
    let space = |b: &u8| matches!(b, b' ' | b'\t'..=b'\r' | b'\0');
    let start = text.iter().position(|b| !space(b)).unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|b| !space(b))
        .map_or(start, |i| i + 1);
    &text[start..end]
}

#[cfg(test)]
mod tests;
