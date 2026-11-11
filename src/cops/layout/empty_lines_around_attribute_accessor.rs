//! `Layout/EmptyLinesAroundAttributeAccessor` — an empty line follows an
//! attribute accessor or a group of them.
//!
//! Port of upstream's `on_send` over the parser AST that RuboCop builds from
//! Prism, at the fixed defaults `AllowAliasSyntax: true` and
//! `AllowedMethods: [alias_method, public, protected, private]`. An
//! `attr_reader`/`attr_writer`/`attr_accessor`/`attr` call without receiver
//! and with arguments is checked when it is a statement with a parser right
//! sibling (see `declaration_spacing`): the next statement, or the `rescue`
//! clause or `ensure` body after a sole protected statement. The sole
//! statement of a branch (parser `if` parent) or of any other body has none.
//! It is reported unless the next line is blank, or holds an `enable`
//! directive (`rubocop:` or `rustocop:`) followed by a blank line, or the
//! sibling starts on the accessor's last line, is an `alias`, or is a `send`
//! that is itself an accessor or one of the allowed methods.
//!
//! The correction is upstream's line break after the accessor's last line, or
//! after the `enable` directive on the next line, with that line's own
//! ending. It is withheld, finding kept, for the listed source
//! observers (the shared position barrier holds a later `__LINE__`), and when
//! the insertion point is inside a literal, ends a line that a heredoc body
//! continues, or follows a backslash.
use ruby_prism::Node;

use super::declaration_spacing::{
    Sibling, each_with_sibling, has_arguments, insertion, next_line_free, send_named,
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct EmptyLinesAroundAttributeAccessor;

const MSG: &str = "Add an empty line after attribute accessor.";
const ACCESSORS: &[&[u8]] = &[b"attr_reader", b"attr_writer", b"attr_accessor", b"attr"];
/// The fixed `AllowedMethods`.
const ALLOWED: &[&[u8]] = &[b"alias_method", b"public", b"protected", b"private"];

impl Cop for EmptyLinesAroundAttributeAccessor {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundAttributeAccessor"
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
        each_with_sibling(node, cx, true, |statement, sibling| {
            if let Some(offense) = self.check(&statement, &sibling, cx, file) {
                out.push(offense);
            }
        });
    }
}

impl EmptyLinesAroundAttributeAccessor {
    fn check(
        &self,
        node: &Node<'_>,
        sibling: &Sibling<'_>,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Offense> {
        if !attribute_accessor(node) {
            return None;
        }
        let last_line = file.last_line_of(node.location());
        if next_line_free(file, last_line)
            || file.line_of(sibling.start) == last_line
            || sibling.node.as_ref().is_some_and(allowed)
        {
            return None;
        }
        let root = cx.ancestors().last()?;
        let (line, col) = file.line_col(node.location().start_offset());
        Some(Offense {
            correction: cx
                .corrections_requested()
                .then(|| insertion(file, root, last_line))
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line: last_line,
            col,
        })
    }
}

/// RuboCop AST's `attribute_accessor?`: `(send nil? {attr_*} _ ...)`.
fn attribute_accessor(node: &Node<'_>) -> bool {
    send_named(node, ACCESSORS)
        && node
            .as_call_node()
            .is_some_and(|call| call.receiver().is_none() && has_arguments(&call))
}

/// `allow_alias?` or `attribute_or_allowed_method?` for the sibling.
fn allowed(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::AliasMethodNode { .. } | Node::AliasGlobalVariableNode { .. }
    ) || attribute_accessor(node)
        || send_named(node, ALLOWED)
}

#[cfg(test)]
mod tests;
