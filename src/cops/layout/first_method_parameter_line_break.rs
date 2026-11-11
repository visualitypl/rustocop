//! `Layout/FirstMethodParameterLineBreak` (opt-in) — a multi-line,
//! parenthesized parameter list whose first parameter starts on the
//! definition's first line.
//!
//! A port of RuboCop's `on_def`/`on_defs` (97f30c4, MIT) at the fixed
//! `AllowMultilineFinalElement: false`. Every Prism `def` (with or without a
//! receiver, endless included); the parameters in source order, every kind
//! (required, optional, rest, post, keyword, keyword rest, `**nil`, `...`,
//! block, destructuring). Upstream's `method_uses_parens?` reads the
//! definition's first line up to the first parameter, so only lists whose
//! first parameter follows `(` on the `def` line are checked. The shared
//! `FirstElementLineBreak` projection (`first_element_line_break`) decides
//! the finding and the guarded line-break insertion before the first
//! parameter. Upstream's pass-ordering `autocorrect_incompatible_with`
//! (`Style/MethodDefParentheses`, default-on here) is not declared: it would
//! withhold every edit in the default selection, and the two cops never edit
//! the same list (that cop only adds missing parentheses).
use ruby_prism::Node;

use super::first_element_line_break::{first_element, method_uses_parens, report};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct FirstMethodParameterLineBreak;

const MSG: &str =
    "Add a line break before the first parameter of a multi-line method parameter list.";

impl Cop for FirstMethodParameterLineBreak {
    fn name(&self) -> &'static str {
        "Layout/FirstMethodParameterLineBreak"
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
        let elements = parameters_in_source_order(&parameters);
        let Some(first) = elements.first() else {
            return;
        };
        let start = node.location().start_offset();
        if !method_uses_parens(file, start, first.location().start_offset()) {
            return;
        }
        if let Some(first) = first_element(file, file.line_of(start), &elements) {
            report(self, MSG, first, cx, file, out);
        }
    }
}

/// The parser's `args` children of a definition: parameters of every kind
/// (required, optional, rest, post, keyword, keyword rest, `**nil`, `...`,
/// block, destructuring) in source order. Shared with
/// `Layout/MultilineMethodParameterLineBreaks`.
pub(crate) fn parameters_in_source_order<'pr>(
    parameters: &ruby_prism::ParametersNode<'pr>,
) -> Vec<Node<'pr>> {
    let mut elements: Vec<Node<'pr>> = parameters.requireds().iter().collect();
    elements.extend(parameters.optionals().iter());
    elements.extend(parameters.rest());
    elements.extend(parameters.posts().iter());
    elements.extend(parameters.keywords().iter());
    elements.extend(parameters.keyword_rest());
    elements.extend(parameters.block().map(|b| b.as_node()));
    elements.sort_by_key(|e| e.location().start_offset());
    elements
}

#[cfg(test)]
mod tests;
