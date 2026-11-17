//! `Layout/SpaceAroundBlockParameters` — no blanks inside the pipes of a
//! block's parameters, one blank after the closing pipe and before each
//! parameter.
//!
//! A port of RuboCop's cop (97f30c4, MIT) at the fixed
//! `EnforcedStyleInsidePipes: no_space`. `on_block` sees every block with a
//! literal body and every lambda literal, never a numbered-parameter or `it`
//! block: Prism's `BlockNode` and `LambdaNode` whose parameters are a
//! `BlockParametersNode` with delimiters (`|...|`, or `(...)` for a lambda;
//! `->a {}` has none) and at least one parameter or block-local variable
//! (`{ || }` and `->() {}` have none). Three checks follow, with RuboCop's
//! messages and ranges:
//!
//! - Inside the delimiters, skipped for a lambda's parentheses while
//!   `Layout/SpaceInsideParens` is configured enabled (its configured
//!   enablement, never the CLI selection, as `config.cop_enabled?`): the gap
//!   between the opening pipe and the first parameter (`Space before first
//!   block parameter detected.`) and between the last parameter (after its
//!   trailing comma, if any) and the closing pipe (`Space after last block
//!   parameter detected.`), unless the gap holds a line break.
//! - With a body, a closing pipe followed directly by the body (`Space after
//!   closing `|` missing.`, at the pipe).
//! - Before each parameter, and each element of a destructuring parameter
//!   after the elements it holds, the blanks beyond one (`Extra space before
//!   block parameter detected.`), unless a line break precedes them.
//!
//! Parameters are RuboCop's `args` children: Prism's parameters in source
//! order, then the block-local variables; the trailing comma of `|a,|` (an
//! `ImplicitRestNode`) is not a parameter.
//!
//! The corrections remove a reported gap or insert one space after the
//! closing pipe. A removal is made only when the gap is spaces and tabs; the
//! one other gap the checks can report, the `;` before the first
//! block-local variable of `|;a|`, keeps its finding without an edit (RuboCop
//! would remove the `;` and turn the local into a parameter). A gap inside
//! another reported gap gets no edit of its own, so one pass removes the
//! whole outer gap. Everything is withheld for the listed source
//! observers.
use std::ops::Range;

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAroundBlockParameters {
    /// Whether `Layout/SpaceInsideParens` is configured enabled.
    space_inside_parens: bool,
}

impl Default for SpaceAroundBlockParameters {
    fn default() -> Self {
        Self {
            space_inside_parens: true,
        }
    }
}

const BEFORE: &str = "Space before first block parameter detected.";
const AFTER: &str = "Space after last block parameter detected.";
const CLOSING: &str = "Space after closing `|` missing.";
const EXTRA: &str = "Extra space before block parameter detected.";

/// Collect `node` unless it is the implicit rest of a trailing comma.
fn push<'pr>(out: &mut Vec<Node<'pr>>, node: Node<'pr>) {
    if node.as_implicit_rest_node().is_none() {
        out.push(node);
    }
}

/// RuboCop's `args` children of a block's parameters, in source order.
pub(super) fn arguments<'pr>(parameters: &ruby_prism::BlockParametersNode<'pr>) -> Vec<Node<'pr>> {
    let mut out = Vec::new();
    if let Some(list) = parameters.parameters() {
        for node in list.requireds().iter() {
            push(&mut out, node);
        }
        for node in list.optionals().iter() {
            push(&mut out, node);
        }
        if let Some(node) = list.rest() {
            push(&mut out, node);
        }
        for node in list.posts().iter() {
            push(&mut out, node);
        }
        for node in list.keywords().iter() {
            push(&mut out, node);
        }
        if let Some(node) = list.keyword_rest() {
            push(&mut out, node);
        }
        if let Some(node) = list.block() {
            push(&mut out, node.as_node());
        }
    }
    for node in parameters.locals().iter() {
        push(&mut out, node);
    }
    out.sort_by_key(|node| node.location().start_offset());
    out
}

/// The elements of a destructuring parameter (`mlhs` children).
fn elements<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    let mut out = Vec::new();
    if let Some(target) = node.as_multi_target_node() {
        for node in target.lefts().iter() {
            push(&mut out, node);
        }
        if let Some(node) = target.rest() {
            push(&mut out, node);
        }
        for node in target.rights().iter() {
            push(&mut out, node);
        }
    }
    out
}

/// Where the parser gem's body starts. Prism's implicit `begin` of a `do`
/// block with `rescue` or `ensure` starts at `do`; the parser gem's starts at
/// the first statement, else at the first `rescue` or the `ensure` keyword.
fn body_start(body: &Node<'_>) -> usize {
    if let Some(begin) = body.as_begin_node()
        && begin.begin_keyword_loc().is_none()
    {
        let first = begin
            .statements()
            .map(|n| n.location())
            .or_else(|| begin.rescue_clause().map(|n| n.location()))
            .or_else(|| begin.else_clause().map(|n| n.location()))
            .or_else(|| begin.ensure_clause().map(|n| n.location()));
        if let Some(first) = first {
            return first.start_offset();
        }
    }
    body.location().start_offset()
}

/// One finding: its byte range, message and canonical edit.
struct Finding {
    range: Range<usize>,
    message: &'static str,
    insertion: bool,
}

/// RuboCop's `check_no_space`: report `start..end` unless it is empty or
/// holds a line break.
fn no_space(bytes: &[u8], start: usize, end: usize, message: &'static str, out: &mut Vec<Finding>) {
    if start >= end || bytes[start..end].contains(&b'\n') {
        return;
    }
    out.push(Finding {
        range: start..end,
        message,
        insertion: false,
    });
}

/// RuboCop's `check_arg`: the elements of a destructuring parameter first,
/// then the blanks beyond one before the parameter itself.
fn check_arg(bytes: &[u8], arg: &Node<'_>, out: &mut Vec<Finding>) {
    for element in elements(arg) {
        check_arg(bytes, &element, out);
    }
    let begin = arg.location().start_offset();
    // `range_with_surrounding_space(side: :left)`: spaces and tabs, then
    // line breaks.
    let mut left = begin;
    while left > 0 && matches!(bytes[left - 1], b' ' | b'\t') {
        left -= 1;
    }
    while left > 0 && bytes[left - 1] == b'\n' {
        left -= 1;
    }
    no_space(bytes, left, begin.saturating_sub(1), EXTRA, out);
}

impl Cop for SpaceAroundBlockParameters {
    fn name(&self) -> &'static str {
        "Layout/SpaceAroundBlockParameters"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn with_space_inside_parens(&self, enabled: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            space_inside_parens: enabled,
        }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (parameters, body) = match node {
            Node::BlockNode { .. } => {
                let block = node.as_block_node().unwrap();
                (block.parameters(), block.body())
            }
            Node::LambdaNode { .. } => {
                let lambda = node.as_lambda_node().unwrap();
                (lambda.parameters(), lambda.body())
            }
            _ => return,
        };
        let Some(parameters) = parameters.and_then(|p| p.as_block_parameters_node()) else {
            return;
        };
        let (Some(opening), Some(closing)) = (parameters.opening_loc(), parameters.closing_loc())
        else {
            return;
        };
        let args = arguments(&parameters);
        let (Some(first), Some(last)) = (args.first(), args.last()) else {
            return;
        };
        let bytes = &file.bytes[..];
        let mut found = Vec::new();
        // Lambda parentheses belong to a configured Layout/SpaceInsideParens.
        let parens_checked_elsewhere = opening.as_slice() == b"(" && self.space_inside_parens;
        if !parens_checked_elsewhere {
            no_space(
                bytes,
                opening.end_offset(),
                first.location().start_offset(),
                BEFORE,
                &mut found,
            );
            // `last_end_pos_inside_pipes`: after a trailing comma.
            let mut last_end = last.location().end_offset();
            if let Some(comma) = bytes[last_end..closing.end_offset()]
                .iter()
                .position(|&b| b == b',')
            {
                last_end += comma + 1;
            }
            no_space(bytes, last_end, closing.start_offset(), AFTER, &mut found);
        }
        if let Some(body) = body
            && closing.end_offset() == body_start(&body)
        {
            found.push(Finding {
                range: closing.start_offset()..closing.end_offset(),
                message: CLOSING,
                insertion: true,
            });
        }
        for arg in &args {
            check_arg(bytes, arg, &mut found);
        }

        let correct = cx.corrections_requested();
        for finding in &found {
            let nested = found.iter().any(|other| {
                !other.insertion
                    && other.range != finding.range
                    && other.range.start <= finding.range.start
                    && finding.range.end <= other.range.end
            });
            let correction = if !correct || nested {
                None
            } else if finding.insertion {
                Some(Correction {
                    range: finding.range.end..finding.range.end,
                    replacement: " ".into(),
                })
            } else if bytes[finding.range.clone()]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t'))
            {
                Some(Correction {
                    range: finding.range.clone(),
                    replacement: String::new(),
                })
            } else {
                None
            };
            let (line, col) = file.line_col(finding.range.start);
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: finding.message.into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests;
