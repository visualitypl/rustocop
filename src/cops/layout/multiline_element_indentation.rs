//! Port of RuboCop's `MultilineElementIndentation` mixin at the fixed
//! `special_inside_parentheses` style, shared by
//! `FirstArrayElementIndentation` and `FirstHashElementIndentation`.
//!
//! Upstream's `on_send` walks each argument of a parenthesized call with
//! `on_node(type, arg, :send)` (descending into everything but `send` nodes)
//! and claims a literal whose opening shares the `(` line; the claimed literal
//! is then ignored by its own `on_array`/`on_hash`. Here the same parser
//! lineage is followed upward through `NodeContext` ancestors, never by
//! rescanning the tree: the outermost claiming call wins, as in upstream's
//! pre-order dispatch.
use super::element_indentation::{column, line_indentation};
use crate::source::SourceFile;
use ruby_prism::Node;

/// Where the expected column comes from (`indent_base_type`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Base {
    ParentHashKey,
    AfterLeftParenthesis,
    StartOfLine,
}

/// `indent_base`: the column a literal's first element is indented from (and
/// its closing delimiter aligned with). `has_first` is whether the literal has
/// a first element (array value or hash pair); `ancestors` are the literal's
/// Prism ancestors, innermost first.
pub(super) fn indent_base(
    file: &SourceFile,
    left: usize,
    has_first: bool,
    ancestors: &[&Node<'_>],
    paren: Option<usize>,
) -> (usize, Base) {
    if has_first && let Some(pair) = pair_anchor(file, ancestors) {
        return (pair, Base::ParentHashKey);
    }
    if let Some(paren) = paren {
        return (column(file, paren) + 1, Base::AfterLeftParenthesis);
    }
    (
        line_indentation(file, file.line_of(left)),
        Base::StartOfLine,
    )
}

/// `hash_pair_where_value_beginning_with` plus its two conditions: the
/// literal is a key or value of a pair whose key and value start on one line
/// and whose next sibling starts on a later line than the pair ends.
fn pair_anchor(file: &SourceFile, ancestors: &[&Node<'_>]) -> Option<usize> {
    let pair = ancestors.first()?.as_assoc_node()?;
    let same_line = file.line_of(pair.key().location().start_offset())
        == file.line_of(pair.value().location().start_offset());
    if !same_line {
        return None;
    }
    let hash = ancestors.get(1)?;
    let elements: Vec<Node<'_>> = if let Some(h) = hash.as_hash_node() {
        h.elements().iter().collect()
    } else {
        hash.as_keyword_hash_node()?.elements().iter().collect()
    };
    let index = elements
        .iter()
        .position(|e| same(&e.location(), &pair.location()))?;
    let sibling = elements.get(index + 1)?;
    (file.last_line_of(pair.location()) < file.line_of(sibling.location().start_offset()))
        .then(|| column(file, pair.location().start_offset()))
}

/// The `(` of the outermost parenthesized `send`/`csend` whose argument
/// subtree reaches the literal without crossing a parser `send`, when the
/// literal's opening `left` is on that `(` line.
pub(super) fn left_parenthesis(
    file: &SourceFile,
    literal: &Node<'_>,
    left: usize,
    ancestors: &[&Node<'_>],
) -> Option<usize> {
    let mut found = None;
    let mut child = literal.location();
    for (i, parent) in ancestors.iter().enumerate() {
        match parent {
            Node::CallNode { .. } => {
                let call = parent.as_call_node()?;
                // `=~` with a named-capture regexp is `match_with_lvasgn`.
                let send = !call.is_safe_navigation()
                    && !ancestors
                        .get(i + 1)
                        .is_some_and(|p| p.as_match_write_node().is_some());
                let through_block = call
                    .block()
                    .is_some_and(|b| b.as_block_node().is_some() && same(&b.location(), &child));
                let argument = call
                    .arguments()
                    .is_some_and(|a| same(&a.as_node().location(), &child))
                    || call.block().is_some_and(|b| {
                        b.as_block_argument_node().is_some() && same(&b.location(), &child)
                    });
                if argument
                    && let Some(open) = call.opening_loc()
                    && open.as_slice() == b"("
                    && file.line_of(open.start_offset()) == file.line_of(left)
                {
                    found = Some(open.start_offset());
                }
                if send && !through_block {
                    break;
                }
            }
            // The value is a child of `op_asgn`; anything else is inside the
            // (index) send.
            Node::CallOperatorWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::CallTargetNode { .. }
            | Node::IndexTargetNode { .. }
                if !write_value(parent).is_some_and(|v| same(&v.location(), &child))
                    && !safe_navigation(parent) =>
            {
                break;
            }
            _ => {}
        }
        child = parent.location();
    }
    found
}

fn write_value<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    match node {
        Node::CallOperatorWriteNode { .. } => node.as_call_operator_write_node().map(|n| n.value()),
        Node::CallAndWriteNode { .. } => node.as_call_and_write_node().map(|n| n.value()),
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node().map(|n| n.value()),
        Node::IndexOperatorWriteNode { .. } => {
            node.as_index_operator_write_node().map(|n| n.value())
        }
        Node::IndexAndWriteNode { .. } => node.as_index_and_write_node().map(|n| n.value()),
        Node::IndexOrWriteNode { .. } => node.as_index_or_write_node().map(|n| n.value()),
        _ => None,
    }
}

fn safe_navigation(node: &Node<'_>) -> bool {
    match node {
        Node::CallOperatorWriteNode { .. } => node
            .as_call_operator_write_node()
            .is_some_and(|n| n.is_safe_navigation()),
        Node::CallAndWriteNode { .. } => node
            .as_call_and_write_node()
            .is_some_and(|n| n.is_safe_navigation()),
        Node::CallOrWriteNode { .. } => node
            .as_call_or_write_node()
            .is_some_and(|n| n.is_safe_navigation()),
        Node::IndexOperatorWriteNode { .. } => node
            .as_index_operator_write_node()
            .is_some_and(|n| n.is_safe_navigation()),
        Node::IndexAndWriteNode { .. } => node
            .as_index_and_write_node()
            .is_some_and(|n| n.is_safe_navigation()),
        Node::IndexOrWriteNode { .. } => node
            .as_index_or_write_node()
            .is_some_and(|n| n.is_safe_navigation()),
        Node::CallTargetNode { .. } => node
            .as_call_target_node()
            .is_some_and(|n| n.is_safe_navigation()),
        Node::IndexTargetNode { .. } => node
            .as_index_target_node()
            .is_some_and(|n| n.is_safe_navigation()),
        _ => false,
    }
}

fn same(a: &ruby_prism::Location<'_>, b: &ruby_prism::Location<'_>) -> bool {
    (a.start_offset(), a.end_offset()) == (b.start_offset(), b.end_offset())
}
