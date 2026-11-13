//! `Layout/FirstHashElementLineBreak` (opt-in) — a multi-line braced hash
//! whose first element starts on its opening line.
//!
//! A port of RuboCop's `on_hash` (97f30c4, MIT) at the fixed
//! `AllowMultilineFinalElement: false`. Only hashes with a `{` (Prism's
//! `HashNode`) are checked; keyword arguments and other brace-less hashes
//! (`KeywordHashNode`) are `Layout/FirstMethodArgumentLineBreak`'s. Pairs and
//! `**` splats are the elements. The shared `FirstElementLineBreak`
//! projection (`first_element_line_break`) decides the finding and the
//! guarded line-break insertion before the first element.
use ruby_prism::Node;

use super::first_element_line_break::{first_element, report};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct FirstHashElementLineBreak;

const MSG: &str = "Add a line break before the first element of a multi-line hash.";

impl Cop for FirstHashElementLineBreak {
    fn name(&self) -> &'static str {
        "Layout/FirstHashElementLineBreak"
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
        let start_line = file.line_of(node.location().start_offset());
        if let Some(first) = first_element(file, start_line, &elements) {
            report(self, MSG, first, cx, file, out);
        }
    }
}

#[cfg(test)]
mod tests;
