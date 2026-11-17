//! `Layout/BlockEndNewline` — the `end` or `}` of a multi-line block starts
//! its own line.
//!
//! A port of RuboCop's `on_block`/`on_numblock`/`on_itblock` (97f30c4, MIT)
//! over every parser block of the Prism translation (`parser_block`): a
//! multi-line block whose `loc.end` does not begin its line is reported
//! there (`Expression at L, C should be on its own line.`, with the 1-based
//! column), unless the text from the end of the block's last child (the
//! body, else the written parameters) to `loc.end` starts with `;` after
//! leading whitespace. Where the parser's last child is a location-less
//! `args` node (no parameters and no body) upstream raises, and nothing is
//! reported.
//!
//! The correction is upstream's: that text is replaced by a line break and
//! itself without its leading whitespace, or, when the body's call chain has
//! a heredoc argument (`last_heredoc_argument`: the last single-line-body
//! heredoc `str` argument of the body call, else of its receivers), the text
//! is removed and the line break and stripped text are inserted after that
//! heredoc's terminator, as one contiguous edit. The edit is withheld for
//! the listed source observers, when the text holds a comment,
//! when a heredoc opened on `loc.end`'s line before it is not exactly the
//! one upstream moves past (the moved delimiter would land inside a
//! heredoc body: upstream breaks the program there), when that heredoc was
//! opened on an earlier line (upstream would move the delimiter before the
//! rest of the body), when code follows the delimiter on its line (it would
//! stay behind on the heredoc's line, continuing the body), when a lambda
//! has no body (the text then starts before `loc.begin`), and when the
//! touched
//! lines hold a `rubocop:`/`rustocop:` directive. `__LINE__`, `DATA` and
//! heredoc boundaries are held by the shared `run_cops` guards.
use std::ops::Range;

use ruby_prism::Node;

use super::{
    heredoc::{call_type, ruby_space},
    parser_block::{ParserBlock, line_break},
};
use crate::{
    cops::{
        Cop, NodeContext,
        style::correction::touches_directive,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct BlockEndNewline;

impl Cop for BlockEndNewline {
    fn name(&self) -> &'static str {
        "Layout/BlockEndNewline"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(block) = ParserBlock::of(node, cx) else {
            return;
        };
        let closing = block.closing.clone();
        // `node.single_line?`: rubocop-ast's `BlockNode` compares the lines
        // of `loc.begin` and `loc.end`, not the call's start.
        if file.line_of(block.opening.start) == file.line_of(closing.start) {
            return;
        }
        // `begins_its_line?(node.loc.end)`.
        let line_start = file.line_start(file.line_of(closing.start));
        if file.bytes[line_start..closing.start]
            .iter()
            .all(|&b| ruby_space(b))
        {
            return;
        }
        let Some(last_end) = block.last_child_end() else {
            return;
        };
        let offense_range = last_end..closing.end;
        let stripped = lstrip(&file.bytes[offense_range.clone()]);
        if stripped.starts_with(b";") {
            return;
        }
        let (line, col) = file.line_col(closing.start);
        let correction = cx
            .corrections_requested()
            .then(|| correction(&block, &offense_range, stripped, cx, file))
            .flatten();
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Expression at {line}, {col} should be on its own line."),
            line,
            end_line: line,
            col,
        });
    }
}

/// Ruby's `String#lstrip`: leading whitespace and NULs.
fn lstrip(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|&b| !(ruby_space(b) || b == 0))
        .unwrap_or(bytes.len());
    &bytes[start..]
}

/// A heredoc opener and the end of its terminator line's text (Prism's
/// `closing_loc` without its line break: the parser's `heredoc_end`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Heredoc {
    opening: usize,
    end: usize,
}

fn heredoc_of(file: &SourceFile, node: &Node<'_>) -> Option<Heredoc> {
    let (opening, closing) = match node {
        Node::StringNode { .. } => {
            let n = node.as_string_node()?;
            (n.opening_loc()?, n.closing_loc()?)
        }
        Node::InterpolatedStringNode { .. } => {
            let n = node.as_interpolated_string_node()?;
            (n.opening_loc()?, n.closing_loc()?)
        }
        Node::XStringNode { .. } => {
            let n = node.as_x_string_node()?;
            (n.opening_loc(), n.closing_loc())
        }
        Node::InterpolatedXStringNode { .. } => {
            let n = node.as_interpolated_x_string_node()?;
            (n.opening_loc(), n.closing_loc())
        }
        _ => return None,
    };
    if !opening.as_slice().starts_with(b"<<") {
        return None;
    }
    let mut end = closing.end_offset();
    while end > closing.start_offset() && matches!(file.bytes[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    Some(Heredoc {
        opening: opening.start_offset(),
        end,
    })
}

/// A heredoc the parser translation emits as a `str` (`str_type? &&
/// heredoc?`): a `StringNode` whose body is exactly one line (an empty or
/// multi-line body is a `dstr`).
fn str_heredoc(file: &SourceFile, node: &Node<'_>) -> Option<Heredoc> {
    let string = node.as_string_node()?;
    let heredoc = heredoc_of(file, node)?;
    let content = string.content_loc();
    let closing = string.closing_loc()?;
    (!content.as_slice().is_empty()
        && file.line_of(closing.start_offset()) == file.line_of(content.start_offset()) + 1)
        .then_some(heredoc)
}

/// `last_heredoc_argument(node.body)`: the last `str` heredoc argument of
/// the body when it is a single `send`/`csend`, else of its receiver,
/// recursively.
fn last_heredoc_argument(file: &SourceFile, body: &Node<'_>) -> Option<Heredoc> {
    let statements = body.as_statements_node()?;
    let mut statements = statements.body().iter();
    let (Some(statement), None) = (statements.next(), statements.next()) else {
        return None;
    };
    let mut node = statement;
    loop {
        call_type(&node)?;
        let call = node.as_call_node()?;
        // A call with a literal block is a parser `block`, not a `send`.
        if call.block().is_some_and(|b| b.as_block_node().is_some()) {
            return None;
        }
        let found = call
            .arguments()
            .into_iter()
            .flat_map(|a| a.arguments().iter())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .find_map(|argument| str_heredoc(file, &argument));
        if found.is_some() {
            return found;
        }
        node = call.receiver()?;
    }
}

/// Every heredoc of a subtree.
struct Heredocs<'f> {
    file: &'f SourceFile,
    found: Vec<Heredoc>,
}

impl<'pr> NodeVisitor<'pr> for Heredocs<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(heredoc) = heredoc_of(self.file, &node) {
            self.found.push(heredoc);
        }
    }
}

/// Whether a comment overlaps `span`.
fn comment_in(file: &SourceFile, span: &Range<usize>) -> bool {
    let comments = file.comments();
    let first = comments.partition_point(|c| c.end <= span.start);
    comments.get(first).is_some_and(|c| c.start < span.end)
}

/// Upstream's edit as one contiguous replacement, or `None` when withheld.
fn correction(
    block: &ParserBlock<'_>,
    offense_range: &Range<usize>,
    stripped: &[u8],
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    if comment_in(file, offense_range) || touches_directive(file, offense_range.clone()) {
        return None;
    }
    let closing_line_start = file.line_start(file.line_of(block.closing.start));
    // Heredocs opened on `loc.end`'s line before it: their bodies follow
    // that line, so the moved delimiter must go after the last of them.
    let pending: Vec<Heredoc> = if file.heredoc_pending_at(root, block.closing.start) {
        let mut all = Heredocs {
            file,
            found: Vec::new(),
        };
        if let Some(body) = &block.body {
            walk(&mut all, body);
        }
        if let Some(parameters) = &block.parameters {
            walk(&mut all, parameters);
        }
        all.found
            .into_iter()
            .filter(|h| h.opening >= closing_line_start && h.opening < block.closing.start)
            .collect()
    } else {
        Vec::new()
    };
    let moved = block
        .body
        .as_ref()
        .and_then(|body| last_heredoc_argument(file, body));
    // A lambda without a body: its parameters precede `loc.begin`, which
    // upstream would move onto a line of its own.
    if offense_range.start < block.opening.start {
        return None;
    }
    let mut replacement = Vec::new();
    let range = match moved {
        Some(heredoc) => {
            // What follows the delimiter on its line would stay behind, now
            // continuing the body instead of the block.
            let line_end =
                closing_line_start + file.line_text(file.line_of(block.closing.start)).len();
            if !file.bytes[block.closing.end..line_end]
                .iter()
                .all(|&b| ruby_space(b))
            {
                return None;
            }
            // Upstream moves past this heredoc even when it was opened on an
            // earlier line, which would move the delimiter before the rest of
            // the body.
            if !pending.contains(&heredoc) || pending.iter().any(|h| h.end > heredoc.end) {
                return None;
            }
            replacement.extend_from_slice(&file.bytes[offense_range.end..heredoc.end]);
            offense_range.start..heredoc.end
        }
        None if !pending.is_empty() => return None,
        None => offense_range.clone(),
    };
    replacement.extend_from_slice(line_break(file, range.end).as_bytes());
    replacement.extend_from_slice(stripped);
    Some(Correction {
        range,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests;
