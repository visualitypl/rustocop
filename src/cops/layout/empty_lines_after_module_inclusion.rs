//! `Layout/EmptyLinesAfterModuleInclusion` — an empty line follows a module
//! inclusion (`include`, `extend`, `prepend`) or a group of them.
//!
//! Port of upstream's `on_send` over the parser AST that RuboCop builds from
//! Prism. An inclusion without receiver and with arguments is checked when it
//! is a statement with a parser right sibling (see `declaration_spacing`):
//! used as an argument or receiver (`send` parent), with a literal block
//! (`any_block` parent), as an array element or as the sole statement of a
//! branch, `rescue` or `ensure` body it has none. It is reported unless the
//! next line is blank, or holds an `enable` directive (`rubocop:` or
//! `rustocop:`) followed by a blank line, or the sibling is itself an
//! inclusion `send` (any receiver or arguments), directly or as the body of a
//! modifier `if`/`unless`/`while`/`until`.
//!
//! Upstream marks the cop `pending`; rustocop enables it by default. The
//! correction is upstream's line break after the inclusion's last line, or
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

pub struct EmptyLinesAfterModuleInclusion;

const MSG: &str = "Add an empty line after module inclusion.";
const METHODS: &[&[u8]] = &[b"include", b"extend", b"prepend"];

impl Cop for EmptyLinesAfterModuleInclusion {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAfterModuleInclusion"
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
        each_with_sibling(node, cx, false, |statement, sibling| {
            if let Some(offense) = self.check(&statement, &sibling, cx, file) {
                out.push(offense);
            }
        });
    }
}

impl EmptyLinesAfterModuleInclusion {
    fn check(
        &self,
        node: &Node<'_>,
        sibling: &Sibling<'_>,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Offense> {
        let call = node.as_call_node()?;
        if call.receiver().is_some()
            || !has_arguments(&call)
            || !send_named(node, METHODS)
            || sibling.node.as_ref().is_some_and(allowed)
        {
            return None;
        }
        let last_line = file.last_line_of(node.location());
        if next_line_free(file, last_line) {
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

/// `allowed_method?`: the sibling, or a modifier form's body, is an inclusion
/// `send`.
fn allowed(node: &Node<'_>) -> bool {
    let modifier_body = if let Some(n) = node.as_if_node() {
        let modifier = n.if_keyword_loc().is_some_and(|k| k.as_slice() == b"if")
            && n.end_keyword_loc().is_none();
        modifier.then(|| n.statements())
    } else if let Some(n) = node.as_unless_node() {
        n.end_keyword_loc().is_none().then(|| n.statements())
    } else if let Some(n) = node.as_while_node() {
        n.closing_loc().is_none().then(|| n.statements())
    } else if let Some(n) = node.as_until_node() {
        n.closing_loc().is_none().then(|| n.statements())
    } else {
        None
    };
    match modifier_body {
        Some(body) => body
            .and_then(|b| b.body().iter().next())
            .is_some_and(|b| send_named(&b, METHODS)),
        None => send_named(node, METHODS),
    }
}

#[cfg(test)]
mod tests;
