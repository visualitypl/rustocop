//! `Layout/MultilineMethodCallBraceLayout` — the closing parenthesis of a
//! multi-line method call sits on the last argument's line exactly when the
//! opening parenthesis sits on the first argument's line.
//!
//! A port of RuboCop's `on_send`/`on_csend` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: symmetrical`. Every Prism call that the parser
//! translation emits as a `send` or `csend` (never `index`, `[]=` or
//! `super`) with parentheses; the parser's arguments as the children: the
//! Prism arguments (a brace-less keyword hash is one argument) and a block
//! pass. `single_line_ignoring_receiver?`: a call whose parentheses share a
//! line is ignored however many lines its receiver spans. The shared
//! `MultilineLiteralBraceLayout` projection (`multiline_literal_brace_layout`)
//! decides the findings and the guarded canonical edit, here with the
//! `send`'s descendants (receiver, arguments, block pass; never the literal
//! block wrapping it) for `heredoc_opened_on_closing_brace_line?`, heredoc
//! bodies allowed between the last argument and the parenthesis, and
//! upstream's `correct_heredoc_argument_method_chain` move.
//!
//! Upstream's `autocorrect_incompatible_with` (the opt-in
//! `Style/MethodCallWithArgsParentheses`) holds the edits while both run.
//! The other pass-ordering peers (`MultilineArrayBraceLayout`,
//! `MultilineHashBraceLayout`, whose upstream lists name this cop) are
//! default-on and not declared: as there, the edit starts at the last
//! argument's last character, so a nested brace edit overlaps it and one
//! `--fix` pass keeps one of the two.
use ruby_prism::Node;

use super::{
    heredoc::call_type,
    multiline_literal_brace_layout::{List, Literal, Messages, check_brace_layout},
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineMethodCallBraceLayout;

const MESSAGES: Messages = Messages {
    same_line: "Closing method call brace must be on the same line as the last argument when \
                opening brace is on the same line as the first argument.",
    new_line: "Closing method call brace must be on the line after the last argument when \
               opening brace is on a separate line from the first argument.",
};

/// The closing parenthesis of a `send`/`csend` (`same_kind_as_node?`).
fn call_closing(node: &Node<'_>) -> Option<std::ops::Range<usize>> {
    call_type(node)?;
    let closing = node.as_call_node()?.closing_loc()?;
    Some(closing.start_offset()..closing.end_offset())
}

impl Cop for MultilineMethodCallBraceLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineMethodCallBraceLayout"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MethodCallWithArgsParentheses"]
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if call_type(node).is_none() {
            return;
        }
        let Some(call) = node.as_call_node() else {
            return;
        };
        let (Some(opening), Some(closing)) = (call.opening_loc(), call.closing_loc()) else {
            return;
        };
        // `single_line_ignoring_receiver?`.
        if file.line_of(opening.start_offset()) == file.line_of(closing.start_offset()) {
            return;
        }
        // The parser's arguments: the Prism arguments and a block pass.
        let arguments = || {
            call.arguments()
                .into_iter()
                .flat_map(|a| a.arguments().iter())
                .chain(
                    call.block()
                        .filter(|b| b.as_block_argument_node().is_some()),
                )
        };
        let children: Vec<_> = arguments().collect();
        let scan: Vec<_> = call.receiver().into_iter().chain(arguments()).collect();
        let literal = Literal {
            node,
            opening: Some(opening.start_offset()),
            closing: Some(closing.start_offset()..closing.end_offset()),
            children: &children,
            same_kind_closing: call_closing,
            scan: &scan,
            list: List::Call,
        };
        check_brace_layout(self, &MESSAGES, &literal, cx, file, out);
    }
}

#[cfg(test)]
mod tests;
