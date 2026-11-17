//! `Layout/ConditionPosition` — the condition of a non-modifier `if`,
//! `unless`, `elsif`, `while` or `until` starts on the keyword's line.
//!
//! Port of RuboCop's `on_if` (never a ternary), `on_while` and `on_until`:
//! modifier forms (including `begin ... end while`) and pattern guards (not
//! parser `if` nodes) are skipped.
use std::ops::Range;

use ruby_prism::{Location, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        naming::heredoc::Heredoc,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct ConditionPosition;

impl Cop for ConditionPosition {
    fn name(&self) -> &'static str {
        "Layout/ConditionPosition"
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
        let Some(conditional) = Conditional::of(node) else {
            return;
        };
        let keyword_line = file.line_of(conditional.keyword.start_offset());
        let condition = conditional.condition.location();
        // `single_line_condition?`
        if file.line_of(condition.start_offset()) == keyword_line {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| correction(&conditional, file))
            .flatten();
        let (line, col) = file.line_col(condition.start_offset());
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Place the condition on the same line as `{}`.",
                String::from_utf8_lossy(conditional.keyword.as_slice())
            ),
            line,
            end_line: file.last_line_of(condition),
            col,
        });
    }
}

/// A non-modifier, non-ternary parser `if`/`while`/`until` node.
struct Conditional<'pr> {
    keyword: Location<'pr>,
    condition: Node<'pr>,
    /// Upstream's normalized `body`: the statements run when the condition
    /// holds (`unless` included), or the loop body.
    body: Option<Location<'pr>>,
}

impl<'pr> Conditional<'pr> {
    fn of(node: &Node<'pr>) -> Option<Self> {
        let (keyword, condition, statements, modifier) = match node {
            Node::IfNode { .. } => {
                let branch = node.as_if_node()?;
                // A ternary has no keyword.
                let keyword = branch.if_keyword_loc()?;
                let statements = branch.statements();
                // Modifier forms and pattern guards put the statements first.
                let modifier = statements
                    .as_ref()
                    .is_some_and(|s| s.location().start_offset() < keyword.start_offset());
                (
                    keyword,
                    branch.predicate(),
                    statements.map(|s| s.location()),
                    modifier,
                )
            }
            Node::UnlessNode { .. } => {
                let branch = node.as_unless_node()?;
                let keyword = branch.keyword_loc();
                let statements = branch.statements();
                let modifier = statements
                    .as_ref()
                    .is_some_and(|s| s.location().start_offset() < keyword.start_offset());
                (
                    keyword,
                    branch.predicate(),
                    statements.map(|s| s.location()),
                    modifier,
                )
            }
            Node::WhileNode { .. } => {
                let loop_node = node.as_while_node()?;
                (
                    loop_node.keyword_loc(),
                    loop_node.predicate(),
                    loop_node.statements().map(|s| s.location()),
                    loop_node.closing_loc().is_none(),
                )
            }
            Node::UntilNode { .. } => {
                let loop_node = node.as_until_node()?;
                (
                    loop_node.keyword_loc(),
                    loop_node.predicate(),
                    loop_node.statements().map(|s| s.location()),
                    loop_node.closing_loc().is_none(),
                )
            }
            _ => return None,
        };
        (!modifier).then_some(Self {
            keyword,
            condition,
            body: statements,
        })
    }
}

/// Upstream's two actions as one atomic edit from the keyword's end: insert
/// `" #{condition.source}"` after the keyword and remove the condition's whole
/// lines, or (with a body statement on its last line) the condition up to that
/// statement. Withheld where those actions would drop a comment or another
/// token, join lines through a continuation, or move a heredoc opener away
/// from its body.
fn correction(conditional: &Conditional<'_>, file: &SourceFile) -> Option<Correction> {
    let bytes = &file.bytes;
    let keyword_end = conditional.keyword.end_offset();
    let condition = conditional.condition.location();
    let (start, end) = (condition.start_offset(), condition.end_offset());
    let last_line = file.line_of(end);
    // Between the keyword and the condition only whitespace and comments,
    // which stay in place; a backslash continuation would join the moved
    // condition's line with the next one.
    if bytes[keyword_end..start].contains(&b'\\') || contains_heredoc(&conditional.condition, file)
    {
        return None;
    }
    let removal: Range<usize> = match &conditional.body {
        Some(body) if file.line_of(body.start_offset()) == last_line => {
            let separator = &bytes[end..body.start_offset()];
            if !separator_only(separator) {
                return None;
            }
            start..body.start_offset()
        }
        _ => {
            let first = file.line_start(file.line_of(start));
            let line_end = file.line_start(last_line + 1);
            let before = &bytes[first..start];
            let after = &bytes[end..line_end];
            if !before.iter().all(|b| matches!(b, b' ' | b'\t')) || !separator_only(after) {
                return None;
            }
            first..line_end
        }
    };
    let mut replacement = b" ".to_vec();
    replacement.extend_from_slice(&bytes[start..end]);
    replacement.extend_from_slice(&bytes[keyword_end..removal.start]);
    Some(Correction {
        range: keyword_end..removal.end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

/// Whitespace with at most one `;`, `then` or `do` token: what may follow the
/// removed condition on its line without being dropped.
fn separator_only(text: &[u8]) -> bool {
    let trimmed = text.trim_ascii();
    matches!(trimmed, b"" | b";" | b"then" | b"do")
}

/// Whether a heredoc opens inside the condition.
fn contains_heredoc(condition: &Node<'_>, file: &SourceFile) -> bool {
    struct Finder<'f> {
        file: &'f SourceFile,
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Finder<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            self.found |= Heredoc::of(&node, self.file).is_some();
        }
    }
    let mut finder = Finder { file, found: false };
    walk(&mut finder, condition);
    finder.found
}

#[cfg(test)]
mod tests;
