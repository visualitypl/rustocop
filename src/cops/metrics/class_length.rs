//! `Metrics/ClassLength` — classes longer than the fixed 100 lines.
//!
//! Upstream `on_class`, `on_sclass` (only outside every `class`) and
//! `on_casgn` at the pinned defaults: `Max: 100`, `CountComments: false`,
//! `CountAsOne: []`. A constant assigned a `Class.new`/`Struct.new` block
//! (top-level constant receiver, no safe navigation, nonempty body) is
//! measured as that block, whether the constant is assigned directly,
//! through `||=`/`&&=`/an operator, or as a direct target of a multiple
//! assignment; the block is reported once. A `class` is counted by lines
//! (nested `class`/`module` lines excluded, upstream's line indexing kept);
//! a singleton class and a constructor block by their body. There is no
//! correction.

use ruby_prism::{BlockNode, Node};

use super::code_length::{body_length, check, classlike_length};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// Pinned `Max`.
const MAX: usize = 100;

/// The `Metrics/ClassLength` cop.
pub struct ClassLength;

impl Cop for ClassLength {
    fn name(&self) -> &'static str {
        "Metrics/ClassLength"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let location = node.location();
        if let Some(class) = node.as_class_node() {
            let length = || classlike_length(node, class.body(), file);
            check(self, "Class", MAX, &location, &location, length, file, out);
        } else if let Some(sclass) = node.as_singleton_class_node() {
            if cx.ancestors().any(|a| a.as_class_node().is_some()) {
                return;
            }
            let length = || body_length(sclass.body(), file);
            check(self, "Class", MAX, &location, &location, length, file, out);
        } else if let Some(value) = constant_assignment_value(node)
            && let Some(block) = class_definition(&value)
        {
            let location = value.location();
            let length = || body_length(block.body(), file);
            check(self, "Class", MAX, &location, &location, length, file, out);
        }
    }
}

/// The value RuboCop's `on_casgn` finds for a constant assignment: the
/// expression of a `casgn`, of the `||=`/`&&=`/operator assignment around
/// it, or of the multiple assignment whose direct target it is (one value
/// for all such targets, as upstream reports one range once).
fn constant_assignment_value<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    Some(match node {
        Node::ConstantWriteNode { .. } => node.as_constant_write_node()?.value(),
        Node::ConstantOrWriteNode { .. } => node.as_constant_or_write_node()?.value(),
        Node::ConstantAndWriteNode { .. } => node.as_constant_and_write_node()?.value(),
        Node::ConstantOperatorWriteNode { .. } => node.as_constant_operator_write_node()?.value(),
        Node::ConstantPathWriteNode { .. } => node.as_constant_path_write_node()?.value(),
        Node::ConstantPathOrWriteNode { .. } => node.as_constant_path_or_write_node()?.value(),
        Node::ConstantPathAndWriteNode { .. } => node.as_constant_path_and_write_node()?.value(),
        Node::ConstantPathOperatorWriteNode { .. } => {
            node.as_constant_path_operator_write_node()?.value()
        }
        Node::MultiWriteNode { .. } => {
            let write = node.as_multi_write_node()?;
            let constant = |t: &Node<'_>| {
                t.as_constant_target_node().is_some() || t.as_constant_path_target_node().is_some()
            };
            if !write
                .lefts()
                .iter()
                .chain(write.rights().iter())
                .any(|t| constant(&t))
            {
                return None;
            }
            write.value()
        }
        _ => return None,
    })
}

/// RuboCop's `class_definition?` for a block: a `Struct.new`/`Class.new`
/// send (top-level constant, any arguments) with a nonempty literal block.
fn class_definition<'pr>(value: &Node<'pr>) -> Option<BlockNode<'pr>> {
    let call = value.as_call_node()?;
    let receiver = call.receiver()?;
    let block = call.block()?.as_block_node()?;
    (call.name().as_slice() == b"new"
        && !call.is_safe_navigation()
        && ast::is_global_const(&receiver, &["Struct", "Class"])
        && block.body().is_some())
    .then_some(block)
}

#[cfg(test)]
mod tests;
