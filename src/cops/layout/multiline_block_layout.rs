//! `Layout/MultilineBlockLayout` — a multi-line block's parameters sit on
//! the line of its opening `do`/`{`, and its body starts on a later line.
//!
//! A port of RuboCop's `on_block`/`on_numblock`/`on_itblock` (97f30c4, MIT)
//! over every parser block of the Prism translation (`parser_block`):
//! calls and `super` with a literal block and `->` lambdas, with written,
//! numbered or `it` parameters. A multi-line block whose written parameters
//! end on another line than `loc.begin` is reported at the parameters
//! (`Block argument expression is not on the same line as the block
//! start.`) unless joining them onto that line would exceed
//! `Layout/LineLength`'s `Max` (120, or the configured one;
//! `needed_length_for_args`, with `block_arg_string` and its trailing comma
//! rule), and a body starting on `loc.begin`'s line is reported at the body
//! (`Block body expression is on the same line as the block start.`).
//!
//! The correction is upstream's single `autocorrect` (the same for both
//! findings): the parameters are rewritten as ` |a, (b, c)|` right after
//! `loc.begin`, replacing the spaces after them too, and a body on the line
//! of `loc.begin` (or of the rewritten parameters) gets `"\n  "` and the
//! block's start column in spaces before it. Both edits are one contiguous
//! replacement. The edit is withheld for the listed source
//! observers, for a lambda whose parameters are rewritten (they precede
//! `loc.begin`, so upstream's range is reversed), when the rewritten span
//! holds anything but parameters, blanks, line continuations, `|`, `,` and
//! destructuring parentheses (comments, block-local `;` variables, which
//! upstream would turn into parameters), when a kept parameter source is not
//! UTF-8 (a BINARY or legacy byte the rewrite cannot copy byte for byte;
//! ASCII and UTF-8 parameters in such files are still rewritten), when it
//! holds a heredoc, when
//! upstream's trailing comma rule disagrees with a real trailing comma
//! (`|a, b,|`, `|a = [1, 2]|`, nested `(a,)`), when the inserted line break
//! would follow a heredoc opener on its line, and when either edit touches a
//! line holding a `rubocop:`/`rustocop:` directive. `__LINE__`, `DATA` and
//! heredoc boundaries are held by the shared `run_cops` guards.
//!
//! Upstream's `autocorrect_incompatible_with` (`Style/HashEachMethods`,
//! `Style/ItBlockParameter`, `Style/Lambda`) only orders RuboCop's
//! correction rounds and is not declared, as decided for
//! `Layout/ExtraSpacing` and the brace layout cops: all three peers are
//! default-on, and rustocop withholds a cop's edits whenever a declared
//! companion is selected, so the declaration would withhold every edit in
//! the default selection. A peer edit of the same parameters overlaps the
//! rewrite (one `--fix` pass applies one of them), the body line break is
//! inserted where none of them edits, and a lambda's parameters are never
//! rewritten here.
use std::ops::Range;

use ruby_prism::Node;

use super::{
    heredoc::ruby_space,
    parser_block::{ParserBlock, line_break},
};
use crate::{
    cops::{
        Cop, NodeContext,
        style::correction::combine,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct MultilineBlockLayout {
    max_line_length: Option<usize>,
}

impl Default for MultilineBlockLayout {
    fn default() -> Self {
        Self {
            max_line_length: Some(120),
        }
    }
}

const MSG: &str = "Block body expression is on the same line as the block start.";
const ARG_MSG: &str = "Block argument expression is not on the same line as the block start.";

impl Cop for MultilineBlockLayout {
    fn name(&self) -> &'static str {
        "Layout/MultilineBlockLayout"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn with_line_length_limit(&self, max: Option<usize>) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            max_line_length: max,
        }))
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
        // `node.single_line?`: rubocop-ast's `BlockNode` compares the lines
        // of `loc.begin` and `loc.end`, not the call's start.
        let begin_line = file.line_of(block.opening.start);
        if begin_line == file.line_of(block.closing.start) {
            return;
        }
        let arguments = block.arguments();
        let misplaced_args = arguments.as_ref().filter(|args| {
            file.last_line_of(args.location()) != begin_line
                && !self.line_break_necessary_in_args(&block, args, file)
        });
        if let Some(args) = misplaced_args {
            self.report(&block, range(args), ARG_MSG, cx, file, out);
        }
        if let Some(body) = block.body_range()
            && file.line_of(body.start) == begin_line
        {
            self.report(&block, body, MSG, cx, file, out);
        }
    }
}

fn range(node: &Node<'_>) -> Range<usize> {
    let location = node.location();
    location.start_offset()..location.end_offset()
}

impl MultilineBlockLayout {
    /// `line_break_necessary_in_args?`.
    fn line_break_necessary_in_args(
        &self,
        block: &ParserBlock<'_>,
        args: &Node<'_>,
        file: &SourceFile,
    ) -> bool {
        let Some(max) = self.max_line_length else {
            return false;
        };
        // `node.source.lines.first`, with its line break.
        let first_line_end = file.line_start(file.line_of(block.start) + 1);
        let first_line = &file.bytes[block.start..first_line_end.max(block.start)];
        let pipes = if first_line.ends_with(b"|\n") { 1 } else { 3 };
        let chomped = first_line
            .strip_suffix(b"\n")
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
            .or_else(|| first_line.strip_suffix(b"\r"))
            .unwrap_or(first_line);
        let column = file.line_col(block.start).1 as usize - 1;
        let needed =
            column + pipes + chars(chomped) + chars(ArgString::of(args, file).upstream.as_bytes());
        needed > max
    }

    fn report(
        &self,
        block: &ParserBlock<'_>,
        expression: Range<usize>,
        message: &str,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let correction = cx
            .corrections_requested()
            .then(|| correction(block, cx, file))
            .flatten();
        let (line, col) = file.line_col(expression.start);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: message.to_owned(),
            line,
            end_line: file.line_of(expression.end.saturating_sub(1).max(expression.start)),
            col,
        });
    }
}

/// The number of characters (UTF-8 scalar values; other bytes count one).
fn chars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b & 0xc0 != 0x80).count()
}

/// `block_arg_string` for the parameters, as upstream builds it, and facts
/// for deciding whether that rewrite is exact.
struct ArgString {
    /// Upstream's string: each parser argument's source (block-local
    /// variables included), destructuring as `(...)`, joined by `, `, with
    /// a `,` appended at every level by `include_trailing_comma?`.
    upstream: String,
    /// Upstream's `include_trailing_comma?`: exactly one argument-type
    /// descendant and a `,` anywhere in the parameters' source.
    trailing_comma: bool,
    /// The parameter source ranges the rewrite keeps.
    kept: Vec<Range<usize>>,
    /// A Prism `ImplicitRestNode` (a real trailing comma) at the top level.
    implicit_rest: bool,
    /// An implicit rest anywhere else (no upstream spelling keeps it).
    nested_rest: bool,
    /// Destructuring, where upstream appends its comma at every level.
    destructuring: bool,
    /// Block-local (`;`) variables.
    locals: bool,
    /// A kept parameter source that is not UTF-8 (a BINARY or legacy byte):
    /// the `String` rewrite cannot copy it byte for byte.
    not_utf8: bool,
}

/// Counts upstream's `argument_type?` descendants and Prism implicit rests.
#[derive(Default)]
struct Census {
    arguments: usize,
    implicit_rests: usize,
}

impl<'pr> NodeVisitor<'pr> for Census {
    fn enter(&mut self, node: Node<'pr>) {
        match node {
            Node::RequiredParameterNode { .. }
            | Node::OptionalParameterNode { .. }
            | Node::RestParameterNode { .. }
            | Node::RequiredKeywordParameterNode { .. }
            | Node::OptionalKeywordParameterNode { .. }
            | Node::KeywordRestParameterNode { .. }
            | Node::BlockParameterNode { .. }
            | Node::ForwardingParameterNode { .. }
            | Node::BlockLocalVariableNode { .. } => self.arguments += 1,
            // A destructuring splat is a parser `restarg`.
            Node::MultiTargetNode { .. } => {
                let target = node.as_multi_target_node().unwrap();
                if target.rest().is_some_and(|r| r.as_splat_node().is_some()) {
                    self.arguments += 1;
                }
            }
            Node::ImplicitRestNode { .. } => self.implicit_rests += 1,
            _ => {}
        }
    }
}

impl ArgString {
    fn of(args: &Node<'_>, file: &SourceFile) -> Self {
        let mut census = Census::default();
        walk(&mut census, args);
        let source = &file.bytes[range(args)];
        let trailing_comma = census.arguments == 1 && source.contains(&b',');
        let mut this = Self {
            upstream: String::new(),
            trailing_comma,
            kept: Vec::new(),
            implicit_rest: false,
            nested_rest: false,
            destructuring: false,
            locals: false,
            not_utf8: false,
        };
        let Some(params) = args.as_block_parameters_node() else {
            return this;
        };
        let mut children: Vec<Node<'_>> = Vec::new();
        if let Some(p) = params.parameters() {
            children.extend(p.requireds().iter());
            children.extend(p.optionals().iter());
            children.extend(p.rest());
            children.extend(p.posts().iter());
            children.extend(p.keywords().iter());
            children.extend(p.keyword_rest());
            children.extend(p.block().map(|b| b.as_node()));
        }
        this.locals = params.locals().iter().next().is_some();
        children.extend(params.locals().iter());
        children.sort_by_key(|c| c.location().start_offset());
        this.upstream = this.join(&children, file, true);
        this.nested_rest = census.implicit_rests > usize::from(this.implicit_rest);
        this
    }

    fn join(&mut self, children: &[Node<'_>], file: &SourceFile, top: bool) -> String {
        let mut parts = Vec::new();
        for child in children {
            if let Some(target) = child.as_multi_target_node() {
                self.destructuring = true;
                let mut inner: Vec<Node<'_>> = target.lefts().iter().collect();
                inner.extend(target.rest());
                inner.extend(target.rights().iter());
                inner.sort_by_key(|c| c.location().start_offset());
                parts.push(format!("({})", self.join(&inner, file, false)));
            } else if child.as_implicit_rest_node().is_some() {
                if top {
                    self.implicit_rest = true;
                }
            } else {
                let r = range(child);
                let bytes = &file.bytes[r.clone()];
                // A lossy copy still measures `needed_length_for_args`, but
                // `exact` then withholds the rewrite: U+FFFD would replace
                // the byte.
                self.not_utf8 |= std::str::from_utf8(bytes).is_err();
                parts.push(String::from_utf8_lossy(bytes).into_owned());
                self.kept.push(r);
            }
        }
        let mut joined = parts.join(", ");
        if self.trailing_comma {
            joined.push(',');
        }
        joined
    }

    /// Whether upstream's rewrite spells exactly the parameters as written:
    /// no block-local variables, every kept source byte for byte, and a
    /// trailing comma exactly where Ruby has one.
    fn exact(&self) -> bool {
        !self.locals
            && !self.not_utf8
            && !self.nested_rest
            && self.trailing_comma == self.implicit_rest
            && !(self.trailing_comma && self.destructuring)
    }
}

/// Upstream's `autocorrect`, as one contiguous edit, or `None` when it is
/// withheld.
fn correction(
    block: &ParserBlock<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let begin_line = file.line_of(block.opening.start);
    let mut edits = Vec::new();
    let mut before_body_line = begin_line;
    if let Some(args) = block.arguments()
        && file.last_line_of(args.location()) != begin_line
    {
        if block.lambda {
            return None;
        }
        let args_range = range(&args);
        let mut end = args_range.end;
        while matches!(file.bytes.get(end), Some(b' ' | b'\t')) {
            end += 1;
        }
        let span = block.opening.end..end;
        let string = ArgString::of(&args, file);
        if !string.exact()
            || !only_layout_between(file, &span, &string.kept)
            || file.heredoc_opening_in(root, span.clone())
        {
            return None;
        }
        edits.push(Correction {
            range: span,
            replacement: format!(" |{}|", string.upstream),
        });
        before_body_line = file.last_line_of(args.location());
    }
    if let Some(body) = block.body_range()
        && file.line_of(body.start) == before_body_line
    {
        if file.heredoc_pending_at(root, body.start) {
            return None;
        }
        let column = file.line_col(block.start).1 as usize - 1;
        edits.push(Correction {
            range: body.start..body.start,
            replacement: format!("{}  {}", line_break(file, body.start), " ".repeat(column)),
        });
    }
    combine(file, edits)
}

/// Whether `span` holds nothing but the `kept` parameter sources, Ruby
/// whitespace, line continuations, `|`, `,` and parentheses.
fn only_layout_between(file: &SourceFile, span: &Range<usize>, kept: &[Range<usize>]) -> bool {
    let mut kept: Vec<_> = kept.to_vec();
    kept.sort_by_key(|r| r.start);
    let mut cursor = span.start;
    let mut gaps = Vec::new();
    for r in &kept {
        if r.start < cursor || r.end > span.end {
            return false;
        }
        gaps.push(cursor..r.start);
        cursor = r.end;
    }
    gaps.push(cursor..span.end);
    gaps.iter().all(|gap| {
        let bytes = &file.bytes[gap.clone()];
        bytes.iter().enumerate().all(|(i, &b)| {
            ruby_space(b)
                || matches!(b, b'|' | b',' | b'(' | b')')
                || b == b'\\' && bytes.get(i + 1) == Some(&b'\n')
        })
    }) && !comment_in(file, span)
}

/// Whether a comment overlaps `span`.
fn comment_in(file: &SourceFile, span: &Range<usize>) -> bool {
    let comments = file.comments();
    let first = comments.partition_point(|c| c.end <= span.start);
    comments.get(first).is_some_and(|c| c.start < span.end)
}

#[cfg(test)]
mod tests;
