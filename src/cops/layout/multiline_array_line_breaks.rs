//! `Layout/MultilineArrayLineBreaks` (opt-in) — an item of a multi-line
//! array that starts on a line an earlier item already occupies.
//!
//! A port of RuboCop's `on_array` (97f30c4, MIT) at the fixed
//! `AllowMultilineFinalElement: false`. Every Prism `ArrayNode` (the
//! parser's `array`): bracketed, percent (`%w`, `%i`, `%W`, `%I`) and the
//! implicit arrays of assignments (`x = 1,\n  2, 3`). Its elements are the
//! children, in source order. The shared `MultilineElementLineBreaks`
//! projection (`multiline_element_line_breaks`) decides the findings and the
//! guarded line-break insertion before each reported item.
use ruby_prism::Node;

use super::multiline_element_line_breaks::check_line_breaks;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineArrayLineBreaks;

const MSG: &str = "Each item in a multi-line array must start on a separate line.";

impl Cop for MultilineArrayLineBreaks {
    fn name(&self) -> &'static str {
        "Layout/MultilineArrayLineBreaks"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    /// Opt-in as upstream: only its exact name in `--only` or its own
    /// `enabled = true` runs it, never a department selection.
    fn department_selection_enables(&self) -> bool {
        false
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(array) = node.as_array_node() else {
            return;
        };
        let elements: Vec<_> = array.elements().iter().collect();
        check_line_breaks(self, MSG, &elements, cx, file, out);
    }
}

#[cfg(test)]
mod tests;
