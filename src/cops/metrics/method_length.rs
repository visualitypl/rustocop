//! `Metrics/MethodLength` — methods longer than the fixed 10 lines.
//!
//! Upstream `on_def`/`on_defs` and `define_method` blocks (`block`,
//! `numblock` and `itblock`, any receiver and name argument) at the pinned
//! defaults: `Max: 10`, `CountComments: false`, `CountAsOne: []` and no
//! allowed method names or patterns, so no definition is exempt. The whole
//! definition is reported; there is no correction.

use ruby_prism::Node;

use super::code_length::{body_length, check};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The default `Max`.
const MAX: usize = 10;

/// The `Metrics/MethodLength` cop.
pub struct MethodLength {
    /// The value of `Max`.
    max: usize,
}

impl Default for MethodLength {
    fn default() -> Self {
        MethodLength { max: MAX }
    }
}

impl Cop for MethodLength {
    fn name(&self) -> &'static str {
        "Metrics/MethodLength"
    }

    fn max_setting(&self) -> Option<&'static str> {
        Some("max")
    }

    fn with_max(&self, max: usize) -> Option<Box<dyn Cop>> {
        Some(Box::new(MethodLength { max }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let max = self.max;
        let body = if let Some(def) = node.as_def_node() {
            def.body()
        } else if let Some(call) = node.as_call_node()
            && call.name().as_slice() == b"define_method"
            && let Some(block) = call.block().and_then(|b| b.as_block_node())
        {
            // RuboCop's block node spans the send and its block, as Prism's call.
            block.body()
        } else {
            return;
        };
        let location = node.location();
        check(
            self,
            "Method",
            max,
            &location,
            &location,
            || body_length(body, file),
            file,
            out,
        );
    }
}

#[cfg(test)]
mod tests;
