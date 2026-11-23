//! `Layout/MultilineOperationIndentation` — fixed `aligned` style.
//!
//! The right operand of a binary operator `send` (no dot, not unary, never
//! `[]`) or of `&&`/`||`/`and`/`or` that begins its line is aligned with the
//! start of the operation inside a keyword expression, an assignment value
//! (or when that value begins its line) and a method argument (not of a
//! `private def` modifier); elsewhere it is indented one step (two in a
//! prefix keyword expression) from its left operand's line. Reuses the
//! `MultilineExpressionIndentation` port landed with
//! `Layout/MultilineMethodCallIndentation`.
use super::multiline_expression::{
    Parens, Span, argument_in_method_call, arguments, column, correct_indentation, dot,
    keyword_node, line_indentation, method_name, node_span, not_for_this_cop,
    operation_description, parser_ancestors, parser_span, part_of_assignment_rhs, receiver,
    selector,
};
use crate::{
    cops::{
        Cop, NodeContext,
        layout::alignment::{Alignment, starts_line},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MultilineOperationIndentation;

impl Cop for MultilineOperationIndentation {
    fn name(&self) -> &'static str {
        "Layout/MultilineOperationIndentation"
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
        let Some((lhs, rhs)) = operands(node) else {
            return;
        };
        if !starts_line(file, rhs.start) {
            return;
        }
        let prism: Vec<&Node<'pr>> = cx.ancestors().collect();
        let ancestors = parser_ancestors(node, &prism);
        let span = parser_span(node);
        if not_for_this_cop(&span, &ancestors) {
            return;
        }
        let align = should_align(&span, &rhs, &ancestors, file);
        let correct = if align {
            column(file, span.start)
        } else {
            line_indentation(file, lhs.start) + correct_indentation(&span, &ancestors)
        };
        let delta = correct as i64 - column(file, rhs.start) as i64;
        if delta == 0 {
            return;
        }
        let what = operation_description(&span, &rhs, &ancestors);
        let message = if align {
            format!("Align the operands of {what} spanning multiple lines.")
        } else {
            format!(
                "Use {} (not {}) spaces for indenting {what} spanning multiple lines.",
                correct_indentation(&span, &ancestors),
                column(file, rhs.start) as i64 - line_indentation(file, lhs.start) as i64,
            )
        };
        let (line, col) = file.line_col(rhs.start);
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = prism.last().copied().unwrap_or(node);
                Alignment {
                    expression: rhs.clone(),
                    delta,
                }
                .correction(file, root)
            })
            .flatten();
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(rhs.end.saturating_sub(1).max(rhs.start)),
        });
    }
}

/// The left-hand side and right operand of an `and`/`or`, or of a relevant
/// operator `send` (`on_send` with `relevant_node?`).
fn operands(node: &Node<'_>) -> Option<(Span, Span)> {
    if let Some(and) = node.as_and_node() {
        return Some((node_span(&and.left()), node_span(&and.right())));
    }
    if let Some(or) = node.as_or_node() {
        return Some((node_span(&or.left()), node_span(&or.right())));
    }
    // Index op-assignments contain a `send :[]`, which is never checked.
    node.as_call_node()?;
    let receiver = receiver(node)?;
    if method_name(node) == b"[]" || dot(node).is_some() || unary_operation(node) {
        return None;
    }
    let first = arguments(node).into_iter().next()?;
    Some((node_span(&receiver), node_span(&first)))
}

/// `unary_operation?`: an operator method whose selector starts the node.
fn unary_operation(node: &Node<'_>) -> bool {
    selector(node).is_some_and(|s| {
        crate::cops::ast::is_operator_method(method_name(node))
            && s.start_offset() == node.location().start_offset()
    })
}

/// `should_align?` for the fixed `aligned` style.
fn should_align(
    span: &Span,
    rhs: &Span,
    ancestors: &[super::multiline_expression::Anc<'_, '_>],
    file: &SourceFile,
) -> bool {
    let assignment = part_of_assignment_rhs(Some(rhs), ancestors);
    if let Some(value) = &assignment
        && starts_line(file, value.location().start_offset())
    {
        return true;
    }
    if keyword_node(span, ancestors).is_some() || assignment.is_some() {
        return true;
    }
    argument_in_method_call(span, Parens::Any, ancestors).is_some_and(|call| !def_modifier(call))
}

/// `def_modifier?`: a receiverless `send` whose first argument is a `def`
/// or (recursively) such a modifier.
fn def_modifier(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.receiver().is_some() || call.is_safe_navigation() || call.call_operator_loc().is_some()
    {
        return false;
    }
    match arguments(node).into_iter().next() {
        Some(Node::DefNode { .. }) => true,
        Some(argument) => def_modifier(&argument),
        None => false,
    }
}

#[cfg(test)]
mod tests;
