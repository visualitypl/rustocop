//! `Layout/SpaceAroundMethodCallOperator` — no blanks around a `.` or `&.`
//! method call operator, or after a `::`.
//!
//! A port of RuboCop's cop (97f30c4, MIT). `on_send`/`on_csend` check every
//! call whose operator is `.` or `&.` (never `::`): the gap between the end of
//! the receiver and the operator, and the gap between the operator and the
//! selector (the `(` of the `.()` shorthand). Prism's calls with a receiver
//! and a call operator are covered: `CallNode` and the receiver calls of
//! operator, `&&=` and `||=` assignments (`a .b += 1`) and of multiple
//! assignment targets (`a .b, c = 1`). `on_const` checks the gap after the
//! `::` of a constant path (`A:: B`, `:: A`), except the constant assigned by
//! a constant-path write, which is a `casgn` in RuboCop's AST. A gap is
//! reported (`Avoid using spaces around a method call operator.`) only when
//! it is one or more spaces and tabs: a gap holding a line break or a comment
//! is accepted.
//!
//! The correction removes the gap: only blanks between two tokens change,
//! and `.`, `&.` and `::` lex the same with or without them (a `::` after a
//! blank is read from the blank before it, which is kept). It is withheld
//! for the listed source observers.
use ruby_prism::{Location, Node};

use super::space_after_colon::observed;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAroundMethodCallOperator;

const MSG: &str = "Avoid using spaces around a method call operator.";

/// The gaps to check: `(start, end)` byte offsets.
fn gaps(node: &Node<'_>, parent: Option<&Node<'_>>) -> Vec<(usize, usize)> {
    let dotted =
        |receiver: Option<Node<'_>>, operator: Option<Location<'_>>, selector: Option<usize>| {
            let (Some(receiver), Some(operator), Some(selector)) = (receiver, operator, selector)
            else {
                return Vec::new();
            };
            if !matches!(operator.as_slice(), b"." | b"&.") {
                return Vec::new();
            }
            vec![
                (receiver.location().end_offset(), operator.start_offset()),
                (operator.end_offset(), selector),
            ]
        };
    match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            let selector = call
                .message_loc()
                .or_else(|| call.opening_loc())
                .map(|loc| loc.start_offset());
            dotted(call.receiver(), call.call_operator_loc(), selector)
        }
        Node::CallOperatorWriteNode { .. } => {
            let call = node.as_call_operator_write_node().unwrap();
            let selector = call.message_loc().map(|loc| loc.start_offset());
            dotted(call.receiver(), call.call_operator_loc(), selector)
        }
        Node::CallAndWriteNode { .. } => {
            let call = node.as_call_and_write_node().unwrap();
            let selector = call.message_loc().map(|loc| loc.start_offset());
            dotted(call.receiver(), call.call_operator_loc(), selector)
        }
        Node::CallOrWriteNode { .. } => {
            let call = node.as_call_or_write_node().unwrap();
            let selector = call.message_loc().map(|loc| loc.start_offset());
            dotted(call.receiver(), call.call_operator_loc(), selector)
        }
        Node::CallTargetNode { .. } => {
            let call = node.as_call_target_node().unwrap();
            dotted(
                Some(call.receiver()),
                Some(call.call_operator_loc()),
                Some(call.message_loc().start_offset()),
            )
        }
        Node::ConstantPathNode { .. } => {
            let target = parent.and_then(|parent| match parent {
                Node::ConstantPathWriteNode { .. } => {
                    Some(parent.as_constant_path_write_node().unwrap().target())
                }
                Node::ConstantPathOperatorWriteNode { .. } => Some(
                    parent
                        .as_constant_path_operator_write_node()
                        .unwrap()
                        .target(),
                ),
                Node::ConstantPathAndWriteNode { .. } => {
                    Some(parent.as_constant_path_and_write_node().unwrap().target())
                }
                Node::ConstantPathOrWriteNode { .. } => {
                    Some(parent.as_constant_path_or_write_node().unwrap().target())
                }
                _ => None,
            });
            let assigned = target.is_some_and(|target| {
                target.location().start_offset() == node.location().start_offset()
                    && target.location().end_offset() == node.location().end_offset()
            });
            if assigned {
                return Vec::new();
            }
            let path = node.as_constant_path_node().unwrap();
            vec![(
                path.delimiter_loc().end_offset(),
                path.name_loc().start_offset(),
            )]
        }
        _ => Vec::new(),
    }
}

impl Cop for SpaceAroundMethodCallOperator {
    fn name(&self) -> &'static str {
        "Layout/SpaceAroundMethodCallOperator"
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
        for (start, end) in gaps(node, cx.parent()) {
            if end <= start
                || !file.bytes[start..end]
                    .iter()
                    .all(|b| matches!(b, b' ' | b'\t'))
            {
                continue;
            }
            let correction =
                (cx.corrections_requested() && !observed(cx, file)).then(|| Correction {
                    range: start..end,
                    replacement: String::new(),
                });
            let (line, col) = file.line_col(start);
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

#[cfg(test)]
mod tests;
