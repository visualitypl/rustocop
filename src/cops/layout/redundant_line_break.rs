//! `Layout/RedundantLineBreak` — an expression broken over several lines
//! that fits on one (opt-in).
//!
//! A port of RuboCop's cop (97f30c4, MIT) at the fixed `InspectBlocks:
//! false`. RuboCop keeps per-file state across its callbacks (ignored
//! offense nodes and the line ranges of earlier offenses), so the cop walks
//! the tree once from the root in RuboCop's callback order, as
//! `Style/RedundantParentheses` does, viewing Prism's tree as the parser
//! gem's: a call with a literal block is a `block` whose `send` excludes the
//! block, a lambda is a `block` with a `lambda` send, an index is a `send
//! :[]`.
//!
//! Every assignment (`on_lvasgn` and its aliases: variables, constants,
//! multiple and operator assignments) is a candidate, except a local
//! variable assignment in a file ending in `"%\n\n"`. Every `send`/`csend`
//! climbs to "the whole expression": while its parent is a `send` (as
//! receiver or argument), an `and`/`or`, or a block whose send it is and
//! that send has parentheses or no arguments. A candidate is reported when
//! it spans lines, its lines joined with `to_single_line` fit in
//! `Layout/LineLength`'s `max` (the configured one, 120 by default; none when
//! that cop is configured off), no comment starts on its lines, it holds no
//! `if`/`case`/`begin`/`def`/`rescue`/`ensure`, heredoc or string value with
//! a line break, multi-line `begin` or symbol, an `and`/`or` breaks after an
//! operator ending its line in `\`, it is no `[]` call on a `[]` call, it is
//! no block and holds no multi-line block, and (when `Layout/SingleLineBlockChain`
//! is configured enabled) holds no single-line block that a dotted call is
//! chained to. A candidate inside an earlier offense is skipped; one whose
//! joined line would, together with earlier offenses sharing its lines,
//! exceed the `max` is skipped. Finally the exact single-line correction
//! must reparse to the same tree (the shared `cops::reparse`, in the
//! innermost `def`/`class`/`module`/`class << x` or the whole file), with
//! string concatenations folded (`"a" \` + `"b"`, `"a" + 'b'`) and
//! `__LINE__` neutralized on both sides, as upstream's
//! `ReparsedEquivalence`. These are detection facts only.
//!
//! The correction replaces the expression by `to_single_line` of its
//! source, stripped (CRLF line breaks are joined whole, where upstream's
//! chain substitution leaves a stray `\r`). It is withheld, finding kept, for the listed
//! source observers, when the join turns a split string into a `+` call
//! (`"a" \` + `'b'` becomes `"a" + 'b'`: a new, unfrozen string at run
//! time), when a `__LINE__` lies in the joined span (its value would
//! change), and when the touched lines hold a `rubocop:`/`rustocop:`
//! directive. `__LINE__` after the edit, `DATA` and heredoc boundaries are
//! held by the shared `run_cops` guards.
use std::{borrow::Cow, ops::Range};

use ruby_prism::Node;

use crate::{
    cops::{
        Companions, Cop, NodeContext,
        ast::fingerprint_folding_strings,
        reparse::{self, Equivalence},
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct RedundantLineBreak {
    max_line_length: Option<usize>,
    single_line_block_chain: bool,
}

impl Default for RedundantLineBreak {
    fn default() -> Self {
        Self {
            max_line_length: Some(120),
            single_line_block_chain: false,
        }
    }
}

const MSG: &str = "Redundant line break detected.";

impl Cop for RedundantLineBreak {
    fn name(&self) -> &'static str {
        "Layout/RedundantLineBreak"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn default_enabled(&self) -> bool {
        false
    }

    /// Opt-in as upstream: only its exact name enables it.
    fn department_selection_enables(&self) -> bool {
        false
    }

    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            max_line_length: companions.line_length_limit,
            single_line_block_chain: companions.single_line_block_chain,
        }))
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let mut walker = Walker {
            cop: self,
            file,
            root: node,
            corrections: cx.corrections_requested(),
            percent_blank: file.bytes.ends_with(b"%\n\n"),
            stack: Vec::new(),
            ignored: Vec::new(),
            offense_lines: Vec::new(),
            offenses: std::collections::HashMap::new(),
            verifier: None,
            out,
        };
        walk(&mut walker, node);
    }
}

/// A parser-gem node of the Prism tree.
#[derive(Clone, Copy)]
enum Parser<'a, 'pr> {
    /// A `send`/`csend`: the call without its literal block.
    Send(&'a Node<'pr>),
    /// A `block`: a call or `super` with a literal block (the whole call),
    /// or a lambda.
    Block(&'a Node<'pr>),
    /// The `lambda` send of a `->` block.
    LambdaSend(&'a Node<'pr>),
    /// Any other node.
    Other(&'a Node<'pr>),
}

/// A Prism call that the parser translation emits as a `send`/`csend`.
fn is_send(node: &Node<'_>, parent: Option<&Node<'_>>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if parent.is_some_and(|p| p.as_match_write_node().is_some()) {
        // `match_with_lvasgn`.
        return false;
    }
    !(call.name().as_slice() == b"-@"
        && call.receiver().is_some_and(|r| {
            r.as_integer_node().is_some()
                || r.as_float_node().is_some()
                || r.as_rational_node().is_some()
                || r.as_imaginary_node().is_some()
        }))
}

/// The literal block of a call or `super`.
fn literal_block<'pr>(node: &Node<'pr>) -> Option<ruby_prism::BlockNode<'pr>> {
    let block = match node {
        Node::CallNode { .. } => node.as_call_node()?.block()?,
        Node::SuperNode { .. } => node.as_super_node()?.block()?,
        Node::ForwardingSuperNode { .. } => node.as_forwarding_super_node()?.block()?.as_node(),
        _ => return None,
    };
    block.as_block_node()
}

fn location_range(node: &Node<'_>) -> Range<usize> {
    let location = node.location();
    location.start_offset()..location.end_offset()
}

impl<'pr> Parser<'_, 'pr> {
    fn node(&self) -> &Node<'pr> {
        match self {
            Parser::Send(n) | Parser::Block(n) | Parser::LambdaSend(n) | Parser::Other(n) => n,
        }
    }

    /// `node.source_range`.
    fn range(&self) -> Range<usize> {
        match self {
            Parser::Send(node) => {
                let range = location_range(node);
                let Some(block) = literal_block(node) else {
                    return range;
                };
                // The send ends before its literal block.
                let call = node.as_call_node().unwrap();
                let end = [
                    call.closing_loc().map(|l| l.end_offset()),
                    call.arguments().map(|a| a.location().end_offset()),
                    call.message_loc().map(|l| l.end_offset()),
                    call.opening_loc().map(|l| l.end_offset()),
                    call.receiver().map(|r| r.location().end_offset()),
                ]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(block.location().start_offset());
                range.start..end.min(block.location().start_offset())
            }
            Parser::LambdaSend(node) => {
                let operator = node.as_lambda_node().unwrap().operator_loc();
                operator.start_offset()..operator.end_offset()
            }
            Parser::Block(node) | Parser::Other(node) => location_range(node),
        }
    }

    fn is_block(&self) -> bool {
        matches!(self, Parser::Block(_))
    }
}

struct Walker<'a, 'pr> {
    cop: &'a RedundantLineBreak,
    file: &'a SourceFile,
    root: &'a Node<'pr>,
    corrections: bool,
    percent_blank: bool,
    /// The Prism ancestors of the node being visited, outermost first.
    stack: Vec<Node<'pr>>,
    /// Ranges of the offense nodes (`ignore_node`).
    ignored: Vec<Range<usize>>,
    /// `@offense_line_ranges`.
    offense_lines: Vec<(u32, u32)>,
    /// `offense?` by node range and kind: every send of an expression
    /// climbs to the same node, whose answer does not change.
    offenses: std::collections::HashMap<(usize, usize, bool), bool>,
    /// The reparse verification prepared on the first candidate and reused
    /// for the file's others (no root rescan per candidate).
    verifier: Option<reparse::Verifier<'static>>,
    out: &'a mut Vec<Offense>,
}

/// `ReparsedEquivalence` as this cop compares trees.
static EQUIVALENCE: Equivalence = Equivalence {
    fingerprint: fingerprint_folding_strings,
    preprocess: neutralize_line,
};

impl<'pr> NodeVisitor<'pr> for Walker<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        self.stack.push(node);
        let index = self.stack.len() - 1;
        let stack = std::mem::take(&mut self.stack);
        self.visit(&stack, index);
        self.stack = stack;
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

impl<'pr> Walker<'_, 'pr> {
    fn visit<'s>(&mut self, stack: &'s [Node<'pr>], index: usize) {
        let node = &stack[index];
        if crate::cops::ast::is_assignment(node) {
            if !(self.percent_blank && node.as_local_variable_write_node().is_some()) {
                self.check(Parser::Other(node));
            }
            return;
        }
        let start = if node.as_lambda_node().is_some() {
            (Parser::LambdaSend(node), index)
        } else if is_send(node, index.checked_sub(1).map(|i| &stack[i])) {
            (Parser::Send(node), index)
        } else {
            return;
        };
        // `on_send`: include "the whole expression".
        let (mut current, mut at) = start;
        while let Some((parent, parent_at)) = parser_parent(stack, current, at) {
            let climb = match parent {
                Parser::Send(call) => !call.as_call_node().unwrap().is_safe_navigation(),
                Parser::Block(_) => convertible_block(current),
                Parser::Other(n) => n.as_and_node().is_some() || n.as_or_node().is_some(),
                Parser::LambdaSend(_) => false,
            };
            if !climb {
                break;
            }
            current = parent;
            at = parent_at;
        }
        self.check(current);
    }

    /// `offense?(node) && !part_of_ignored_node?(node)`, then
    /// `register_offense`.
    fn check(&mut self, node: Parser<'_, 'pr>) {
        let range = node.range();
        if self
            .ignored
            .iter()
            .any(|ignored| ignored.start <= range.start && range.end <= ignored.end)
        {
            return;
        }
        let key = (range.start, range.end, node.is_block());
        let offense = match self.offenses.get(&key) {
            Some(&offense) => offense,
            None => {
                let offense = self.offense(&node, &range);
                self.offenses.insert(key, offense);
                offense
            }
        };
        if !offense {
            return;
        }
        let file = self.file;
        let first = file.line_of(range.start);
        let last = file.line_of(range.end.saturating_sub(1).max(range.start));
        // `joined_line_range`.
        let (mut low, mut high) = (first, last);
        for &(begin, end) in &self.offense_lines {
            if begin <= last && first <= end {
                low = low.min(begin);
                high = high.max(end);
            }
        }
        if self.cop.too_long(file, low, high) {
            return;
        }
        // Every line break inside a candidate is layout (literals holding
        // one are never split): join CRLF lines whole, where upstream's
        // chain substitution leaves the `\r` of `\r\n` behind.
        let source = &file.bytes[range.clone()];
        let joined = if memchr::memmem::find(source, b"\r\n").is_some() {
            to_single_line(&crlf_to_lf(source))
        } else {
            to_single_line(source)
        };
        let joined = strip(&joined).to_vec();
        if !self.verified(&range, &joined) {
            return;
        }
        let correction = self
            .corrections
            .then(|| self.correction(&range, joined))
            .flatten();
        let (line, col) = file.line_col(range.start);
        self.out.push(Offense {
            correction,
            cop: self.cop.name(),
            severity: self.cop.severity(),
            message: MSG.to_owned(),
            line,
            end_line: last,
            col,
        });
        self.ignored.push(range);
        self.offense_lines
            .retain(|&(begin, end)| !(low <= begin && end <= high));
        self.offense_lines.push((low, high));
    }

    /// `offense?`.
    fn offense(&self, node: &Parser<'_, 'pr>, range: &Range<usize>) -> bool {
        let file = self.file;
        let first = file.line_of(range.start);
        let last = file.line_of(range.end.saturating_sub(1).max(range.start));
        // `node.multiline? && suitable_as_single_line?(node)`.
        if first == last
            || self.cop.too_long(file, first, last)
            || file.has_comment_in_lines(first..last + 1)
        {
            return false;
        }
        let facts = Descendants::of(file, node);
        if !facts.safe_to_split {
            return false;
        }
        // `operator_keyword?`: `require_backslash?`.
        let operator = match node.node() {
            Node::AndNode { .. } => Some(node.node().as_and_node().unwrap().operator_loc()),
            Node::OrNode { .. } => Some(node.node().as_or_node().unwrap().operator_loc()),
            _ => None,
        };
        if let Some(operator) = operator {
            if !matches!(node, Parser::Other(_)) {
                return false;
            }
            return file
                .line_text(file.line_of(operator.start_offset()))
                .ends_with(b"\\");
        }
        if index_access_call_chained(node) {
            return false;
        }
        // `configured_to_not_be_inspected?` at `InspectBlocks: false`.
        !(self.cop.single_line_block_chain && facts.chained_single_line_block
            || node.is_block()
            || facts.multiline_block)
    }

    /// `verified_by_reparse([node], oversized: :verify)`.
    fn verified(&mut self, range: &Range<usize>, joined: &[u8]) -> bool {
        let apply = |fragment: &[u8], start: usize, _: &[usize]| {
            let mut corrected = fragment.to_vec();
            corrected.splice(
                range.start - start..range.end - start,
                joined.iter().copied(),
            );
            Some(corrected)
        };
        let root = self.root;
        !self
            .verifier
            .get_or_insert_with(|| reparse::Verifier::new(root, &EQUIVALENCE))
            .verified(self.file, root, std::slice::from_ref(range), &apply)
            .is_empty()
    }

    fn correction(&self, range: &Range<usize>, joined: Vec<u8>) -> Option<Correction> {
        Some(Correction {
            range: range.clone(),
            replacement: String::from_utf8(joined).ok()?,
        })
    }
}

impl RedundantLineBreak {
    /// `too_long_as_single_line?(first..last)`.
    fn too_long(&self, file: &SourceFile, first: u32, last: u32) -> bool {
        let Some(max) = self.max_line_length else {
            return false;
        };
        let mut lines = Vec::new();
        for line in first..=last {
            if line > first {
                lines.push(b'\n');
            }
            lines.extend_from_slice(file.line_text(line));
        }
        chars(&to_single_line(&lines)) > max
    }
}

/// The number of characters (UTF-8 scalar values; other bytes count one).
fn chars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b & 0xc0 != 0x80).count()
}

/// The parser parent of `node` (whose Prism node is `stack[at]`) and the
/// stack index of the parent's Prism node.
fn parser_parent<'s, 'pr>(
    stack: &'s [Node<'pr>],
    node: Parser<'s, 'pr>,
    at: usize,
) -> Option<(Parser<'s, 'pr>, usize)> {
    match node {
        Parser::Send(call) if literal_block(call).is_some() => {
            return Some((Parser::Block(call), at));
        }
        Parser::LambdaSend(lambda) => return Some((Parser::Block(lambda), at)),
        _ => {}
    }
    let parent = stack.get(at.checked_sub(1)?)?;
    if parent.as_arguments_node().is_some() {
        let owner = stack.get(at.checked_sub(2)?)?;
        let owner_parent = at.checked_sub(3).map(|i| &stack[i]);
        return Some(if is_send(owner, owner_parent) {
            (Parser::Send(owner), at - 2)
        } else {
            (Parser::Other(owner), at - 2)
        });
    }
    let parent_parent = at.checked_sub(2).map(|i| &stack[i]);
    if is_send(parent, parent_parent)
        && parent
            .as_call_node()
            .unwrap()
            .receiver()
            .is_some_and(|r| location_range(&r) == location_range(&stack[at]))
    {
        return Some((Parser::Send(parent), at - 1));
    }
    Some((Parser::Other(parent), at - 1))
}

/// `convertible_block?(node)` for the send of its parent block.
fn convertible_block(node: Parser<'_, '_>) -> bool {
    match node {
        Parser::LambdaSend(_) => true,
        Parser::Send(call) => {
            let call = call.as_call_node().unwrap();
            call.opening_loc().is_some_and(|o| o.as_slice() == b"(") || call.arguments().is_none()
        }
        _ => false,
    }
}

/// `index_access_call_chained?`: a `[]` send on a `[]` send.
fn index_access_call_chained(node: &Parser<'_, '_>) -> bool {
    let Parser::Send(call) = node else {
        return false;
    };
    let call = call.as_call_node().unwrap();
    call.name().as_slice() == b"[]"
        && call.receiver().is_some_and(|r| {
            r.as_call_node()
                .is_some_and(|r| r.name().as_slice() == b"[]" && !r.is_safe_navigation())
        })
}

/// Facts about a parser node's descendants (`each_descendant`).
struct Descendants {
    /// `safe_to_split?`.
    safe_to_split: bool,
    /// A multi-line descendant block.
    multiline_block: bool,
    /// `other_cop_takes_precedence?`'s block: single-line, child of a
    /// dotted `send`/`csend`.
    chained_single_line_block: bool,
}

struct Scan<'a, 'pr> {
    file: &'a SourceFile,
    /// The root's own literal block (a `send`'s descendants exclude it), or
    /// the root of a `block` (not its own descendant).
    skip: Option<Range<usize>>,
    root_block: Option<Range<usize>>,
    stack: Vec<Node<'pr>>,
    /// Depth inside the skipped subtree.
    skipping: usize,
    facts: Descendants,
}

impl Descendants {
    fn of<'pr>(file: &SourceFile, node: &Parser<'_, 'pr>) -> Self {
        let facts = Descendants {
            safe_to_split: true,
            multiline_block: false,
            chained_single_line_block: false,
        };
        let (skip, root_block) = match node {
            Parser::Send(call) => (
                literal_block(call).map(|b| location_range(&b.as_node())),
                None,
            ),
            Parser::LambdaSend(_) => return facts,
            Parser::Block(owner) => (None, Some(location_range(owner))),
            Parser::Other(_) => (None, None),
        };
        let mut scan = Scan {
            file,
            skip,
            root_block,
            stack: Vec::new(),
            skipping: 0,
            facts,
        };
        walk(&mut scan, node.node());
        scan.facts
    }
}

impl<'pr> NodeVisitor<'pr> for Scan<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if self.skipping > 0
            || self.skip.as_ref() == Some(&location_range(&node)) && node.as_block_node().is_some()
        {
            self.skipping += 1;
            self.stack.push(node);
            return;
        }
        let is_root = self.stack.is_empty();
        if !is_root {
            self.inspect(&node);
        }
        self.stack.push(node);
    }

    fn leave(&mut self) {
        self.stack.pop();
        if self.skipping > 0 {
            self.skipping -= 1;
        }
    }
}

impl<'pr> Scan<'_, 'pr> {
    fn multiline(&self, range: &Range<usize>) -> bool {
        self.file.line_of(range.start)
            != self
                .file
                .line_of(range.end.saturating_sub(1).max(range.start))
    }

    fn inspect(&mut self, node: &Node<'pr>) {
        let parent = self.stack.last();
        let unsafe_node = match node {
            Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::CaseNode { .. }
            | Node::DefNode { .. }
            | Node::RescueModifierNode { .. } => true,
            Node::BeginNode { .. } => {
                let begin = node.as_begin_node().unwrap();
                begin.begin_keyword_loc().is_some()
                    || begin.rescue_clause().is_some()
                    || begin.ensure_clause().is_some()
            }
            Node::StringNode { .. } => {
                let string = node.as_string_node().unwrap();
                let heredoc = string
                    .opening_loc()
                    .is_some_and(|o| o.as_slice().starts_with(b"<<"));
                let in_regexp = parent.is_some_and(|p| {
                    p.as_interpolated_regular_expression_node().is_some()
                        || p.as_interpolated_match_last_line_node().is_some()
                });
                let value: &[u8] = if in_regexp {
                    string.content_loc().as_slice()
                } else {
                    string.unescaped()
                };
                heredoc || value.contains(&b'\n')
            }
            Node::InterpolatedStringNode { .. } => {
                let string = node.as_interpolated_string_node().unwrap();
                string
                    .opening_loc()
                    .is_some_and(|o| o.as_slice().starts_with(b"<<"))
                    || string.parts().iter().any(|part| {
                        part.as_string_node().is_none()
                            && part.location().as_slice().contains(&b'\n')
                    })
            }
            Node::XStringNode { .. } => node
                .as_x_string_node()
                .unwrap()
                .unescaped()
                .contains(&b'\n'),
            Node::RegularExpressionNode { .. } => node
                .as_regular_expression_node()
                .unwrap()
                .content_loc()
                .as_slice()
                .contains(&b'\n'),
            Node::MatchLastLineNode { .. } => node
                .as_match_last_line_node()
                .unwrap()
                .content_loc()
                .as_slice()
                .contains(&b'\n'),
            // Multi-line parser `begin`s and symbols.
            Node::ParenthesesNode { .. }
            | Node::EmbeddedStatementsNode { .. }
            | Node::SymbolNode { .. } => self.multiline(&location_range(node)),
            Node::StatementsNode { .. } => {
                node.as_statements_node().unwrap().body().iter().count() > 1
                    && !parent.is_some_and(|p| {
                        p.as_parentheses_node().is_some()
                            || p.as_begin_node().is_some()
                            || p.as_program_node().is_some()
                    })
                    && self.multiline(&location_range(node))
            }
            _ => false,
        };
        if unsafe_node {
            self.facts.safe_to_split = false;
        }
        // Descendant parser blocks.
        let block = (node.as_lambda_node().is_some()
            || literal_block(node).is_some()
                && self.root_block.as_ref() != Some(&location_range(node)))
        .then(|| location_range(node));
        if let Some(block) = block {
            if self.multiline(&block) {
                self.facts.multiline_block = true;
            } else if self.dotted_call_parent(node) {
                self.facts.chained_single_line_block = true;
            }
        }
    }

    /// Whether the parser parent of the block owned by `node` is a dotted
    /// `send`/`csend` (`parent.call_type? && parent.loc.dot`).
    fn dotted_call_parent(&self, node: &Node<'pr>) -> bool {
        let Some(parent) = self.stack.last() else {
            return false;
        };
        let call = if parent.as_arguments_node().is_some() {
            self.stack.iter().rev().nth(1)
        } else {
            Some(parent)
        };
        call.and_then(|c| c.as_call_node()).is_some_and(|call| {
            call.call_operator_loc().is_some()
                && (parent.as_arguments_node().is_some()
                    || call
                        .receiver()
                        .is_some_and(|r| location_range(&r) == location_range(node)))
        })
    }
}

/// RuboCop's `to_single_line`.
pub(crate) fn to_single_line(source: &[u8]) -> Vec<u8> {
    let step = quote_join(source, b'"', b'\'', b"\" + '");
    let step = quote_join(&step, b'\'', b'"', b"' + \"");
    let step = same_quote_join(&step);
    let step = chain_join(&step);
    line_join(&step)
}

/// Ruby's `\s`.
fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// `/#{open} *\\\n\s*#{close}/` replaced by `with`.
fn quote_join(source: &[u8], open: u8, close: u8, with: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(source.len());
    let mut i = 0;
    while i < source.len() {
        if source[i] == open {
            let mut j = i + 1;
            while source.get(j) == Some(&b' ') {
                j += 1;
            }
            if source.get(j) == Some(&b'\\') && source.get(j + 1) == Some(&b'\n') {
                let mut k = j + 2;
                while source.get(k).is_some_and(|&b| space(b)) {
                    k += 1;
                }
                if source.get(k) == Some(&close) {
                    out.extend_from_slice(with);
                    i = k + 1;
                    continue;
                }
            }
        }
        out.push(source[i]);
        i += 1;
    }
    out
}

/// `/(["']) *\\\n\s*\1/` removed.
fn same_quote_join(source: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(source.len());
    let mut i = 0;
    while i < source.len() {
        let quote = source[i];
        if quote == b'"' || quote == b'\'' {
            let mut j = i + 1;
            while source.get(j) == Some(&b' ') {
                j += 1;
            }
            if source.get(j) == Some(&b'\\') && source.get(j + 1) == Some(&b'\n') {
                let mut k = j + 2;
                while source.get(k).is_some_and(|&b| space(b)) {
                    k += 1;
                }
                if source.get(k) == Some(&quote) {
                    i = k + 1;
                    continue;
                }
            }
        }
        out.push(source[i]);
        i += 1;
    }
    out
}

/// `/\n\s*(?=(&)?\.\w)/` removed.
fn chain_join(source: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(source.len());
    let mut i = 0;
    while i < source.len() {
        if source[i] == b'\n' {
            let mut k = i + 1;
            while source.get(k).is_some_and(|&b| space(b)) {
                k += 1;
            }
            let dot = if source.get(k) == Some(&b'&') {
                k + 1
            } else {
                k
            };
            if source.get(dot) == Some(&b'.')
                && source
                    .get(dot + 1)
                    .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
            {
                i = k;
                continue;
            }
        }
        out.push(source[i]);
        i += 1;
    }
    out
}

/// `/\s*\\?\n\s*/` replaced by `' '`.
fn line_join(source: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(source.len());
    let mut i = 0;
    while i < source.len() {
        // The greedy `\s*` run, backtracked to the last line break (or a
        // backslash-newline right after the run).
        let mut k = i;
        while source.get(k).is_some_and(|&b| space(b)) {
            k += 1;
        }
        let break_end = if source.get(k) == Some(&b'\\') && source.get(k + 1) == Some(&b'\n') {
            Some(k + 2)
        } else {
            (i..k).rev().find(|&m| source[m] == b'\n').map(|m| m + 1)
        };
        if let Some(mut end) = break_end {
            while source.get(end).is_some_and(|&b| space(b)) {
                end += 1;
            }
            out.push(b' ');
            i = end;
            continue;
        }
        out.push(source[i]);
        i += 1;
    }
    out
}

/// `\r\n` line breaks as `\n`.
fn crlf_to_lf(source: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(source.len());
    for (i, &byte) in source.iter().enumerate() {
        if !(byte == b'\r' && source.get(i + 1) == Some(&b'\n')) {
            out.push(byte);
        }
    }
    out
}

/// Ruby's `String#strip`: leading and trailing whitespace and NULs.
fn strip(bytes: &[u8]) -> &[u8] {
    let white = |b: &u8| space(*b) || *b == 0;
    let start = bytes.iter().position(|b| !white(b)).unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !white(b))
        .map_or(start, |e| e + 1);
    &bytes[start..end]
}

/// `preprocess_reparsed_source`: `\b__LINE__\b` becomes `__LINE0__`.
fn neutralize_line(source: &[u8]) -> Cow<'_, [u8]> {
    const LINE: &[u8] = b"__LINE__";
    if memchr::memmem::find(source, LINE).is_none() {
        return Cow::Borrowed(source);
    }
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = Vec::with_capacity(source.len() + 8);
    let mut i = 0;
    while i < source.len() {
        if source[i..].starts_with(LINE)
            && (i == 0 || !word(source[i - 1]))
            && source.get(i + LINE.len()).is_none_or(|&b| !word(b))
        {
            out.extend_from_slice(b"__LINE0__");
            i += LINE.len();
            continue;
        }
        out.push(source[i]);
        i += 1;
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests;
