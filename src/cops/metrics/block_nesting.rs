//! `Metrics/BlockNesting` — conditional and looping constructs nested more
//! than the fixed three levels deep.
//!
//! Upstream walks the whole tree from the root, adding a level for every
//! `case`, `case_match`, `if`, `while`, `while_post`, `until`, `until_post`,
//! `for` and `resbody` node, and reports the node whose level first exceeds
//! `Max: 3`; that node is then ignored together with its descendants. At the
//! pinned defaults `CountBlocks: false` and `CountModifierForms: false`:
//! blocks never count, and neither do modifier `if`/`unless` (`elsif` never
//! counts either). Ternaries and modifier `while`/`until` count. So a counting
//! node is reported exactly when three counting nodes enclose it; every
//! deeper one lies inside a reported node.
//!
//! Prism's tree differs in two places, both handled when counting the
//! enclosing nodes through `NodeContext`: a later `rescue` clause hangs off
//! the earlier one as its `subsequent` (in the parser AST the `resbody`
//! nodes are siblings), and a modifier `rescue` is a single node whose
//! rescued expression lies outside the parser's `resbody`.
//!
//! A directive that disables the reported node's range keeps its subtree
//! from being ignored (upstream's `ignore_node` runs in the `add_offense`
//! block, which is skipped for a disabled range), so the next counting node
//! below it is reported instead.
//!
//! Detection only: there is no correction.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// Pinned `Max`.
const MAX: usize = 3;

/// The `Metrics/BlockNesting` cop.
pub struct BlockNesting;

impl Cop for BlockNesting {
    fn name(&self) -> &'static str {
        "Metrics/BlockNesting"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !counts(node) {
            return;
        }
        let name = self.name();
        // Count the enclosing counting nodes. Upstream ignores the subtree
        // of a reported node, but only when its offense is enabled: the
        // `add_offense` block does not run on a directive-disabled range.
        // Without such a directive anything deeper than the limit sits inside
        // an ignored node, so the count can stop there.
        let mut enclosing = 0;
        let mut child = node;
        for ancestor in cx.ancestors() {
            if counts(ancestor) && encloses(ancestor, child) {
                enclosing += 1;
                if enclosing == MAX + 1 && file.cop_enabled_for_lines(name, 1, u32::MAX) {
                    return;
                }
            }
            child = ancestor;
        }
        if enclosing < MAX {
            return;
        }
        // Every enclosing node over the limit must have been reported on a
        // disabled range for this one to be reached.
        let mut over = enclosing - MAX;
        let mut child = node;
        for ancestor in cx.ancestors() {
            if over == 0 {
                break;
            }
            if counts(ancestor) && encloses(ancestor, child) {
                let (start, end) = range(ancestor);
                let first = file.line_of(start);
                let last = file.line_of(end.saturating_sub(1).max(start));
                if file.cop_enabled_for_lines(name, first, last) {
                    return;
                }
                over -= 1;
            }
            child = ancestor;
        }
        let (start, end) = range(node);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Avoid more than {MAX} levels of block nesting."),
            line,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            col,
        });
    }
}

/// Whether `node` is a parser `case`/`case_match`/`if`/loop/`resbody` node
/// that adds a level at the fixed defaults.
fn counts(node: &Node<'_>) -> bool {
    if let Some(node) = node.as_if_node() {
        // `elsif` never counts; a ternary has no keyword and counts; a
        // modifier (and a pattern guard, which Prism also shapes as an `if`)
        // has no `end` and does not count at `CountModifierForms: false`.
        return match node.if_keyword_loc() {
            None => true,
            Some(keyword) => keyword.as_slice() == b"if" && node.end_keyword_loc().is_some(),
        };
    }
    if let Some(node) = node.as_unless_node() {
        return node.end_keyword_loc().is_some();
    }
    matches!(
        node,
        Node::CaseNode { .. }
            | Node::CaseMatchNode { .. }
            | Node::WhileNode { .. }
            | Node::UntilNode { .. }
            | Node::ForNode { .. }
            | Node::RescueNode { .. }
            | Node::RescueModifierNode { .. }
    )
}

/// Whether the counting `ancestor` encloses its direct `child` in the parser
/// AST: a `rescue` clause does not enclose the next clause, and a modifier
/// `rescue`'s `resbody` holds only the expression after the keyword.
fn encloses(ancestor: &Node<'_>, child: &Node<'_>) -> bool {
    if let Some(rescue) = ancestor.as_rescue_node() {
        return rescue
            .subsequent()
            .is_none_or(|next| next.location().start_offset() != child.location().start_offset());
    }
    if let Some(rescue) = ancestor.as_rescue_modifier_node() {
        return child.location().start_offset() >= rescue.keyword_loc().start_offset();
    }
    true
}

/// The parser expression range of a counting node.
fn range(node: &Node<'_>) -> (usize, usize) {
    if let Some(rescue) = node.as_rescue_modifier_node() {
        // The `resbody`: the keyword through the rescuing expression.
        return (
            rescue.keyword_loc().start_offset(),
            rescue.rescue_expression().location().end_offset(),
        );
    }
    if node.as_rescue_node().is_some() {
        return (node.location().start_offset(), ast::source_end(node));
    }
    (node.location().start_offset(), node.location().end_offset())
}

#[cfg(test)]
mod tests;
