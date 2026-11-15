//! `Metrics/BlockLength` — blocks longer than the fixed 25 lines.
//!
//! Upstream `on_block`/`on_numblock`/`on_itblock` at the pinned defaults:
//! `Max: 25`, `CountComments: false`, `CountAsOne: []`,
//! `AllowedMethods: [refine]` (matched by method name with any receiver, as
//! upstream's `allowed_method?` and `method_receiver_excluded?` do for a
//! receiverless entry), `AllowedPatterns: []`, and the pinned
//! `Exclude: **/*.gemspec` path policy. Blocks of the class constructors
//! `Class.new`, `Module.new`, `Struct.new` and `Data.define` (top-level
//! constant receivers, no safe navigation) are skipped. RuboCop's block node
//! is Prism's call (or `super`) with a literal block, or a `->` lambda; the
//! whole node is reported. There is no correction.

use ruby_prism::{CallNode, Node};

use super::code_length::{body_length, check};
use crate::{
    cops::{
        Cop, NodeContext, ast,
        settings::{self, Setting, Values},
    },
    offense::Offense,
    source::SourceFile,
};

/// The default `Max`.
const MAX: usize = 25;

/// The `Metrics/BlockLength` cop.
pub struct BlockLength {
    /// The value of `Max`.
    max: usize,
}

impl Default for BlockLength {
    fn default() -> Self {
        BlockLength { max: MAX }
    }
}

impl Cop for BlockLength {
    fn name(&self) -> &'static str {
        "Metrics/BlockLength"
    }

    fn settings(&self) -> &'static [Setting] {
        &[settings::MAX]
    }

    fn configure(&self, values: &Values) -> Option<Box<dyn Cop>> {
        let max = values.count("max").unwrap_or(self.max);
        Some(Box::new(BlockLength { max }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let max = self.max;
        let body = match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
                    return;
                };
                if call.name().as_slice() == b"refine" || class_constructor(&call) {
                    return;
                }
                block.body()
            }
            Node::SuperNode { .. } => {
                let Some(block) = node
                    .as_super_node()
                    .unwrap()
                    .block()
                    .and_then(|b| b.as_block_node())
                else {
                    return;
                };
                block.body()
            }
            Node::ForwardingSuperNode { .. } => {
                let Some(block) = node.as_forwarding_super_node().unwrap().block() else {
                    return;
                };
                block.body()
            }
            // `->` is RuboCop's block with a `lambda` send.
            Node::LambdaNode { .. } => node.as_lambda_node().unwrap().body(),
            _ => return,
        };
        let location = node.location();
        check(
            self,
            "Block",
            max,
            &location,
            &location,
            || body_length(body, file),
            file,
            out,
        );
    }
}

/// RuboCop's `class_constructor?`: a `Class`/`Module`/`Struct.new` or
/// `Data.define` send on a top-level constant.
fn class_constructor(call: &CallNode<'_>) -> bool {
    if call.is_safe_navigation() {
        return false;
    }
    let names: &[&str] = match call.name().as_slice() {
        b"new" => &["Class", "Module", "Struct"],
        b"define" => &["Data"],
        _ => return false,
    };
    call.receiver()
        .is_some_and(|receiver| ast::is_global_const(&receiver, names))
}

#[cfg(test)]
mod tests;
