//! Sibling and next-line rules shared by `Layout/EmptyLinesAfterModuleInclusion`
//! and `Layout/EmptyLinesAroundAttributeAccessor`.
//!
//! Both upstream cops read the parser AST's `right_sibling` of a declaration
//! call. In Prism a parser `begin`/`kwbegin` (a list of two or more
//! statements, also inside parentheses, a block, a branch or a `begin`) is a
//! `StatementsNode`: each statement's right sibling is the next statement. A
//! sole statement's parser parent is the enclosing node, where the body is the
//! last child, except for a protected body: `(rescue body resbody... else)`
//! and `(ensure body ensure_body)` put the `rescue` clause or the `ensure`
//! body after it. Declarations used as values (arguments, receivers, array
//! elements, assignment values) are never statements and are not visited.
use ruby_prism::Node;

use super::empty_line_after_guard_clause::{enable_comment, line_break_after, next_line_empty};
use crate::{cops::NodeContext, offense::Correction, source::SourceFile};

/// The parser right sibling of a statement.
pub(super) struct Sibling<'pr> {
    /// Start offset of the sibling's parser expression.
    pub start: usize,
    /// The sibling as one Prism node; `None` for a `resbody` or a
    /// multi-statement `ensure` body (a parser `begin`), which are never a
    /// send or an `alias`.
    pub node: Option<Node<'pr>>,
}

/// Calls `f(statement, sibling)` for every statement of the `StatementsNode`
/// `node` that has a parser right sibling, each list once (linear). With
/// `clauses`, a sole protected statement gets its `rescue` clause or `ensure`
/// body as sibling.
pub(super) fn each_with_sibling<'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    clauses: bool,
    mut f: impl FnMut(Node<'pr>, Sibling<'pr>),
) {
    let Some(statements) = node.as_statements_node() else {
        return;
    };
    let list = statements.body();
    let mut nexts = list.iter().skip(1);
    for current in list.iter() {
        let Some(next) = nexts.next() else {
            if list.len() == 1
                && clauses
                && let Some(sibling) = cx
                    .parent()
                    .and_then(|parent| parent.as_begin_node())
                    .and_then(|begin| protected_sibling(&begin))
            {
                f(current, sibling);
            }
            return;
        };
        let sibling = Sibling {
            start: next.location().start_offset(),
            node: Some(next),
        };
        f(current, sibling);
    }
}

/// The parser sibling after a `begin` body with `rescue` or `ensure`.
fn protected_sibling<'pr>(begin: &ruby_prism::BeginNode<'pr>) -> Option<Sibling<'pr>> {
    if let Some(rescue) = begin.rescue_clause() {
        return Some(Sibling {
            start: rescue.location().start_offset(),
            node: None,
        });
    }
    let statements = begin.ensure_clause()?.statements()?;
    let mut body = statements.body().iter();
    let first = body.next()?;
    let single = body.next().is_none();
    Some(Sibling {
        start: first.location().start_offset(),
        node: single.then_some(first),
    })
}

/// `next_line_empty_or_enable_directive_comment?(line)`: the line after
/// `line` is blank (or past the code), or it holds an `enable` directive in
/// either spelling and the line after that is blank.
pub(super) fn next_line_free(file: &SourceFile, line: u32) -> bool {
    next_line_empty(file, line)
        || (enable_comment(file, line + 1).is_some() && next_line_empty(file, line + 1))
}

/// Upstream's autocorrection: a line break after the declaration's last line,
/// or after the `enable` directive on the next line, guarded like the other
/// empty-line insertions.
pub(super) fn insertion(file: &SourceFile, root: &Node<'_>, line: u32) -> Option<Correction> {
    line_break_after(file, root, line, enable_comment(file, line + 1))
}

/// A parser `send` (not `csend`, not the call of a literal block) named one
/// of `names`.
pub(super) fn send_named(node: &Node<'_>, names: &[&[u8]]) -> bool {
    node.as_call_node().is_some_and(|call| {
        !call.is_safe_navigation()
            && call
                .block()
                .is_none_or(|b| b.as_block_argument_node().is_some())
            && names.contains(&call.name().as_slice())
    })
}

/// A call has parser arguments: Prism keeps a `&block` argument apart.
pub(super) fn has_arguments(call: &ruby_prism::CallNode<'_>) -> bool {
    call.arguments().is_some()
        || call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some())
}
