//! `Layout/FirstMethodArgumentLineBreak` (opt-in) — a multi-line,
//! parenthesized argument list whose first argument starts on the call's
//! first line.
//!
//! A port of RuboCop's `on_send`/`on_csend`/`on_super` (97f30c4, MIT) at the
//! fixed `AllowMultilineFinalElement: false` and `AllowedMethods: []`. The
//! parser's arguments of every Prism call (`send` and `csend`) and every
//! `super` with arguments: the positional arguments, a trailing brace-less
//! keyword hash flattened into its pairs and splats, and a block pass (which
//! keeps a keyword hash before it whole, as upstream's last argument is then
//! the block pass). Upstream's `method_uses_parens?` reads the call's first
//! line up to the first argument, so only calls whose first argument follows
//! `(` on the line where the call (its receiver, for a chain) starts are
//! checked. The shared `FirstElementLineBreak` projection
//! (`first_element_line_break`) decides the finding and the guarded
//! line-break insertion before the first argument; upstream's
//! `autocorrect_incompatible_with` (the opt-in
//! `Style/MethodCallWithArgsParentheses`) holds the edits while both run.
use ruby_prism::Node;

use super::first_element_line_break::{first_element, method_uses_parens, report};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct FirstMethodArgumentLineBreak;

const MSG: &str =
    "Add a line break before the first argument of a multi-line method argument list.";

impl Cop for FirstMethodArgumentLineBreak {
    fn name(&self) -> &'static str {
        "Layout/FirstMethodArgumentLineBreak"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MethodCallWithArgsParentheses"]
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
            (call.arguments(), call.block())
        } else if let Some(call) = node.as_super_node() {
            (call.arguments(), call.block())
        } else {
            return;
        };
        let elements = flattened_arguments(arguments, block);
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

/// The parser's arguments of a call, in source order, with upstream's
/// trailing keyword-hash flattening: the positional arguments, a trailing
/// brace-less keyword hash replaced by its pairs and splats, and a block
/// pass (which keeps a keyword hash before it whole, as upstream's last
/// argument is then the block pass). Shared with
/// `Layout/MultilineMethodArgumentLineBreaks`.
pub(crate) fn flattened_arguments<'pr>(
    arguments: Option<ruby_prism::ArgumentsNode<'pr>>,
    block: Option<Node<'pr>>,
) -> Vec<Node<'pr>> {
    let mut elements: Vec<_> = arguments
        .map(|a| a.arguments().iter().collect())
        .unwrap_or_default();
    match block {
        Some(pass) if pass.as_block_argument_node().is_some() => elements.push(pass),
        _ => {
            // `args = args[0...-1] + last_arg.children if
            // last_arg&.hash_type? && !last_arg&.braces?`
            if let Some(hash) = elements.last().and_then(|a| a.as_keyword_hash_node()) {
                let pairs: Vec<_> = hash.elements().iter().collect();
                elements.pop();
                elements.extend(pairs);
            }
        }
    }
    elements
}

#[cfg(test)]
mod tests;
