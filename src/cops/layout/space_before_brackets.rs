//! `Layout/SpaceBeforeBrackets` — no blank between a receiver and the `[`
//! of an index call.
//!
//! A port of RuboCop's cop (97f30c4, MIT); upstream's `Enabled: pending` is
//! enabled under the fixed defaults. `on_send` with `RESTRICT_ON_SEND = [],
//! []=` sees the parser gem's index sends without a dot: Prism's index
//! `CallNode` (`a[1]`, `a [1] = 2`, name `[]` or `[]=` with an opening `[`
//! and no call operator) and the index sends inside operator, `&&=` and
//! `||=` assignments (`a [1] += 2`) and multiple-assignment targets
//! (`a [1], b = 1, 2`), which the parser gem also builds as `[]`/`[]=`
//! sends. A dotted `a.[](1)`/`a&.[](1)` is skipped. Only an actual index is
//! checked: Ruby parses `foo [1]` as a call with an array argument unless
//! `foo` is a local variable (or another receiver such as `@a`, `a.b(1)`),
//! and Prism agrees. The gap from the end of the receiver to the `[` is
//! reported (`Remove the space before the opening brackets.`) at its start.
//!
//! The correction removes the gap when it is spaces and tabs: the `[` binds
//! to the receiver either way, and every receiver ends in a complete token.
//! A gap holding a line continuation keeps its finding without an edit,
//! since joining the lines would move every later line (`__LINE__`). Edits
//! are withheld for the listed source observers.
use ruby_prism::{Location, Node};

use super::space_after_colon::observed;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceBeforeBrackets;

const MSG: &str = "Remove the space before the opening brackets.";

/// The receiver and opening bracket of an undotted index send.
fn index<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Location<'pr>)> {
    let (receiver, operator, opening) = match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            if !matches!(call.name().as_slice(), b"[]" | b"[]=") {
                return None;
            }
            (
                call.receiver(),
                call.call_operator_loc(),
                call.opening_loc(),
            )
        }
        Node::IndexOperatorWriteNode { .. } => {
            let call = node.as_index_operator_write_node().unwrap();
            (
                call.receiver(),
                call.call_operator_loc(),
                Some(call.opening_loc()),
            )
        }
        Node::IndexAndWriteNode { .. } => {
            let call = node.as_index_and_write_node().unwrap();
            (
                call.receiver(),
                call.call_operator_loc(),
                Some(call.opening_loc()),
            )
        }
        Node::IndexOrWriteNode { .. } => {
            let call = node.as_index_or_write_node().unwrap();
            (
                call.receiver(),
                call.call_operator_loc(),
                Some(call.opening_loc()),
            )
        }
        Node::IndexTargetNode { .. } => {
            let call = node.as_index_target_node().unwrap();
            (Some(call.receiver()), None, Some(call.opening_loc()))
        }
        _ => return None,
    };
    if operator.is_some() {
        return None;
    }
    let opening = opening.filter(|opening| opening.as_slice() == b"[")?;
    Some((receiver?, opening))
}

impl Cop for SpaceBeforeBrackets {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeBrackets"
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
        let Some((receiver, opening)) = index(node) else {
            return;
        };
        let start = receiver.location().end_offset();
        let end = opening.start_offset();
        if start >= end {
            return;
        }
        let blank = file.bytes[start..end]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t'));
        let correction =
            (blank && cx.corrections_requested() && !observed(cx, file)).then(|| Correction {
                range: start..end,
                replacement: String::new(),
            });
        let (line, col) = file.line_col(start);
        let (end_line, _) = file.line_col(end);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line,
            col,
        });
    }
}

#[cfg(test)]
mod tests;
