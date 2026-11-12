//! `Metrics/CollectionLiteralLength` — array, hash and `Set[]` "literals"
//! with at least the fixed 250 entries.
//!
//! Upstream's `on_array`/`on_hash` count the children of every parser
//! `array` and `hash` node, and `on_index`/`on_send` count the arguments of
//! `[]` sent to `Set` or `::Set`. In the pinned Prism translation those
//! nodes are: array literals of every kind (`[]`, `%w`/`%i`, the bracketless
//! right-hand side of `a = 1, 2` and `a, b = 1, 2`), the exception list of a
//! `rescue` clause, hash literals and braceless keyword arguments, and
//! `Set[…]`, `Set.[](…)` and the `Set[…]` read of an index op-assignment
//! (a block argument is one of the arguments). `Set&.[]` is a `csend` and is
//! not checked. `Max: 250` is fixed (the cop is pending upstream and enabled
//! here); the whole literal is reported and there is no correction.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// Pinned `Max`: a collection with at least this many entries is reported.
const MAX: usize = 250;

const MSG: &str = "Avoid hard coding large quantities of data in code. \
                   Prefer reading the data from an external source.";

/// The `Metrics/CollectionLiteralLength` cop.
pub struct CollectionLiteralLength;

impl Cop for CollectionLiteralLength {
    fn name(&self) -> &'static str {
        "Metrics/CollectionLiteralLength"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((start, end)) = large_collection(node) else {
            return;
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.to_owned(),
            line,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            col,
        });
    }
}

/// The parser expression range of a reported collection, if `node` is one.
fn large_collection(node: &Node<'_>) -> Option<(usize, usize)> {
    let whole = (node.location().start_offset(), node.location().end_offset());
    if let Some(array) = node.as_array_node() {
        return (array.elements().len() >= MAX).then_some(whole);
    }
    if let Some(hash) = node.as_hash_node() {
        return (hash.elements().len() >= MAX).then_some(whole);
    }
    if let Some(hash) = node.as_keyword_hash_node() {
        return (hash.elements().len() >= MAX).then_some(whole);
    }
    if let Some(rescue) = node.as_rescue_node() {
        // The translation wraps the exception list in a bracketless array.
        let exceptions = rescue.exceptions();
        if exceptions.len() < MAX {
            return None;
        }
        let first = exceptions.iter().next()?.location().start_offset();
        let last = exceptions.iter().last()?.location().end_offset();
        return Some((first, last));
    }
    if let Some(call) = node.as_call_node() {
        if call.name().as_slice() != b"[]" || call.is_safe_navigation() {
            return None;
        }
        let block_argument = call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some());
        let count =
            call.arguments().map_or(0, |a| a.arguments().len()) + usize::from(block_argument);
        return (set_const(call.receiver()) && count >= MAX)
            .then(|| (whole.0, ast::source_end(node)));
    }
    // `Set[…] op= v` reads through an `index` send of its own.
    let (receiver, arguments, block, closing) = index_write(node)?;
    let count = arguments.map_or(0, |a| a.arguments().len()) + usize::from(block.is_some());
    (set_const(receiver) && count >= MAX).then_some((whole.0, closing))
}

/// Receiver, arguments, block argument and end of the `]` of an index
/// assignment.
type IndexWrite<'pr> = (
    Option<Node<'pr>>,
    Option<ruby_prism::ArgumentsNode<'pr>>,
    Option<ruby_prism::BlockArgumentNode<'pr>>,
    usize,
);

/// The read of an index operator, `||=` or `&&=` assignment.
fn index_write<'pr>(node: &Node<'pr>) -> Option<IndexWrite<'pr>> {
    if let Some(w) = node.as_index_operator_write_node() {
        return Some((
            w.receiver(),
            w.arguments(),
            w.block(),
            w.closing_loc().end_offset(),
        ));
    }
    if let Some(w) = node.as_index_or_write_node() {
        return Some((
            w.receiver(),
            w.arguments(),
            w.block(),
            w.closing_loc().end_offset(),
        ));
    }
    let w = node.as_index_and_write_node()?;
    Some((
        w.receiver(),
        w.arguments(),
        w.block(),
        w.closing_loc().end_offset(),
    ))
}

/// `Set` or `::Set`: `(const {cbase nil?} :Set)`.
fn set_const(receiver: Option<Node<'_>>) -> bool {
    let Some(receiver) = receiver else {
        return false;
    };
    if let Some(constant) = receiver.as_constant_read_node() {
        return constant.name().as_slice() == b"Set";
    }
    receiver.as_constant_path_node().is_some_and(|path| {
        path.parent().is_none() && path.name().is_some_and(|name| name.as_slice() == b"Set")
    })
}

#[cfg(test)]
mod tests;
