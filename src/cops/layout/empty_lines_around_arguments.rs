//! `Layout/EmptyLinesAroundArguments` — no empty lines around the arguments
//! of a multi-line method call.
//!
//! Upstream's `on_send`/`on_csend` over parser sends: Prism calls, and the
//! index sends inside index operator-assignments and multiple-assignment
//! targets (`a[1] += 2`, `a[1], b = ...`). A send is skipped when it is on
//! one line (its block excluded), has no arguments (a block-pass counts), or
//! has a receiver whose last line is not the selector's line (no selector,
//! as in `a.()`, counts as a different line). The whitespace run before each
//! argument and before the closing parenthesis (index brackets are part of
//! the parser selector, never an end) reports the line before the
//! argument's line when it spans more than one line break.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};

pub struct EmptyLinesAroundArguments;

impl Cop for EmptyLinesAroundArguments {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundArguments"
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
        let Some(send) = Send::of(node) else {
            return;
        };
        if send.arguments.is_empty()
            || file.line_of(send.start) == file.line_of(send.end.saturating_sub(1).max(send.start))
        {
            return;
        }
        if let Some((start, end)) = send.receiver {
            let selector = send.selector.map(|s| file.line_of(s));
            if Some(file.line_of(end.saturating_sub(1).max(start))) != selector {
                return;
            }
        }
        let start = out.len();
        for at in send.arguments.iter().copied().chain(send.closing) {
            let Some(line) = blank_run_line(file, at) else {
                continue;
            };
            out.push(Offense {
                correction: super::body_padding::line_removal(cx, file, line),
                cop: self.name(),
                severity: self.severity(),
                message: "Empty line detected around arguments.".into(),
                line,
                end_line: line,
                col: 1,
            });
        }
        super::body_padding::hold_for_listed_observers(cx, file, &mut out[start..]);
    }
}

/// The parts of a parser send that upstream reads.
struct Send {
    /// Start and end offsets of the send itself (never its literal block).
    start: usize,
    end: usize,
    /// Start and end offsets of the receiver.
    receiver: Option<(usize, usize)>,
    /// The start of the selector (`[` for index sends).
    selector: Option<usize>,
    /// Start offsets of the arguments, a block-pass included.
    arguments: Vec<usize>,
    /// The start of the closing parenthesis (`loc.end`).
    closing: Option<usize>,
}

impl Send {
    fn of(node: &Node<'_>) -> Option<Self> {
        if let Some(call) = node.as_call_node() {
            let index = call.opening_loc().is_some_and(|o| o.as_slice() == b"[");
            let parts = [
                call.receiver().map(|r| r.location()),
                call.message_loc(),
                call.opening_loc(),
                call.arguments().map(|a| a.location()),
                call.closing_loc(),
                call.block()
                    .filter(|b| b.as_block_argument_node().is_some())
                    .map(|b| b.location()),
            ];
            return Some(Self {
                start: node.location().start_offset(),
                end: parts
                    .iter()
                    .flatten()
                    .map(Location::end_offset)
                    .max()
                    .unwrap_or_else(|| node.location().start_offset()),
                receiver: call.receiver().map(|r| span(&r.location())),
                selector: if index {
                    call.opening_loc().map(|o| o.start_offset())
                } else {
                    call.message_loc().map(|m| m.start_offset())
                },
                arguments: arguments(
                    call.arguments(),
                    call.block()
                        .filter(|b| b.as_block_argument_node().is_some())
                        .map(|b| b.location().start_offset()),
                ),
                closing: (!index)
                    .then(|| call.closing_loc().map(|c| c.start_offset()))
                    .flatten(),
            });
        }
        let (receiver, opening, args, closing, block) =
            if let Some(n) = node.as_index_operator_write_node() {
                (
                    n.receiver(),
                    n.opening_loc(),
                    n.arguments(),
                    n.closing_loc(),
                    n.block(),
                )
            } else if let Some(n) = node.as_index_and_write_node() {
                (
                    n.receiver(),
                    n.opening_loc(),
                    n.arguments(),
                    n.closing_loc(),
                    n.block(),
                )
            } else if let Some(n) = node.as_index_or_write_node() {
                (
                    n.receiver(),
                    n.opening_loc(),
                    n.arguments(),
                    n.closing_loc(),
                    n.block(),
                )
            } else {
                let n = node.as_index_target_node()?;
                (
                    Some(n.receiver()),
                    n.opening_loc(),
                    n.arguments(),
                    n.closing_loc(),
                    n.block(),
                )
            };
        let receiver = receiver.map(|r| span(&r.location()));
        Some(Self {
            start: receiver.map_or(opening.start_offset(), |(start, _)| start),
            end: closing.end_offset(),
            receiver,
            selector: Some(opening.start_offset()),
            arguments: arguments(args, block.map(|b| b.location().start_offset())),
            closing: None,
        })
    }
}

fn span(location: &Location<'_>) -> (usize, usize) {
    (location.start_offset(), location.end_offset())
}

/// Argument start offsets, then the block-pass's.
fn arguments(args: Option<ruby_prism::ArgumentsNode<'_>>, block_pass: Option<usize>) -> Vec<usize> {
    let mut starts: Vec<usize> = args
        .iter()
        .flat_map(|a| a.arguments().iter())
        .map(|a| a.location().start_offset())
        .collect();
    starts.extend(block_pass);
    starts
}

/// Upstream's `empty_range_for_starting_point`: the whitespace (`/\s/`) run
/// left of `at`; when it spans more than one line break, the line before
/// `at`'s line.
fn blank_run_line(file: &SourceFile, at: usize) -> Option<u32> {
    let mut start = at;
    while start > 0
        && matches!(
            file.bytes[start - 1],
            b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
        )
    {
        start -= 1;
    }
    let (first, last) = (file.line_of(start), file.line_of(at));
    (last > first + 1).then(|| last - 1)
}

#[cfg(test)]
mod tests;
