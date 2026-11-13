//! `Metrics/ModuleLength` — modules longer than the fixed 100 lines.
//!
//! Upstream `on_module` and `on_casgn` at the pinned defaults: `Max: 100`,
//! `CountComments: false`, `CountAsOne: []`. A `module` is counted by lines
//! (nested `class`/`module` lines excluded, upstream's line indexing kept).
//! A namespace-free constant assigned directly a `Module.new` block (no
//! arguments, top-level `Module` receiver, no safe navigation) is measured
//! by the block body and reported at the constant's name. There is no
//! correction.

use ruby_prism::Node;

use super::code_length::{body_length, check, classlike_length};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// The default `Max`.
const MAX: usize = 100;

/// The `Metrics/ModuleLength` cop.
pub struct ModuleLength {
    /// The value of `Max`.
    max: usize,
}

impl Default for ModuleLength {
    fn default() -> Self {
        ModuleLength { max: MAX }
    }
}

impl Cop for ModuleLength {
    fn name(&self) -> &'static str {
        "Metrics/ModuleLength"
    }

    fn max_setting(&self) -> Option<&'static str> {
        Some("max")
    }

    fn with_max(&self, max: usize) -> Option<Box<dyn Cop>> {
        Some(Box::new(ModuleLength { max }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let max = self.max;
        let location = node.location();
        if let Some(module) = node.as_module_node() {
            let length = || classlike_length(node, module.body(), file);
            check(self, "Module", max, &location, &location, length, file, out);
        } else if let Some(write) = node.as_constant_write_node()
            && let Some(call) = write.value().as_call_node()
            && call.name().as_slice() == b"new"
            && !call.is_safe_navigation()
            && call.arguments().is_none()
            && call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Module"]))
            && let Some(block) = call.block().and_then(|b| b.as_block_node())
        {
            let length = || body_length(block.body(), file);
            check(
                self,
                "Module",
                max,
                &location,
                &write.name_loc(),
                length,
                file,
                out,
            );
        }
    }
}

#[cfg(test)]
mod tests;
