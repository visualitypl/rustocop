//! `Layout/LineEndStringConcatenationIndentation` — the parts of a string
//! concatenated with backslashes line up.
//!
//! A port of RuboCop's `on_dstr` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: aligned` and indentation width 2. A parser `dstr` that
//! spans lines, whose children are all `str`/`dstr` and each on one line, is
//! an implicit concatenation of adjacent string literals: an
//! `InterpolatedStringNode` without an opening (a delimited or heredoc
//! string has an interpolation child, a multi-line piece or a one-line
//! opener). Its parts are the children; columns are RuboCop's character
//! columns.
//!
//! - Usually (`check_aligned` from the second part) the parts are split into
//!   runs of consecutive parts in one column; a run is misaligned when its
//!   column is neither the previous part's nor the first part's, and its first
//!   part is reported (`Align parts of …`).
//! - When the parser parent is none, a `block`, `begin`, `def`, `defs` or
//!   `if` (an implicit return value, an ordinary statement among others, a
//!   branch, a parenthesized or interpolated expression), the second part
//!   must be indented two columns more than the first non-blank character of
//!   the first part's line (`Indent the first part of …`), and the later
//!   parts are aligned with the second part or that column. A sole statement's
//!   parser parent is its enclosing node; several statements have a `begin`,
//!   except directly in a `begin … end` without `rescue`/`ensure`, which is a
//!   `kwbegin`. Numbered-parameter and `it` blocks are `numblock`/`itblock`.
//!
//! The correction is `AlignmentCorrector.correct` for each corrected part:
//! the missing blanks are inserted right before the part, or the extra
//! columns removed right before it when they are spaces and tabs. One run's
//! edits form one correction, copied verbatim between the edits. It is
//! withheld for a part that does not begin its line (blanks between two parts
//! on one line), when an edit is inside literal text (an interpolation of a
//! heredoc), when the copied bytes are not UTF-8, and for the listed
//! source observers. Only blanks before a string part that begins its line,
//! after a line continuation, change: no token, literal byte or line changes.
use std::ops::Range;

use ruby_prism::Node;

use super::alignment::effective_column;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct LineEndStringConcatenationIndentation;

const MSG_ALIGN: &str = "Align parts of a string concatenated with backslash.";
const MSG_INDENT: &str = "Indent the first part of a string concatenated with backslash.";
/// Layout/IndentationWidth's fixed width; this cop's `IndentationWidth` is
/// unset.
const WIDTH: i64 = 2;

struct Part {
    range: Range<usize>,
    column: i64,
}

impl Cop for LineEndStringConcatenationIndentation {
    fn name(&self) -> &'static str {
        "Layout/LineEndStringConcatenationIndentation"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_interpolated_string_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(dstr) = node.as_interpolated_string_node() else {
            return;
        };
        let location = dstr.location();
        if dstr.opening_loc().is_some()
            || file.line_of(location.start_offset()) == file.last_line_of(location)
        {
            return;
        }
        let mut parts = Vec::new();
        for part in dstr.parts().iter() {
            if !matches!(
                part,
                Node::StringNode { .. } | Node::InterpolatedStringNode { .. }
            ) {
                return;
            }
            let location = part.location();
            let range = location.start_offset()..location.end_offset();
            if file.line_of(range.start) != file.last_line_of(location) {
                return;
            }
            parts.push(Part {
                column: effective_column(file, range.start) as i64,
                range,
            });
        }
        if parts.len() < 2 {
            return;
        }
        let mut found = Vec::new();
        if always_indented(cx) {
            let indented = base_column(file, parts[0].range.start) + WIDTH;
            let delta = indented - parts[1].column;
            if delta != 0 {
                found.push((MSG_INDENT, &parts[1..2], delta));
            }
            check_aligned(&parts, 2, indented, &mut found);
        } else {
            check_aligned(&parts, 1, parts[0].column, &mut found);
        }
        for (message, run, delta) in found {
            let first = &run[0];
            let (line, col) = file.line_col(first.range.start);
            let correction = cx
                .corrections_requested()
                .then(|| correction(file, cx, run, delta))
                .flatten();
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: message.into(),
                line,
                end_line: file.line_of(first.range.end.saturating_sub(1).max(first.range.start)),
                col,
            });
        }
    }
}

/// `check_aligned`: runs of parts in one column, from `start`, that are
/// neither in the previous part's column nor in `target`.
fn check_aligned<'a>(
    parts: &'a [Part],
    start: usize,
    target: i64,
    found: &mut Vec<(&'static str, &'a [Part], i64)>,
) {
    let Some(previous) = start.checked_sub(1).and_then(|i| parts.get(i)) else {
        return;
    };
    let mut base = previous.column;
    let mut rest = parts.get(start..).unwrap_or_default();
    while let Some(first) = rest.first() {
        let len = rest
            .iter()
            .take_while(|part| part.column == first.column)
            .count();
        let (run, tail) = rest.split_at(len);
        if first.column != base && first.column != target {
            found.push((MSG_ALIGN, run, target - first.column));
        }
        base = first.column;
        rest = tail;
    }
}

/// `base_column`: the column of the first non-blank character (Ruby's `\S`)
/// of the line holding `offset`.
fn base_column(file: &SourceFile, offset: usize) -> i64 {
    let line = file.line_of(offset);
    let mut start = file.line_start(line);
    if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
        start = 3;
    }
    let first = file.bytes[start..offset]
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c'))
        .map_or(offset, |i| start + i);
    effective_column(file, first) as i64
}

/// Whether the `dstr`'s parser parent is in `PARENT_TYPES_FOR_INDENTED`
/// (`nil`, `block`, `begin`, `def`, `defs`, `if`).
fn always_indented(cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors();
    let Some(parent) = ancestors.next() else {
        return true;
    };
    let Some(statements) = parent.as_statements_node() else {
        // A direct child: a condition, or an unwrapped body.
        return matches!(
            parent,
            Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::ParenthesesNode { .. }
                | Node::DefNode { .. }
        );
    };
    let owner = ancestors.next();
    if statements.body().iter().nth(1).is_some() {
        // Several statements are a `begin`, except in a plain `kwbegin`.
        return !owner.is_some_and(plain_kwbegin);
    }
    let Some(owner) = owner else {
        return true;
    };
    match owner {
        Node::ProgramNode { .. }
        | Node::DefNode { .. }
        | Node::IfNode { .. }
        | Node::UnlessNode { .. }
        | Node::ParenthesesNode { .. }
        | Node::EmbeddedStatementsNode { .. } => true,
        Node::BlockNode { .. } => owner
            .as_block_node()
            .is_some_and(|block| block_parameters(block.parameters())),
        Node::LambdaNode { .. } => owner
            .as_lambda_node()
            .is_some_and(|lambda| block_parameters(lambda.parameters())),
        Node::ElseNode { .. } => ancestors
            .next()
            .is_some_and(|n| matches!(n, Node::IfNode { .. } | Node::UnlessNode { .. })),
        _ => false,
    }
}

/// A parser `block` rather than a `numblock` or `itblock`.
fn block_parameters(parameters: Option<Node<'_>>) -> bool {
    !matches!(
        parameters,
        Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
    )
}

/// `begin … end` without `rescue`, `else` or `ensure`: parser's `kwbegin`
/// holds the statements directly.
fn plain_kwbegin(node: &Node<'_>) -> bool {
    node.as_begin_node().is_some_and(|begin| {
        begin.begin_keyword_loc().is_some()
            && begin.rescue_clause().is_none()
            && begin.else_clause().is_none()
            && begin.ensure_clause().is_none()
    })
}

/// `AlignmentCorrector.correct` for each part of `run`, as one edit.
fn correction(
    file: &SourceFile,
    cx: &NodeContext<'_, '_>,
    run: &[Part],
    delta: i64,
) -> Option<Correction> {
    let root = cx.ancestors().last()?;
    let mut edits: Vec<(Range<usize>, usize)> = Vec::new();
    for part in run {
        let start = part.range.start;
        if !super::alignment::starts_line(file, start) {
            return None;
        }
        let edit = if delta > 0 {
            (start..start, delta as usize)
        } else {
            let remove = delta.unsigned_abs() as usize;
            let line_start = file.line_start(file.line_of(start));
            let Some(from) = start.checked_sub(remove).filter(|&f| f >= line_start) else {
                continue;
            };
            if !file.bytes[from..start]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t'))
            {
                continue;
            }
            (from..start, 0)
        };
        if file.literal_interior_at(root, edit.0.start) || file.overlaps_literal(root, &edit.0) {
            return None;
        }
        edits.push(edit);
    }
    let (first, last) = (edits.first()?.0.start, edits.last()?.0.end);
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
    {
        return None;
    }
    let mut replacement = Vec::new();
    let mut cursor = first;
    for (range, spaces) in edits {
        replacement.extend_from_slice(&file.bytes[cursor..range.start]);
        replacement.extend(std::iter::repeat_n(b' ', spaces));
        cursor = range.end;
    }
    replacement.extend_from_slice(&file.bytes[cursor..last]);
    Some(Correction {
        range: first..last,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests;
