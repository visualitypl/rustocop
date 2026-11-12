//! `Layout/SingleLineBlockChain` — a method called on a single-line block
//! starts a line of its own (opt-in).
//!
//! A port of RuboCop's `on_send`/`on_csend` (97f30c4, MIT): a dotted
//! (`.`, `&.` or `::`) call whose receiver is a parser block (a call or
//! `super` with a literal block, or a `->` lambda; written, numbered or `it`
//! parameters) with `loc.begin` and `loc.end` on one line, where the dot is
//! not on a later line than `loc.end` and lies left of the selector (`loc.selector`,
//! or `loc.begin` for the implicit `.()` call). The dot through the selector
//! is reported. Calls in operator, `||=`/`&&=` assignments and multiple
//! assignment targets are parser `send`s too. Nothing is reported when a
//! heredoc in the block (its call and receivers included) is opened on the
//! dot's line (`heredoc_opened_before?`).
//!
//! The correction is upstream's line break inserted before the dot (`\r\n`
//! on a CRLF line, where upstream inserts a bare `\n`). It is
//! withheld for the listed source observers, for a `::` call (a
//! line starting with `::` is a new top-level constant reference), when a
//! heredoc opened earlier on the dot's line outside the block would take
//! the rest of the line into its body, and when the line holds a
//! `rubocop:`/`rustocop:` directive. `__LINE__`, `DATA` and heredoc
//! boundaries are held by the shared `run_cops` guards. Upstream's
//! `autocorrect_incompatible_with` (`Style/FloatDivision`,
//! `Style/MapToHash`, `Style/NilComparison`, `Style/RedundantSort`) only
//! orders RuboCop's correction rounds and is not declared, as decided for
//! `Layout/ExtraSpacing`: all four are default-on (three of them unsafe,
//! never editing), and a declaration would withhold every edit while they
//! run; a peer edit of the chained call contains the insertion point, so one
//! `--fix` pass applies one of the two.
use std::ops::Range;

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        style::correction::touches_directive,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SingleLineBlockChain;

const MSG: &str = "Put method call on a separate line if chained to a single line block.";

/// The receiver, dot and selector (or `(` of `.()`) of a parser `send`.
struct Send<'pr> {
    receiver: Node<'pr>,
    dot: Range<usize>,
    selector: Range<usize>,
}

fn range(location: &ruby_prism::Location<'_>) -> Range<usize> {
    location.start_offset()..location.end_offset()
}

fn send<'pr>(node: &Node<'pr>) -> Option<Send<'pr>> {
    let (receiver, dot, selector) = match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node()?;
            let selector = call.message_loc().or_else(|| call.opening_loc())?;
            (call.receiver()?, call.call_operator_loc()?, selector)
        }
        Node::CallOperatorWriteNode { .. } => {
            let call = node.as_call_operator_write_node()?;
            (
                call.receiver()?,
                call.call_operator_loc()?,
                call.message_loc()?,
            )
        }
        Node::CallOrWriteNode { .. } => {
            let call = node.as_call_or_write_node()?;
            (
                call.receiver()?,
                call.call_operator_loc()?,
                call.message_loc()?,
            )
        }
        Node::CallAndWriteNode { .. } => {
            let call = node.as_call_and_write_node()?;
            (
                call.receiver()?,
                call.call_operator_loc()?,
                call.message_loc()?,
            )
        }
        Node::CallTargetNode { .. } => {
            let call = node.as_call_target_node()?;
            (
                call.receiver(),
                call.call_operator_loc(),
                call.message_loc(),
            )
        }
        _ => return None,
    };
    Some(Send {
        receiver,
        dot: range(&dot),
        selector: range(&selector),
    })
}

/// The parser block a receiver stands for: a call or `super` with a literal
/// block, or a lambda. `(start, opening, closing)` offsets.
fn receiver_block(receiver: &Node<'_>) -> Option<(Range<usize>, Range<usize>)> {
    let block = match receiver {
        Node::CallNode { .. } => receiver.as_call_node()?.block()?,
        Node::SuperNode { .. } => receiver.as_super_node()?.block()?,
        Node::ForwardingSuperNode { .. } => receiver.as_forwarding_super_node()?.block()?.as_node(),
        Node::LambdaNode { .. } => {
            let lambda = receiver.as_lambda_node()?;
            return Some((range(&lambda.opening_loc()), range(&lambda.closing_loc())));
        }
        _ => return None,
    };
    let block = block.as_block_node()?;
    Some((range(&block.opening_loc()), range(&block.closing_loc())))
}

/// Heredoc openers of a subtree.
struct Openers(Vec<usize>);

impl<'pr> NodeVisitor<'pr> for Openers {
    fn enter(&mut self, node: Node<'pr>) {
        let opening = match &node {
            Node::StringNode { .. } => node.as_string_node().and_then(|n| n.opening_loc()),
            Node::InterpolatedStringNode { .. } => node
                .as_interpolated_string_node()
                .and_then(|n| n.opening_loc()),
            Node::XStringNode { .. } => node.as_x_string_node().map(|n| n.opening_loc()),
            Node::InterpolatedXStringNode { .. } => node
                .as_interpolated_x_string_node()
                .map(|n| n.opening_loc()),
            _ => None,
        };
        if let Some(opening) = opening
            && opening.as_slice().starts_with(b"<<")
        {
            self.0.push(opening.start_offset());
        }
    }
}

impl Cop for SingleLineBlockChain {
    fn name(&self) -> &'static str {
        "Layout/SingleLineBlockChain"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn default_enabled(&self) -> bool {
        false
    }

    /// Opt-in as upstream: only its exact name enables it.
    fn department_selection_enables(&self) -> bool {
        false
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(send) = send(node) else {
            return;
        };
        let Some((opening, closing)) = receiver_block(&send.receiver) else {
            return;
        };
        let closing_line = file.line_of(closing.start);
        if file.line_of(opening.start) < closing_line {
            return;
        }
        // `call_method_after_block?`.
        let (dot_line, dot_col) = file.line_col(send.dot.start);
        if dot_line > closing_line || dot_col >= file.line_col(send.selector.start).1 {
            return;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        // `heredoc_opened_before?`: a heredoc of the block opened on the
        // dot's line.
        if file.contains_heredoc(root, &send.receiver) {
            let mut openers = Openers(Vec::new());
            walk(&mut openers, &send.receiver);
            if openers.0.iter().any(|&o| file.line_of(o) == dot_line) {
                return;
            }
        }
        let correction = cx
            .corrections_requested()
            .then(|| correction(&send, root, file))
            .flatten();
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.to_owned(),
            line: dot_line,
            end_line: dot_line,
            col: dot_col,
        });
    }
}

fn correction(send: &Send<'_>, root: &Node<'_>, file: &SourceFile) -> Option<Correction> {
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
        || file.bytes[send.dot.clone()] == *b"::"
        || file.heredoc_pending_at(root, send.dot.start)
        || touches_directive(file, send.dot.clone())
    {
        return None;
    }
    Some(Correction {
        range: send.dot.start..send.dot.start,
        replacement: super::parser_block::line_break(file, send.dot.start).to_owned(),
    })
}

#[cfg(test)]
mod tests;
