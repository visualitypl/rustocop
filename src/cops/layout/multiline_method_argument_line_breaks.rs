//! `Layout/MultilineMethodArgumentLineBreaks` (opt-in) — an argument of a
//! multi-line method call that starts on a line an earlier argument already
//! occupies.
//!
//! A port of RuboCop's `on_send`/`on_csend` (97f30c4, MIT) at the fixed
//! `AllowMultilineFinalElement: false`. Every parser `send` and `csend`
//! except `[]=`: Prism calls (index assignments and their multiple-assignment
//! targets are `[]=` and excluded), and the `[]` read that an index operator
//! assignment (`a[1, 2] += 3`, `||=`, `&&=`) builds. `super`, `yield` and
//! other keywords are not sends. The children are the parser's arguments,
//! with a trailing brace-less keyword hash flattened into its pairs and
//! splats unless a block pass follows (shared with
//! `Layout/FirstMethodArgumentLineBreak`); the block pass is an argument.
//! The shared `MultilineElementLineBreaks` projection
//! (`multiline_element_line_breaks`) decides the findings and the guarded
//! line-break insertion before each reported argument.
use ruby_prism::Node;

use super::{
    first_method_argument_line_break::flattened_arguments,
    multiline_element_line_breaks::check_line_breaks,
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineMethodArgumentLineBreaks;

const MSG: &str = "Each argument in a multi-line method call must start on a separate line.";

impl Cop for MultilineMethodArgumentLineBreaks {
    fn name(&self) -> &'static str {
        "Layout/MultilineMethodArgumentLineBreaks"
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
        let (arguments, block) = if let Some(call) = node.as_call_node() {
            // `return if node.method?(:[]=)`
            if call.name().as_slice() == b"[]=" {
                return;
            }
            (call.arguments(), call.block())
        } else if let Some(write) = node.as_index_operator_write_node() {
            (write.arguments(), write.block().map(|b| b.as_node()))
        } else if let Some(write) = node.as_index_or_write_node() {
            (write.arguments(), write.block().map(|b| b.as_node()))
        } else if let Some(write) = node.as_index_and_write_node() {
            (write.arguments(), write.block().map(|b| b.as_node()))
        } else {
            return;
        };
        let children = flattened_arguments(arguments, block);
        check_line_breaks(self, MSG, &children, cx, file, out);
    }
}

#[cfg(test)]
mod tests;
