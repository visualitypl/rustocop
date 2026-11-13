//! `Layout/MultilineMethodDefinitionBraceLayout` — the closing parenthesis
//! of a multi-line parameter list sits on the last parameter's line exactly
//! when the opening parenthesis sits on the first parameter's line.
//!
//! A port of RuboCop's `on_def`/`on_defs` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: symmetrical`: `check_brace_layout(node.arguments)`. Every
//! Prism `def` (with or without a receiver, endless included) whose
//! parameters are parenthesized; the parser's `args` children (every
//! parameter kind, in source order) as the children. Parentheses on one line
//! are a single-line `args` node and ignored, as are empty lists. The shared
//! `MultilineLiteralBraceLayout` projection (`multiline_literal_brace_layout`)
//! decides the findings and the guarded canonical edit, here with the
//! parameters (never the body) for `heredoc_opened_on_closing_brace_line?`,
//! heredoc bodies of default values allowed between the last parameter and
//! the parenthesis, and an `args` node that is never `chained?` or
//! `argument?`. Nested `args` nodes with a `)` (definitions and lambda
//! literals in default values) are the same kind for
//! `nested_closing_bracket_moved_before_comment?`.
//!
//! Upstream's `autocorrect_incompatible_with` (the opt-in
//! `Style/MultilineMethodSignature`) holds the edits while both run.
use std::ops::Range;

use ruby_prism::Node;

use super::{
    first_method_parameter_line_break::parameters_in_source_order,
    multiline_literal_brace_layout::{List, Literal, Messages, check_brace_layout},
};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct MultilineMethodDefinitionBraceLayout;

const MESSAGES: Messages = Messages {
    same_line: "Closing method definition brace must be on the same line as the last parameter \
                when opening brace is on the same line as the first parameter.",
    new_line: "Closing method definition brace must be on the line after the last parameter \
               when opening brace is on a separate line from the first parameter.",
};

/// The closing `)` of a parser `args` node: a parenthesized definition's or
/// lambda literal's parameters (`same_kind_as_node?`).
fn args_closing(node: &Node<'_>) -> Option<Range<usize>> {
    let closing = match node {
        Node::DefNode { .. } => node.as_def_node()?.rparen_loc()?,
        Node::BlockParametersNode { .. } => node.as_block_parameters_node()?.closing_loc()?,
        _ => return None,
    };
    Some(closing.start_offset()..closing.end_offset())
}

impl Cop for MultilineMethodDefinitionBraceLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineMethodDefinitionBraceLayout"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MultilineMethodSignature"]
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        let (Some(opening), Some(closing), Some(parameters)) =
            (def.lparen_loc(), def.rparen_loc(), def.parameters())
        else {
            return;
        };
        // `node.single_line?` of the `args` node.
        if file.line_of(opening.start_offset()) == file.line_of(closing.start_offset()) {
            return;
        }
        let children = parameters_in_source_order(&parameters);
        let scan = [parameters.as_node()];
        let literal = Literal {
            node,
            opening: Some(opening.start_offset()),
            closing: Some(closing.start_offset()..closing.end_offset()),
            children: &children,
            same_kind_closing: args_closing,
            scan: &scan,
            list: List::Parameters,
        };
        check_brace_layout(self, &MESSAGES, &literal, cx, file, out);
    }
}

#[cfg(test)]
mod tests;
