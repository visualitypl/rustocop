//! `Layout/SpaceAfterNot` — no space between a prefix `!` and its operand.
//!
//! A port of RuboCop's cop (97f30c4, MIT). A `!` send whose selector is
//! `!` (never the `not` keyword) is reported when its receiver starts more
//! than one byte after the `!` (any whitespace, a line break or a
//! continuation between them), unless the receiver is a unary `~` send
//! (`! ~x`, where removing the gap would read `!~`). The whole send is
//! highlighted (`Do not leave space between `!` and its argument.`), from
//! the `!` to the end of the receiver.
//!
//! The correction removes the gap between the `!` and the receiver: only
//! blanks between two tokens change, as `!` followed by any token is the
//! same unary operator. It is withheld when the gap holds anything but
//! whitespace and backslash-newline continuations (a comment or an
//! embedded document would be deleted), defensively when the receiver's
//! first byte is `~` or `=` (which would form `!~` or `!=`; valid Ruby only
//! reaches them through the exempt unary `~`), and for the
//! listed source observers.
use ruby_prism::Node;

use super::space_after_colon::{observed, ruby_space};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAfterNot;

const MSG: &str = "Do not leave space between `!` and its argument.";

/// Whether `node` is a unary `~` send (`~x`, not `x.~`).
fn unary_tilde(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.name().as_slice() == b"~"
            && !call.is_safe_navigation()
            && call
                .message_loc()
                .is_some_and(|selector| selector.start_offset() == node.location().start_offset())
    })
}

/// Whether the gap holds only whitespace and backslash-newline continuations.
fn blank_gap(gap: &[u8]) -> bool {
    let mut bytes = gap.iter().peekable();
    while let Some(&byte) = bytes.next() {
        if ruby_space(byte) {
            continue;
        }
        if byte != b'\\' {
            return false;
        }
        if bytes.peek() == Some(&&b'\r') {
            bytes.next();
        }
        if bytes.next() != Some(&b'\n') {
            return false;
        }
    }
    true
}

impl Cop for SpaceAfterNot {
    fn name(&self) -> &'static str {
        "Layout/SpaceAfterNot"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"!" {
            return;
        }
        let (Some(receiver), Some(selector)) = (call.receiver(), call.message_loc()) else {
            return;
        };
        if selector.as_slice() != b"!" {
            return;
        }
        let start = node.location().start_offset();
        let operand = receiver.location().start_offset();
        if operand <= start + 1 || unary_tilde(&receiver) {
            return;
        }
        let gap = selector.end_offset()..operand;
        let correction = (cx.corrections_requested()
            && blank_gap(&file.bytes[gap.clone()])
            && !matches!(file.bytes[operand], b'~' | b'=')
            && !observed(cx, file))
        .then(|| Correction {
            range: gap,
            replacement: String::new(),
        });
        let end = ast::source_end(node).max(start + 1);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line: file.line_of(end - 1),
            col,
        });
    }
}

#[cfg(test)]
mod tests;
