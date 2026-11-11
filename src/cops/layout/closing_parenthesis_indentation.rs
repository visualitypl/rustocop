//! `Layout/ClosingParenthesisIndentation` — a hanging `)` of a call,
//! definition or grouped expression is indented by the fixed width 2 less
//! than the first element's line when that element follows a line break, is
//! aligned with `(` when every element is, and otherwise matches the first
//! element's line. Without elements any of three candidate columns is
//! accepted.
use super::element_indentation::{column, line_indentation};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::ops::Range;
pub struct ClosingParenthesisIndentation;
impl Cop for ClosingParenthesisIndentation {
    fn name(&self) -> &'static str {
        "Layout/ClosingParenthesisIndentation"
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
        let Some(target) = target(node) else { return };
        let right = target.right.start;
        if !super::alignment::starts_line(file, right) {
            return;
        }
        let actual = column(file, right);
        let left = column(file, target.left);
        let expected = match target.elements.first() {
            None => {
                // `correct_column_candidates`: any of them is accepted; the
                // first one is reported and used by the correction.
                let candidates = [
                    line_indentation(file, file.line_of(target.left)),
                    left,
                    column(file, target.start),
                ];
                if candidates.contains(&actual) {
                    return;
                }
                candidates[0]
            }
            Some(first) => {
                let line = file.line_of(first.location().start_offset());
                if line > file.line_of(target.left) {
                    line_indentation(file, line).saturating_sub(super::alignment::INDENT)
                } else if all_aligned(file, &target.elements) {
                    left
                } else {
                    line_indentation(file, line)
                }
            }
        };
        if expected == actual {
            return;
        }
        let message = if expected == left {
            "Align `)` with `(`.".to_owned()
        } else {
            format!("Indent `)` to column {expected} (not {actual})")
        };
        let alignment = super::alignment::Alignment {
            expression: target.right.clone(),
            delta: expected as i64 - actual as i64,
        };
        let (line, col) = file.line_col(right);
        out.push(Offense {
            // An interpolation's `}` closes code inside a literal: upstream
            // never shifts it inside strings; hold it for every literal kind.
            correction: (cx.corrections_requested() && !target.interpolation)
                .then(|| alignment.correction(file, cx.ancestors().next_back().unwrap_or(node)))
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: line,
        });
    }
}
/// A parenthesized parser node: `send`/`csend` (`on_send`), the `args` of a
/// `def`/`defs` (`on_def`) or a `begin` with delimiters (`on_begin`: grouped
/// and pinned expressions and interpolation).
struct Target<'pr> {
    left: usize,
    right: Range<usize>,
    /// The parser node's own start, the third no-element candidate.
    start: usize,
    elements: Vec<Node<'pr>>,
    interpolation: bool,
}
fn target<'pr>(node: &Node<'pr>) -> Option<Target<'pr>> {
    let range = |l: ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
    let statements = |body: Option<Node<'pr>>| -> Vec<Node<'pr>> {
        match body {
            Some(body) => match body.as_statements_node() {
                Some(statements) => statements.body().iter().collect(),
                None => vec![body],
            },
            None => Vec::new(),
        }
    };
    let (left, right, start, elements, interpolation) = match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node()?;
            let open = call.opening_loc()?;
            if open.as_slice() != b"(" {
                return None;
            }
            // `not(x)` has a receiver and no arguments, like upstream's `!`.
            let mut elements: Vec<_> = call
                .arguments()
                .map(|a| a.arguments().iter().collect())
                .unwrap_or_default();
            elements.extend(
                call.block()
                    .filter(|b| b.as_block_argument_node().is_some()),
            );
            let start = call.location().start_offset();
            (
                open.start_offset(),
                call.closing_loc()?,
                start,
                elements,
                false,
            )
        }
        Node::DefNode { .. } => {
            let def = node.as_def_node()?;
            let open = def.lparen_loc()?.start_offset();
            let elements = super::parameter_alignment::parameters(node).unwrap_or_default();
            (open, def.rparen_loc()?, open, elements, false)
        }
        Node::ParenthesesNode { .. } => {
            let parens = node.as_parentheses_node()?;
            let open = parens.opening_loc().start_offset();
            let elements = statements(parens.body());
            (open, parens.closing_loc(), open, elements, false)
        }
        Node::PinnedExpressionNode { .. } => {
            let pin = node.as_pinned_expression_node()?;
            let open = pin.lparen_loc().start_offset();
            (open, pin.rparen_loc(), open, vec![pin.expression()], false)
        }
        Node::EmbeddedStatementsNode { .. } => {
            let embedded = node.as_embedded_statements_node()?;
            let open = embedded.opening_loc().start_offset();
            let elements = statements(embedded.statements().map(|s| s.as_node()));
            (open, embedded.closing_loc(), open, elements, true)
        }
        _ => return None,
    };
    Some(Target {
        left,
        right: range(right),
        start,
        elements,
        interpolation,
    })
}
/// `all_elements_aligned?`: a leading hash contributes its children's columns
/// instead of every element's.
fn all_aligned(file: &SourceFile, elements: &[Node<'_>]) -> bool {
    let starts: Vec<usize> = if let Some(hash) = elements[0].as_hash_node() {
        hash.elements()
            .iter()
            .map(|c| c.location().start_offset())
            .collect()
    } else if let Some(hash) = elements[0].as_keyword_hash_node() {
        hash.elements()
            .iter()
            .map(|c| c.location().start_offset())
            .collect()
    } else {
        elements
            .iter()
            .map(|c| c.location().start_offset())
            .collect()
    };
    let mut columns = starts.into_iter().map(|start| column(file, start));
    columns
        .next()
        .is_some_and(|first| columns.all(|c| c == first))
}
#[cfg(test)]
mod tests;
