//! `Layout/EmptyLineAfterMultilineCondition` (opt-in) — an empty line follows
//! a multi-line condition.
//!
//! Port of upstream's callbacks over the parser AST that RuboCop builds from
//! Prism: `if`/`unless`/`elsif` (never a ternary or a pattern guard) and
//! `while`/`until` check their condition; a modifier `if`/`unless`/`while`/
//! `until` and a `begin ... end while`/`until` only when another statement
//! follows in the same statement list (a parser `begin`/`kwbegin` with a right
//! sibling). Each `when` whose first and last conditions span lines, and each
//! `rescue` clause with several exceptions spanning lines, is checked after
//! its last condition or exception. A multi-line condition is reported (at
//! the condition, the `when` or the `rescue`) unless the next line is blank
//! or the body starts on the condition's last line (`then`/`do` bodies).
//!
//! The correction is upstream's line break after the condition's last line,
//! with that line's own ending. It is withheld, finding kept, for the
//! listed source observers (the shared position barrier holds a later
//! `__LINE__`), and when the insertion point is inside a literal, ends a line
//! that a heredoc body continues, or follows a backslash.
//!
//! The cop is opt-in as upstream (`Enabled: false`): only its exact name in
//! `--only` or its own `enabled = true` enables it, never a department
//! selection, so the accepted `Layout` compositions keep their results.
use ruby_prism::{Location, Node, StatementsNode};

use super::empty_line_after_guard_clause::{insertion_after_line, next_line_empty};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct EmptyLineAfterMultilineCondition;

const MSG: &str = "Use empty line after multiline condition.";

impl Cop for EmptyLineAfterMultilineCondition {
    fn name(&self) -> &'static str {
        "Layout/EmptyLineAfterMultilineCondition"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    /// Opt-in as upstream: a department selection does not enable it.
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
        let mut report = |at: usize, last_line: u32| {
            let root = cx.ancestors().last().unwrap_or(node);
            let (line, col) = file.line_col(at);
            out.push(Offense {
                correction: cx
                    .corrections_requested()
                    .then(|| insertion_after_line(file, root, last_line, false))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: MSG.into(),
                line,
                end_line: last_line,
                col,
            });
        };
        match node {
            // Modifier forms and `begin ... end while` (`while_post`) are
            // checked only when another statement follows them in the same
            // list (parser `begin`/`kwbegin` with a right sibling): each list
            // is visited once, keeping long lists linear.
            Node::StatementsNode { .. } => {
                let statements = node.as_statements_node().unwrap();
                let mut body = statements.body().iter().peekable();
                while let Some(current) = body.next() {
                    if body.peek().is_none() {
                        break;
                    }
                    if let Some((predicate, _, true)) = conditional(&current)
                        && let Some(last) = offending(file, predicate.location(), None)
                    {
                        report(predicate.location().start_offset(), last);
                    }
                }
            }
            Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::WhileNode { .. }
            | Node::UntilNode { .. } => {
                if cx.parent().is_some_and(|p| p.as_in_node().is_some()) {
                    return; // a pattern guard
                }
                if let Some((predicate, statements, false)) = conditional(node)
                    && let Some(last) = offending(file, predicate.location(), statements.as_ref())
                {
                    report(predicate.location().start_offset(), last);
                }
            }
            Node::WhenNode { .. } => {
                let when = node.as_when_node().unwrap();
                let conditions: Vec<Node<'pr>> = when.conditions().iter().collect();
                let (Some(first), Some(last)) = (conditions.first(), conditions.last()) else {
                    return;
                };
                let last_line = file.last_line_of(last.location());
                if file.line_of(first.location().start_offset()) == last_line {
                    return;
                }
                if !next_line_empty(file, last_line)
                    && !body_on_line(file, when.statements().as_ref(), last_line)
                {
                    report(when.keyword_loc().start_offset(), last_line);
                }
            }
            Node::RescueNode { .. } => {
                let rescue = node.as_rescue_node().unwrap();
                let exceptions: Vec<Node<'pr>> = rescue.exceptions().iter().collect();
                let [first, .., last] = exceptions.as_slice() else {
                    return;
                };
                let last_line = file.last_line_of(last.location());
                if file.line_of(first.location().start_offset()) == last_line {
                    return;
                }
                if !next_line_empty(file, last_line)
                    && !body_on_line(file, rescue.statements().as_ref(), last_line)
                {
                    report(rescue.keyword_loc().start_offset(), last_line);
                }
            }
            _ => {}
        }
    }
}

/// The predicate, the body and whether it is a modifier (or a `begin ... end
/// while`), for a parser `if` that is not a ternary or a `while`/`until`.
fn conditional<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Option<StatementsNode<'pr>>, bool)> {
    if let Some(n) = node.as_if_node() {
        let keyword = n.if_keyword_loc()?; // a ternary has none
        let modifier = keyword.as_slice() == b"if" && n.end_keyword_loc().is_none();
        Some((n.predicate(), n.statements(), modifier))
    } else if let Some(n) = node.as_unless_node() {
        Some((n.predicate(), n.statements(), n.end_keyword_loc().is_none()))
    } else if let Some(n) = node.as_while_node() {
        Some((n.predicate(), n.statements(), n.closing_loc().is_none()))
    } else {
        let n = node.as_until_node()?;
        Some((n.predicate(), n.statements(), n.closing_loc().is_none()))
    }
}

/// `check_condition`: the last line of a multi-line condition that is neither
/// followed by a blank line nor by a body on that line.
fn offending(
    file: &SourceFile,
    condition: Location<'_>,
    body: Option<&StatementsNode<'_>>,
) -> Option<u32> {
    let first_line = file.line_of(condition.start_offset());
    let last_line = file.last_line_of(condition);
    (first_line != last_line
        && !next_line_empty(file, last_line)
        && !body_on_line(file, body, last_line))
    .then_some(last_line)
}

/// `body&.first_line == line`.
fn body_on_line(file: &SourceFile, body: Option<&StatementsNode<'_>>, line: u32) -> bool {
    body.and_then(|b| b.body().iter().next())
        .is_some_and(|first| file.line_of(first.location().start_offset()) == line)
}

#[cfg(test)]
mod tests;
