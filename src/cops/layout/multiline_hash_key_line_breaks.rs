//! `Layout/MultilineHashKeyLineBreaks` (opt-in) — a key of a multi-line
//! braced hash that starts on a line an earlier key already occupies.
//!
//! A port of RuboCop's `on_hash` (97f30c4, MIT) at the fixed
//! `AllowMultilineFinalElement: false`. Only hashes with a `{` (Prism's
//! `HashNode`, upstream's `loc.begin` gate) are checked; keyword arguments
//! and other brace-less hashes (`KeywordHashNode`) are
//! `Layout/MultilineMethodArgumentLineBreaks`'. Pairs and `**` splats are
//! the children, in source order. The shared `MultilineElementLineBreaks`
//! projection (`multiline_element_line_breaks`) decides the findings and the
//! guarded line-break insertion before each reported key.
use ruby_prism::Node;

use super::multiline_element_line_breaks::check_line_breaks;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineHashKeyLineBreaks;

const MSG: &str = "Each key in a multi-line hash must start on a separate line.";

impl Cop for MultilineHashKeyLineBreaks {
    fn name(&self) -> &'static str {
        "Layout/MultilineHashKeyLineBreaks"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    /// Opt-in as upstream: only its exact name in `--only` or its own
    /// `enabled = true` runs it, never a department selection.
    fn department_selection_enables(&self) -> bool {
        false
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_hash_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(hash) = node.as_hash_node() else {
            return;
        };
        let elements: Vec<_> = hash.elements().iter().collect();
        check_line_breaks(self, MSG, &elements, cx, file, out);
    }
}

#[cfg(test)]
mod tests;
