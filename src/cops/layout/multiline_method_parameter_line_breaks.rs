//! `Layout/MultilineMethodParameterLineBreaks` (opt-in) — a parameter of a
//! multi-line method definition that starts on a line an earlier parameter
//! already occupies.
//!
//! A port of RuboCop's `on_def`/`on_defs` (97f30c4, MIT) at the fixed
//! `AllowMultilineFinalElement: false`. Every Prism `def` (with or without a
//! receiver or parentheses, endless included) with parameters; the children
//! are its parameters of every kind in source order (shared with
//! `Layout/FirstMethodParameterLineBreak`). Block and lambda parameters
//! are not a definition's. The shared `MultilineElementLineBreaks`
//! projection (`multiline_element_line_breaks`) decides the findings and the
//! guarded line-break insertion before each reported parameter.
use ruby_prism::Node;

use super::{
    first_method_parameter_line_break::parameters_in_source_order,
    multiline_element_line_breaks::check_line_breaks,
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineMethodParameterLineBreaks;

const MSG: &str = "Each parameter in a multi-line method definition must start on a separate line.";

impl Cop for MultilineMethodParameterLineBreaks {
    fn name(&self) -> &'static str {
        "Layout/MultilineMethodParameterLineBreaks"
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
        let Some(parameters) = node.as_def_node().and_then(|def| def.parameters()) else {
            return;
        };
        let children = parameters_in_source_order(&parameters);
        check_line_breaks(self, MSG, &children, cx, file, out);
    }
}

#[cfg(test)]
mod tests;
