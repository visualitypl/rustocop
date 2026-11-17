//! `Style/RedundantParentheses` — flags grouping parentheses that do not
//! change what the code means: `(x)`, `foo((1))`, `x = (a && b)`.
//!
//! A port of RuboCop's cop, which works in two steps:
//!
//! 1. Its rules pick candidates by what the parentheses hold and where they
//!    sit (a literal, variable, constant, keyword, method call, assignment,
//!    logical or comparison expression, block body, …), and leave alone the
//!    places where parentheses read as intended or are needed: method
//!    arguments written `foo (x)`, conditions of `begin…end while`, `rescue`
//!    lists, multi-statement groups used as values, `->`/`lambda`/`proc`
//!    with `do…end`, mixed `&&`/`||`, splats, ranges, a heredoc that would
//!    strand the code after `)`, … rustocop ports these rules on Prism's
//!    tree, mapping it back to the shape of RuboCop's (parser gem) AST.
//! 2. Every candidate is verified: its parentheses are removed (as RuboCop's
//!    autocorrection would) and the file is parsed again. Only a candidate
//!    whose removal still parses to the same tree, grouping aside, is
//!    reported. As in RuboCop, candidates are grouped by their innermost
//!    `def`, `class`, `module` or `class << x` and only that fragment is
//!    parsed again (the whole file outside any); each group is verified in
//!    one batch, split in halves while a batch fails (the shared
//!    `cops::reparse`, also used by `Layout/RedundantLineBreak`). `^(var)`
//!    pins need no verification.
//!
//! The defaults that RuboCop takes from other cops are fixed to theirs:
//! `Style/TernaryParentheses` (`require_no_parentheses`, so a parenthesized
//! ternary condition is flagged) and `Style/ParenthesesAroundCondition`
//! (`AllowInMultilineConditions: false`).
//!
//! Deviation: RuboCop skips verification for code over 64 KiB outside any
//! method or class (reporting its candidates unverified); rustocop always
//! verifies. Offenses are reported at the opening parenthesis.

use std::{collections::BTreeMap, ops::Range};

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{self, call_arguments, fingerprint_ignoring_grouping},
        reparse,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Style/RedundantParentheses` cop.
pub struct RedundantParentheses;

/// RuboCop's message for parentheses directly around a block's body.
const BLOCK_BODY: &str = "block body";

impl Cop for RedundantParentheses {
    fn name(&self) -> &'static str {
        "Style/RedundantParentheses"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // The rules look at a node's parent, grandparent and siblings, so one
        // walk from the root keeps the ancestors at hand.
        if node.as_program_node().is_none() || !file.contains(b"(") {
            return;
        }
        let mut finder = Finder {
            file,
            stack: Vec::new(),
            candidates: BTreeMap::new(),
            pins: Vec::new(),
        };
        walk(&mut finder, node);
        let candidates: Vec<Candidate> = finder.candidates.into_values().collect();
        let mut reported: Vec<_> = verified(file, node, &candidates)
            .into_iter()
            .map(|index| {
                (
                    candidates[index].open,
                    candidates[index].end,
                    candidates[index].message,
                    atomic_correction(file, &candidates[index].edits),
                )
            })
            .chain(finder.pins.into_iter().map(|(open, end, message)| {
                let mut left = open + 1;
                while left < end - 1 && is_space(file.bytes[left]) {
                    left += 1;
                }
                let mut right = end - 1;
                while right > left && is_space(file.bytes[right - 1]) {
                    right -= 1;
                }
                let edits = [
                    Edit {
                        range: open..left,
                        text: b"",
                    },
                    Edit {
                        range: right..end,
                        text: b"",
                    },
                ];
                (open, end, message, atomic_correction(file, &edits))
            }))
            .collect();
        reported.sort_unstable_by_key(|(offset, _, _, _)| *offset);
        reported.dedup_by_key(|(offset, _, _, _)| *offset);
        for (offset, end, message, correction) in reported {
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: format!("Don't use parentheses around {message}."),
                line,
                end_line: file.line_of(end.saturating_sub(1).max(offset)),
                col,
            });
        }
    }
}

/// Parentheses that the rules flag, pending verification.
struct Candidate {
    /// The offset of `(`.
    open: usize,
    /// End-exclusive end of the highlighted parentheses.
    end: usize,
    /// RuboCop's description of what the parentheses hold.
    message: &'static str,
    /// The edits that remove the parentheses.
    edits: Vec<Edit>,
}

/// A replacement of a byte range of the source.
#[derive(Debug, Clone)]
struct Edit {
    range: Range<usize>,
    text: &'static [u8],
}

/// Walks the tree, keeping the ancestors, and collects candidates.
struct Finder<'a, 'pr> {
    file: &'a SourceFile,
    stack: Vec<Node<'pr>>,
    /// Candidates by the offset of their `(`; a later rule's message wins,
    /// as RuboCop keys its pending offenses by node.
    candidates: BTreeMap<usize, Candidate>,
    /// `^(var)` pins: (start, end, message), reported without a reparse.
    pins: Vec<(usize, usize, &'static str)>,
}

impl<'pr> NodeVisitor<'pr> for Finder<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        match &node {
            Node::ParenthesesNode { .. } => self.check_parentheses(&node),
            Node::PinnedExpressionNode { .. } => {
                let pin = node.as_pinned_expression_node().unwrap();
                // `^(var)` reads as `^var`; any other expression needs them.
                if ast::is_variable(&pin.expression()) {
                    self.pins.push((
                        pin.lparen_loc().start_offset(),
                        pin.rparen_loc().end_offset(),
                        "a variable",
                    ));
                }
            }
            _ => {}
        }
        self.stack.push(node);
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

impl<'pr> Finder<'_, 'pr> {
    fn check_parentheses(&mut self, paren: &Node<'pr>) {
        let parentheses = paren.as_parentheses_node().unwrap();
        // Empty parentheses, `()`, are allowed.
        let Some(body) = parentheses.body() else {
            return;
        };
        let children: Vec<Node<'pr>> = match body.as_statements_node() {
            Some(statements) => statements.body().iter().collect(),
            None => vec![body],
        };
        if children.is_empty() {
            return;
        }
        let chain = parser_ancestors(&self.stack, paren);
        let context = Context {
            file: self.file,
            stack: &self.stack,
            paren,
            chain: &chain,
            children: &children,
        };
        if context.parens_allowed() || context.ignore_syntax() {
            return;
        }
        let Some((target, message)) = context.check() else {
            return;
        };
        let candidate = match target {
            Target::Itself => Candidate {
                open: parentheses.opening_loc().start_offset(),
                end: paren.location().end_offset(),
                message,
                edits: correction(self.file, paren, &children, &chain),
            },
            Target::Parent(index) => {
                // The parentheses around a range's parentheses.
                let outer = &self.stack[index];
                let outer_parentheses = outer.as_parentheses_node().unwrap();
                let outer_children: Vec<Node<'pr>> = match outer_parentheses
                    .body()
                    .and_then(|body| body.as_statements_node())
                {
                    Some(statements) => statements.body().iter().collect(),
                    None => outer_parentheses.body().into_iter().collect(),
                };
                let outer_chain = parser_ancestors(&self.stack[..index], outer);
                Candidate {
                    open: outer_parentheses.opening_loc().start_offset(),
                    end: outer.location().end_offset(),
                    message,
                    edits: correction(self.file, outer, &outer_children, &outer_chain),
                }
            }
        };
        self.candidates.insert(candidate.open, candidate);
    }
}

/// What a flagged pair of parentheses is: these, or the grouping
/// parentheses around them (at this index of the walk's stack).
enum Target {
    Itself,
    Parent(usize),
}

/// A node of RuboCop's AST that is a parenthesized node's ancestor, mapped
/// from Prism's tree (which has extra wrapper nodes, and lacks some).
#[derive(Clone, Copy)]
enum Parent<'a, 'pr> {
    /// A parenthesized `begin`, at this index of the stack.
    Paren {
        stack_index: usize,
        count: usize,
        index: usize,
    },
    /// A `begin` holding a statement sequence.
    Sequence { count: usize, index: usize },
    /// The `begin` of an interpolation (`#{…}`).
    Interpolation { count: usize, index: usize },
    /// `begin…end` without `rescue` or `ensure` (`kwbegin`).
    Kwbegin { count: usize },
    /// A `rescue` statement (body, `rescue` clauses, `else`).
    Rescue { count: usize },
    /// A `rescue` clause (`resbody`).
    Resbody,
    /// An `ensure` statement.
    Ensure,
    /// The exception list of a `rescue` clause (an `array`).
    ExceptionList { count: usize },
    /// A brace-less hash of keyword arguments.
    KeywordHash { count: usize },
    /// `/(?<name>…)/ =~ value` (`match_with_lvasgn`).
    MatchWithLvasgn,
    /// Any other node, the same in both trees.
    Node(&'a Node<'pr>),
}

/// The ancestors that RuboCop's AST gives `node`, innermost first, from
/// Prism's `stack` (root first, `node`'s parent last).
fn parser_ancestors<'a, 'pr>(stack: &'a [Node<'pr>], node: &Node<'pr>) -> Vec<Parent<'a, 'pr>> {
    let mut chain = Vec::new();
    // The (count, index) of a statement list, for the node that owns it.
    let mut statements: Option<(usize, usize)> = None;
    let mut child = span(node);
    for index in (0..stack.len()).rev() {
        let ancestor = &stack[index];
        let grandparent = index.checked_sub(1).map(|up| &stack[up]);
        match ancestor {
            Node::StatementsNode { .. } => {
                let body = ancestor.as_statements_node().unwrap().body();
                let count = body.iter().count();
                let position = body
                    .iter()
                    .position(|statement| span(&statement) == child)
                    .unwrap_or(0);
                let owned = matches!(
                    grandparent,
                    Some(Node::ParenthesesNode { .. } | Node::EmbeddedStatementsNode { .. })
                ) || grandparent.is_some_and(is_plain_kwbegin);
                if owned {
                    statements = Some((count, position));
                } else if count >= 2 {
                    chain.push(Parent::Sequence {
                        count,
                        index: position,
                    });
                }
            }
            Node::ParenthesesNode { .. } => {
                let (count, index_in) = statements.take().unwrap_or((1, 0));
                chain.push(Parent::Paren {
                    stack_index: index,
                    count,
                    index: index_in,
                });
            }
            Node::EmbeddedStatementsNode { .. } => {
                let (count, index_in) = statements.take().unwrap_or((1, 0));
                chain.push(Parent::Interpolation {
                    count,
                    index: index_in,
                });
            }
            Node::ArgumentsNode { .. }
            | Node::ElseNode { .. }
            | Node::EnsureNode { .. }
            | Node::ProgramNode { .. }
            | Node::MatchWriteNode { .. } => {}
            Node::RescueNode { .. } => {
                let rescue = ancestor.as_rescue_node().unwrap();
                let is_subsequent = rescue
                    .subsequent()
                    .is_some_and(|next| span(&next.as_node()) == child);
                if !is_subsequent {
                    let exceptions = rescue.exceptions();
                    if exceptions.iter().any(|exception| span(&exception) == child) {
                        chain.push(Parent::ExceptionList {
                            count: exceptions.iter().count(),
                        });
                    }
                    chain.push(Parent::Resbody);
                }
            }
            Node::BeginNode { .. } => {
                let begin = ancestor.as_begin_node().unwrap();
                let explicit = begin.begin_keyword_loc().is_some();
                let from_ensure = begin
                    .ensure_clause()
                    .is_some_and(|ensure| span(&ensure.as_node()) == child);
                if from_ensure {
                    chain.push(Parent::Ensure);
                    if explicit {
                        chain.push(Parent::Kwbegin { count: 1 });
                    }
                } else if begin.rescue_clause().is_none() && begin.ensure_clause().is_none() {
                    if explicit {
                        let count = statements.take().map_or(0, |(count, _)| count);
                        chain.push(Parent::Kwbegin { count });
                    }
                } else {
                    if let Some(rescue) = begin.rescue_clause() {
                        let mut clauses = 1;
                        let mut next = rescue.subsequent();
                        while let Some(clause) = next {
                            clauses += 1;
                            next = clause.subsequent();
                        }
                        // Body, clauses, `else` (all slots, even empty).
                        chain.push(Parent::Rescue { count: clauses + 2 });
                    }
                    if begin.ensure_clause().is_some() {
                        chain.push(Parent::Ensure);
                    }
                    if explicit {
                        chain.push(Parent::Kwbegin { count: 1 });
                    }
                }
            }
            Node::KeywordHashNode { .. } => {
                let count = ancestor
                    .as_keyword_hash_node()
                    .unwrap()
                    .elements()
                    .iter()
                    .count();
                chain.push(Parent::KeywordHash { count });
            }
            Node::CallNode { .. } if matches!(grandparent, Some(Node::MatchWriteNode { .. })) => {
                chain.push(Parent::MatchWithLvasgn);
            }
            _ => chain.push(Parent::Node(ancestor)),
        }
        child = span(ancestor);
    }
    chain
}

/// `begin…end` without `rescue` or `ensure`: RuboCop's `kwbegin` holds its
/// statements directly.
fn is_plain_kwbegin(node: &Node<'_>) -> bool {
    node.as_begin_node().is_some_and(|begin| {
        begin.begin_keyword_loc().is_some()
            && begin.rescue_clause().is_none()
            && begin.ensure_clause().is_none()
    })
}

/// A node's byte range, as an identity within one tree.
fn span(node: &Node<'_>) -> (usize, usize) {
    let location = node.location();
    (location.start_offset(), location.end_offset())
}

/// A parenthesized node and its surroundings.
struct Context<'c, 'pr> {
    file: &'c SourceFile,
    stack: &'c [Node<'pr>],
    paren: &'c Node<'pr>,
    chain: &'c [Parent<'c, 'pr>],
    /// The statements inside the parentheses.
    children: &'c [Node<'pr>],
}

impl<'pr> Context<'_, 'pr> {
    fn parent(&self) -> Option<Parent<'_, 'pr>> {
        self.chain.first().copied()
    }

    fn parent_node(&self) -> Option<&Node<'pr>> {
        match self.chain.first() {
            Some(Parent::Node(node)) => Some(node),
            _ => None,
        }
    }

    fn open(&self) -> usize {
        self.paren
            .as_parentheses_node()
            .unwrap()
            .opening_loc()
            .start_offset()
    }

    fn close(&self) -> usize {
        self.paren
            .as_parentheses_node()
            .unwrap()
            .closing_loc()
            .start_offset()
    }

    fn is(&self, node: Option<Node<'_>>) -> bool {
        node.is_some_and(|node| span(&node) == span(self.paren))
    }

    /// RuboCop's `begin_type?` for the parent.
    fn parent_is_begin(&self) -> bool {
        matches!(
            self.parent(),
            Some(Parent::Paren { .. } | Parent::Sequence { .. } | Parent::Interpolation { .. })
        )
    }

    /// RuboCop's `chained?`: the parentheses are a call's receiver.
    fn chained(&self) -> bool {
        self.parent_node()
            .and_then(|parent| parent.as_call_node())
            .is_some_and(|call| self.is(call.receiver()))
    }

    /// Whether the parent is a ternary `if`.
    fn parent_is_ternary(&self) -> bool {
        self.parent_node()
            .and_then(|parent| parent.as_if_node())
            .is_some_and(|if_node| if_node.if_keyword_loc().is_none())
    }

    /// RuboCop's `parens_allowed?`.
    fn parens_allowed(&self) -> bool {
        let rescue = matches!(self.chain.first(), Some(Parent::Resbody))
            || matches!(self.chain.get(1), Some(Parent::Resbody));
        let pattern_in_argument = self
            .parent_node()
            .is_some_and(|parent| parent.as_call_node().is_some())
            && matches!(self.children[0], Node::MatchPredicateNode { .. });
        rescue
            || pattern_in_argument
            || self.allowed_expression()
            || self.heredoc_strands_code_after_close_paren()
    }

    fn allowed_expression(&self) -> bool {
        let allowed_ancestor = self
            .parent()
            .is_some_and(|parent| is_keyword_parent(&parent))
            && self.parens_required();
        let allowed_multiple = self.children.len() > 1
            && self.parent().is_some()
            && !self.parent_is_begin()
            && !self.parent_node().is_some_and(|parent| {
                matches!(
                    parent,
                    Node::DefNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
                )
            });
        let in_range = matches!(self.parent_node(), Some(Node::RangeNode { .. }));
        allowed_ancestor || allowed_multiple || in_range
    }

    /// RuboCop's `parens_required?`: a letter touches the parentheses
    /// (`else(1)`, `(1)end`).
    fn parens_required(&self) -> bool {
        let bytes = &self.file.bytes;
        let open = self.open();
        // Ruby's `source[-1]` reads the file's last character.
        let before = if open == 0 {
            bytes.last()
        } else {
            bytes.get(open - 1)
        };
        let after = bytes.get(self.close() + 1);
        before.is_some_and(u8::is_ascii_lowercase) || after.is_some_and(u8::is_ascii_lowercase)
    }

    /// RuboCop's `heredoc_strands_code_after_close_paren?`.
    fn heredoc_strands_code_after_close_paren(&self) -> bool {
        if !heredoc_ends_before(self.children.last(), self.close()) {
            return false;
        }
        let after = text_after(self.file, self.close());
        !after.is_empty()
            && !after.starts_with(b".")
            && !after.starts_with(b"&.")
            && !after.starts_with(b",")
    }

    /// RuboCop's `ignore_syntax?`.
    fn ignore_syntax(&self) -> bool {
        let Some(parent) = self.parent() else {
            return false;
        };
        let node = match parent {
            Parent::MatchWithLvasgn => return true,
            Parent::Node(node) => node,
            _ => return false,
        };
        match node {
            Node::WhileNode { .. } => node.as_while_node().unwrap().is_begin_modifier(),
            Node::UntilNode { .. } => node.as_until_node().unwrap().is_begin_modifier(),
            Node::ReturnNode { .. } | Node::NextNode { .. } | Node::BreakNode { .. } => {
                let location = node.location();
                self.file.line_of(location.start_offset())
                    != self.file.line_of(location.end_offset().saturating_sub(1))
            }
            _ => like_method_argument_parentheses(node),
        }
    }

    /// RuboCop's `check`: the parentheses to flag and the message.
    fn check(&self) -> Option<(Target, &'static str)> {
        let node = &self.children[0];
        if let Some(message) = self.offense_message(node) {
            if message == BLOCK_BODY {
                return Some((Target::Itself, message));
            }
            if matches!(node, Node::RangeNode { .. }) && !self.argument_of_parenthesized_call(node)
            {
                // RuboCop flags the range's parent; only parentheses can be.
                return match self.parent() {
                    Some(Parent::Paren { stack_index, .. }) => {
                        Some((Target::Parent(stack_index), message))
                    }
                    _ => None,
                };
            }
            return Some((Target::Itself, message));
        }
        if is_call_like(node) {
            return self
                .check_send(node)
                .map(|message| (Target::Itself, message));
        }
        None
    }

    /// RuboCop's `find_offense_message`.
    fn offense_message(&self, node: &Node<'pr>) -> Option<&'static str> {
        if keyword_with_redundant_parentheses(node) {
            return Some("a keyword");
        }
        if ast::is_literal(node) && self.disallowed_literal(node) {
            return Some("a literal");
        }
        if ast::is_variable(node) {
            return Some("a variable");
        }
        if matches!(
            node,
            Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
        ) {
            return Some("a constant");
        }
        let block_parent = self.parent_node().is_some_and(|parent| {
            matches!(parent, Node::BlockNode { .. } | Node::LambdaNode { .. })
        });
        if block_parent || self.body_range(node) {
            return Some(BLOCK_BODY);
        }
        if is_assignment(node) && (self.parent().is_none() || self.parent_is_begin()) {
            return Some("an assignment");
        }
        if is_lambda_or_proc_expression(node) {
            return Some("an expression");
        }
        if self.disallowed_one_line_pattern_matching(node) {
            return Some("a one-line pattern matching");
        }
        let interpolation = matches!(self.chain.first(), Some(Parent::Interpolation { .. }))
            && matches!(
                self.chain.get(1),
                Some(Parent::Node(Node::InterpolatedStringNode { .. }))
            );
        if interpolation {
            return Some("an interpolated expression");
        }
        if self.argument_of_parenthesized_call(node) && !is_keyword_logical_operator(node) {
            return Some("a method argument");
        }
        if self.oneline_rescue_parentheses_required(node) {
            return Some("a one-line rescue");
        }
        if self.chained() {
            return None;
        }
        if matches!(node, Node::AndNode { .. } | Node::OrNode { .. }) {
            if is_keyword_logical_operator(node) && self.parent().is_some() {
                return None;
            }
            let allowed_parent = self.parent_node().is_some_and(|parent| match parent {
                Node::OrNode { .. } | Node::SplatNode { .. } | Node::AssocSplatNode { .. } => true,
                Node::CallNode { .. } => !parent.as_call_node().unwrap().is_safe_navigation(),
                _ => false,
            });
            if allowed_parent {
                return None;
            }
            let and_parent = matches!(self.parent_node(), Some(Node::AndNode { .. }));
            if matches!(node, Node::OrNode { .. }) && and_parent {
                return None;
            }
            if self.parent_is_ternary() {
                return None;
            }
            return Some("a logical expression");
        }
        if let Some(call) = node.as_call_node() {
            const COMPARISONS: [&[u8]; 7] = [b"==", b"===", b"!=", b"<=", b">=", b">", b"<"];
            if COMPARISONS.contains(&call.name().as_slice()) {
                return self.parent().is_none().then_some("a comparison expression");
            }
        }
        None
    }

    /// RuboCop's `disallowed_literal?`: a range only in parentheses of
    /// their own.
    fn disallowed_literal(&self, node: &Node<'pr>) -> bool {
        if !matches!(node, Node::RangeNode { .. }) {
            return true;
        }
        matches!(
            self.parent(),
            Some(
                Parent::Paren { count: 1, .. }
                    | Parent::Sequence { count: 1, .. }
                    | Parent::Interpolation { count: 1, .. }
            )
        )
    }

    /// RuboCop's `body_range?`: a beginless range first, or an endless range
    /// last, in a statement list.
    fn body_range(&self, node: &Node<'pr>) -> bool {
        let Some(range) = node.as_range_node() else {
            return false;
        };
        if self.chained() {
            return false;
        }
        let (count, index) = match self.parent() {
            Some(
                Parent::Paren { count, index, .. }
                | Parent::Sequence { count, index }
                | Parent::Interpolation { count, index },
            ) => (count, index),
            _ => return false,
        };
        (range.left().is_none() && index == 0) || (range.right().is_none() && index + 1 == count)
    }

    /// RuboCop's `disallowed_one_line_pattern_matching?`.
    fn disallowed_one_line_pattern_matching(&self, node: &Node<'pr>) -> bool {
        if let Some(parent) = self.parent_node() {
            let endless_def = parent
                .as_def_node()
                .is_some_and(|def| def.equal_loc().is_some());
            if endless_def || is_assignment(parent) || is_setter_call(parent) {
                return false;
            }
        }
        matches!(
            node,
            Node::MatchPredicateNode { .. } | Node::MatchRequiredNode { .. }
        ) && !self
            .stack
            .iter()
            .any(|ancestor| matches!(ancestor, Node::AndNode { .. } | Node::OrNode { .. }))
    }

    /// RuboCop's `argument_of_parenthesized_method_call?`.
    fn argument_of_parenthesized_call(&self, node: &Node<'pr>) -> bool {
        if is_basic_conditional(node)
            || matches!(node, Node::RescueModifierNode { .. })
            || method_call_parentheses_required(node)
        {
            return false;
        }
        self.parent_node()
            .and_then(|parent| parent.as_call_node())
            .is_some_and(|call| is_parenthesized_call(&call) && !self.is(call.receiver()))
    }

    /// RuboCop's `oneline_rescue_parentheses_required?`.
    fn oneline_rescue_parentheses_required(&self, node: &Node<'pr>) -> bool {
        if !matches!(node, Node::RescueModifierNode { .. }) {
            return false;
        }
        let Some(parent) = self.parent() else {
            return false;
        };
        if self.parent_is_ternary() || self.is_condition_of_parent() {
            return false;
        }
        match parent {
            Parent::ExceptionList { .. } => false,
            Parent::Node(node) => !matches!(
                node,
                Node::CallNode { .. } | Node::ArrayNode { .. } | Node::AssocNode { .. }
            ),
            _ => true,
        }
    }

    /// Whether the parentheses are the condition of the parent `if`,
    /// `unless`, `while`, `until` or `case` (RuboCop's `conditional?` with
    /// `condition == node`).
    fn is_condition_of_parent(&self) -> bool {
        let Some(parent) = self.parent_node() else {
            return false;
        };
        let condition = match parent {
            Node::IfNode { .. } => Some(parent.as_if_node().unwrap().predicate()),
            Node::UnlessNode { .. } => Some(parent.as_unless_node().unwrap().predicate()),
            Node::WhileNode { .. } => Some(parent.as_while_node().unwrap().predicate()),
            Node::UntilNode { .. } => Some(parent.as_until_node().unwrap().predicate()),
            Node::CaseNode { .. } => parent.as_case_node().unwrap().predicate(),
            Node::CaseMatchNode { .. } => parent.as_case_match_node().unwrap().predicate(),
            _ => None,
        };
        self.is(condition)
    }

    /// RuboCop's `check_send`, for a call (or a call with a brace block).
    fn check_send(&self, node: &Node<'pr>) -> Option<&'static str> {
        let call = node.as_call_node().unwrap();
        if is_unary_operation(&call) {
            return self.check_unary(node);
        }
        if self.setter_in_condition(&call) {
            return None;
        }
        self.method_call_with_redundant_parentheses(node, true)
            .then_some("a method call")
    }

    /// RuboCop's `check_unary`: look through unary operators (but `!`).
    fn check_unary(&self, node: &Node<'pr>) -> Option<&'static str> {
        if self.chained() {
            return None;
        }
        let mut current = node.as_call_node().unwrap().as_node();
        let mut as_send = true;
        while let Some(call) = current.as_call_node() {
            let suspect = !call.is_safe_navigation()
                && (as_send || !has_literal_block(&call))
                && is_unary_operation(&call)
                && !is_prefix_not(&call);
            if !suspect {
                break;
            }
            let Some(receiver) = call.receiver() else {
                break;
            };
            current = receiver;
            as_send = false;
        }
        self.method_call_with_redundant_parentheses(&current, as_send)
            .then_some("a unary operation")
    }

    /// RuboCop's `setter_in_condition?`: `if (foo.bar = baz)`.
    fn setter_in_condition(&self, call: &CallNode<'pr>) -> bool {
        call.equal_loc().is_some()
            && self.parent_node().is_some_and(is_basic_conditional)
            && self.is_condition_of_parent()
    }

    /// RuboCop's `method_call_with_redundant_parentheses?`. `as_send`: a
    /// call with a literal block counts as its call alone (RuboCop's
    /// `send_node`), not as a block.
    fn method_call_with_redundant_parentheses(&self, node: &Node<'pr>, as_send: bool) -> bool {
        let (arguments, parenthesized, square_brackets) = match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if (!as_send && has_literal_block(&call)) || is_prefix_not(&call) {
                    return false;
                }
                (
                    call_arguments(&call).len(),
                    is_parenthesized_call(&call),
                    is_square_brackets(&call),
                )
            }
            Node::SuperNode { .. } => {
                let super_node = node.as_super_node().unwrap();
                if super_node
                    .block()
                    .is_some_and(|block| block.as_block_node().is_some())
                {
                    return false;
                }
                let count = super_node
                    .arguments()
                    .map_or(0, |arguments| arguments.arguments().iter().count())
                    + usize::from(super_node.block().is_some());
                (count, super_node.rparen_loc().is_some(), false)
            }
            Node::YieldNode { .. } => {
                let yield_node = node.as_yield_node().unwrap();
                let count = yield_node
                    .arguments()
                    .map_or(0, |arguments| arguments.arguments().iter().count());
                (count, yield_node.rparen_loc().is_some(), false)
            }
            Node::DefinedNode { .. } => (
                1,
                node.as_defined_node().unwrap().rparen_loc().is_some(),
                false,
            ),
            _ => return false,
        };
        if self.singular_parenthesized_parent() {
            return true;
        }
        arguments == 0 || parenthesized || square_brackets
    }

    /// RuboCop's `singular_parenthesized_parent?`.
    fn singular_parenthesized_parent(&self) -> bool {
        match self.parent() {
            None => true,
            Some(Parent::Node(Node::SplatNode { .. } | Node::AssocSplatNode { .. })) => false,
            Some(parent) => parser_children(&parent) == 1,
        }
    }
}

/// How many non-`nil` children RuboCop's AST gives a parent node: RuboCop
/// asks `children.one?`, which skips `nil`s, so an `if` with empty
/// branches has one. Unknown shapes count as two: never "singular".
fn parser_children(parent: &Parent<'_, '_>) -> usize {
    let node = match parent {
        Parent::Paren { count, .. }
        | Parent::Sequence { count, .. }
        | Parent::Interpolation { count, .. }
        | Parent::Kwbegin { count }
        | Parent::Rescue { count }
        | Parent::ExceptionList { count }
        | Parent::KeywordHash { count } => return *count,
        // A clause's exception list, variable and body; an `ensure`'s body
        // and clause. Never singular in practice.
        Parent::Resbody | Parent::Ensure | Parent::MatchWithLvasgn => return 2,
        Parent::Node(node) => node,
    };
    let count_arguments = |arguments: Option<ruby_prism::ArgumentsNode<'_>>| {
        arguments.map_or(0, |arguments| arguments.arguments().iter().count())
    };
    let some = |present: bool| usize::from(present);
    match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            some(call.receiver().is_some()) + 1 + call_arguments(&call).len()
        }
        Node::SuperNode { .. } => {
            let super_node = node.as_super_node().unwrap();
            let block_argument = super_node
                .block()
                .is_some_and(|block| block.as_block_argument_node().is_some());
            count_arguments(super_node.arguments()) + some(block_argument)
        }
        Node::YieldNode { .. } => count_arguments(node.as_yield_node().unwrap().arguments()),
        Node::ReturnNode { .. } => count_arguments(node.as_return_node().unwrap().arguments()),
        Node::BreakNode { .. } => count_arguments(node.as_break_node().unwrap().arguments()),
        Node::NextNode { .. } => count_arguments(node.as_next_node().unwrap().arguments()),
        Node::ArrayNode { .. } => node.as_array_node().unwrap().elements().iter().count(),
        Node::HashNode { .. } => node.as_hash_node().unwrap().elements().iter().count(),
        Node::SplatNode { .. }
        | Node::AssocSplatNode { .. }
        | Node::BlockArgumentNode { .. }
        | Node::DefinedNode { .. } => 1,
        Node::IfNode { .. } => {
            let if_node = node.as_if_node().unwrap();
            1 + some(if_node.statements().is_some()) + some(if_node.subsequent().is_some())
        }
        Node::UnlessNode { .. } => {
            let unless = node.as_unless_node().unwrap();
            1 + some(unless.statements().is_some()) + some(unless.else_clause().is_some())
        }
        Node::WhileNode { .. } => 1 + some(node.as_while_node().unwrap().statements().is_some()),
        Node::UntilNode { .. } => 1 + some(node.as_until_node().unwrap().statements().is_some()),
        Node::ForNode { .. } => 2 + some(node.as_for_node().unwrap().statements().is_some()),
        Node::DefNode { .. } => {
            let def = node.as_def_node().unwrap();
            2 + some(def.receiver().is_some()) + some(def.body().is_some())
        }
        Node::BlockNode { .. } => 2 + some(node.as_block_node().unwrap().body().is_some()),
        Node::LambdaNode { .. } => 2 + some(node.as_lambda_node().unwrap().body().is_some()),
        Node::ClassNode { .. } => {
            let class = node.as_class_node().unwrap();
            1 + some(class.superclass().is_some()) + some(class.body().is_some())
        }
        Node::ModuleNode { .. } => 1 + some(node.as_module_node().unwrap().body().is_some()),
        Node::SingletonClassNode { .. } => {
            1 + some(node.as_singleton_class_node().unwrap().body().is_some())
        }
        Node::CaseNode { .. } => {
            let case = node.as_case_node().unwrap();
            some(case.predicate().is_some())
                + case.conditions().iter().count()
                + some(case.else_clause().is_some())
        }
        Node::CaseMatchNode { .. } => {
            let case = node.as_case_match_node().unwrap();
            some(case.predicate().is_some())
                + case.conditions().iter().count()
                + some(case.else_clause().is_some())
        }
        Node::WhenNode { .. } => {
            let when = node.as_when_node().unwrap();
            when.conditions().iter().count() + some(when.statements().is_some())
        }
        Node::InNode { .. } => 1 + some(node.as_in_node().unwrap().statements().is_some()),
        Node::RangeNode { .. } => {
            let range = node.as_range_node().unwrap();
            some(range.left().is_some()) + some(range.right().is_some())
        }
        Node::ConstantPathWriteNode { .. }
        | Node::LocalVariableOperatorWriteNode { .. }
        | Node::InstanceVariableOperatorWriteNode { .. }
        | Node::ClassVariableOperatorWriteNode { .. }
        | Node::GlobalVariableOperatorWriteNode { .. }
        | Node::ConstantOperatorWriteNode { .. }
        | Node::ConstantPathOperatorWriteNode { .. }
        | Node::CallOperatorWriteNode { .. }
        | Node::IndexOperatorWriteNode { .. } => 3,
        _ => 2,
    }
}

/// Whether RuboCop's `keyword?` holds for a parent node.
fn is_keyword_parent(parent: &Parent<'_, '_>) -> bool {
    let node = match parent {
        Parent::Kwbegin { .. } | Parent::Rescue { .. } | Parent::Ensure => return true,
        Parent::Node(node) => node,
        _ => return false,
    };
    match node {
        Node::IfNode { .. }
        | Node::UnlessNode { .. }
        | Node::WhileNode { .. }
        | Node::UntilNode { .. }
        | Node::CaseNode { .. }
        | Node::WhenNode { .. }
        | Node::ForNode { .. }
        | Node::DefNode { .. }
        | Node::ClassNode { .. }
        | Node::ModuleNode { .. }
        | Node::ReturnNode { .. }
        | Node::BreakNode { .. }
        | Node::NextNode { .. }
        | Node::SuperNode { .. }
        | Node::ForwardingSuperNode { .. }
        | Node::YieldNode { .. }
        | Node::DefinedNode { .. }
        | Node::AliasMethodNode { .. }
        | Node::AliasGlobalVariableNode { .. }
        | Node::UndefNode { .. }
        | Node::PostExecutionNode { .. } => true,
        Node::AndNode { .. } | Node::OrNode { .. } => is_keyword_logical_operator(node),
        Node::CallNode { .. } => is_prefix_not(&node.as_call_node().unwrap()),
        _ => false,
    }
}

/// RuboCop's `like_method_argument_parentheses?`: `foo (x)`, `super (x)`,
/// `yield (x)`, a lone argument in parentheses after a space.
fn like_method_argument_parentheses(parent: &Node<'_>) -> bool {
    let (arguments, parenthesized): (Vec<Node<'_>>, bool) = match parent {
        Node::CallNode { .. } => {
            let call = parent.as_call_node().unwrap();
            if call.is_safe_navigation() || ast::is_operator_method(call.name().as_slice()) {
                return false;
            }
            (call_arguments(&call), is_parenthesized_call(&call))
        }
        Node::SuperNode { .. } => {
            let super_node = parent.as_super_node().unwrap();
            let mut arguments: Vec<Node<'_>> = super_node
                .arguments()
                .map(|arguments| arguments.arguments().iter().collect())
                .unwrap_or_default();
            arguments.extend(
                super_node
                    .block()
                    .filter(|block| block.as_block_argument_node().is_some()),
            );
            (arguments, super_node.rparen_loc().is_some())
        }
        Node::YieldNode { .. } => {
            let yield_node = parent.as_yield_node().unwrap();
            let arguments = yield_node
                .arguments()
                .map(|arguments| arguments.arguments().iter().collect())
                .unwrap_or_default();
            (arguments, yield_node.rparen_loc().is_some())
        }
        _ => return false,
    };
    arguments.len() == 1 && !parenthesized && matches!(arguments[0], Node::ParenthesesNode { .. })
}

/// The arguments of an optional argument list.
fn listed<'pr>(arguments: Option<ruby_prism::ArgumentsNode<'pr>>) -> Vec<Node<'pr>> {
    arguments
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default()
}

/// RuboCop's `keyword_with_redundant_parentheses?`.
fn keyword_with_redundant_parentheses(node: &Node<'_>) -> bool {
    // A keyword whose arguments are absent, or parenthesized: `(return)`,
    // `(return(1))`, `(yield())`, `(defined?(x))`.
    let lone_parentheses = |arguments: &[Node<'_>]| {
        arguments.is_empty()
            || (arguments.len() == 1 && matches!(arguments[0], Node::ParenthesesNode { .. }))
    };
    match node {
        Node::SourceFileNode { .. }
        | Node::SourceLineNode { .. }
        | Node::SourceEncodingNode { .. }
        | Node::SelfNode { .. }
        | Node::RedoNode { .. }
        | Node::RetryNode { .. } => true,
        // With a literal block, RuboCop's AST has a block, not a keyword.
        Node::ForwardingSuperNode { .. } => {
            node.as_forwarding_super_node().unwrap().block().is_none()
        }
        Node::ReturnNode { .. } => {
            lone_parentheses(&listed(node.as_return_node().unwrap().arguments()))
        }
        Node::BreakNode { .. } => {
            lone_parentheses(&listed(node.as_break_node().unwrap().arguments()))
        }
        Node::NextNode { .. } => {
            lone_parentheses(&listed(node.as_next_node().unwrap().arguments()))
        }
        Node::YieldNode { .. } => {
            let yield_node = node.as_yield_node().unwrap();
            lone_parentheses(&listed(yield_node.arguments())) || yield_node.rparen_loc().is_some()
        }
        Node::SuperNode { .. } => {
            let super_node = node.as_super_node().unwrap();
            if super_node
                .block()
                .is_some_and(|block| block.as_block_node().is_some())
            {
                return false;
            }
            let mut arguments = listed(super_node.arguments());
            arguments.extend(super_node.block());
            lone_parentheses(&arguments) || super_node.rparen_loc().is_some()
        }
        Node::DefinedNode { .. } => {
            let defined = node.as_defined_node().unwrap();
            matches!(defined.value(), Node::ParenthesesNode { .. })
                || defined.rparen_loc().is_some()
        }
        Node::BeginNode { .. } if is_plain_kwbegin(node) => {
            let statements: Vec<Node<'_>> = node
                .as_begin_node()
                .unwrap()
                .statements()
                .map(|statements| statements.body().iter().collect())
                .unwrap_or_default();
            lone_parentheses(&statements)
        }
        Node::CallNode { .. } => {
            // `not(x)`: a keyword with parentheses of its own.
            let call = node.as_call_node().unwrap();
            is_prefix_not(&call) && is_parenthesized_call(&call)
        }
        _ => false,
    }
}

/// RuboCop's `assignment?`: variable, constant and multiple assignments,
/// and their `op=`, `||=` and `&&=` forms (on calls and indexes too).
pub(super) fn is_assignment(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::MultiWriteNode { .. }
            | Node::LocalVariableOperatorWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::LocalVariableAndWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::InstanceVariableAndWriteNode { .. }
            | Node::ClassVariableOperatorWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. }
            | Node::GlobalVariableOperatorWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantAndWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::CallOperatorWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
    )
}

/// A setter call: `foo.bar = 1`, `foo[0] = 1` (RuboCop's `setter_method?`).
fn is_setter_call(node: &Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|call| call.equal_loc().is_some())
}

/// RuboCop's `basic_conditional?`: `if`, `unless`, `while`, `until`.
fn is_basic_conditional(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::WhileNode { .. }
            | Node::UntilNode { .. }
    )
}

/// `and`/`or` spelled as keywords (RuboCop's `semantic_operator?`).
fn is_keyword_logical_operator(node: &Node<'_>) -> bool {
    let operator = match node {
        Node::AndNode { .. } => node.as_and_node().unwrap().operator_loc(),
        Node::OrNode { .. } => node.as_or_node().unwrap().operator_loc(),
        _ => return false,
    };
    matches!(operator.as_slice(), b"and" | b"or")
}

/// A call with a literal block (RuboCop's AST has a `block` node there).
fn has_literal_block(call: &CallNode<'_>) -> bool {
    call.block()
        .is_some_and(|block| block.as_block_node().is_some())
}

/// A call whose arguments are in parentheses (RuboCop's `parenthesized?`).
fn is_parenthesized_call(call: &CallNode<'_>) -> bool {
    call.closing_loc()
        .is_some_and(|closing| closing.as_slice() == b")")
}

/// RuboCop's `unary_operation?`: an operator method written before its
/// receiver (`-x`, `!x`, `not x`).
fn is_unary_operation(call: &CallNode<'_>) -> bool {
    let Some(selector) = call.message_loc() else {
        return false;
    };
    ast::is_operator_method(call.name().as_slice())
        && selector.start_offset() == call.location().start_offset()
}

/// RuboCop's `prefix_not?`: `not x`.
fn is_prefix_not(call: &CallNode<'_>) -> bool {
    call.name().as_slice() == b"!"
        && call.receiver().is_some()
        && call
            .message_loc()
            .is_some_and(|selector| selector.as_slice() == b"not")
}

/// RuboCop's `call_node?`: a call, or a call with a brace block that is not
/// a lambda or proc.
fn is_call_like(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    match call.block().and_then(|block| block.as_block_node()) {
        None => true,
        Some(block) => block.opening_loc().as_slice() == b"{" && !is_lambda_or_proc(node),
    }
}

/// RuboCop's `lambda_or_proc?`: `->`, `lambda` with a block, `proc` or
/// `Proc.new` with a plain block.
fn is_lambda_or_proc(node: &Node<'_>) -> bool {
    if matches!(node, Node::LambdaNode { .. }) {
        return true;
    }
    let Some(call) = node.as_call_node() else {
        return false;
    };
    let Some(block) = call.block().and_then(|block| block.as_block_node()) else {
        return false;
    };
    let name = call.name().as_slice();
    match call.receiver() {
        None if name == b"lambda" => true,
        None if name == b"proc" => is_plain_block(&block),
        Some(receiver) if name == b"new" => {
            ast::is_global_const(&receiver, &["Proc"]) && is_plain_block(&block)
        }
        _ => false,
    }
}

/// A block without numbered parameters or `it` (RuboCop's `block_type?`).
fn is_plain_block(block: &ruby_prism::BlockNode<'_>) -> bool {
    !matches!(
        block.parameters(),
        Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
    )
}

/// The lambdas and procs that RuboCop flags as "an expression": `->`, and
/// `lambda`, `proc` or `Proc.new` with braces.
fn is_lambda_or_proc_expression(node: &Node<'_>) -> bool {
    if matches!(node, Node::LambdaNode { .. }) {
        return true;
    }
    is_lambda_or_proc(node)
        && node
            .as_call_node()
            .and_then(|call| call.block())
            .and_then(|block| block.as_block_node())
            .is_some_and(|block| block.opening_loc().as_slice() == b"{")
}

/// RuboCop's `method_call_parentheses_required?`: a call with arguments
/// that is receiverless or uses a dot (not an operator).
fn method_call_parentheses_required(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    !has_literal_block(&call)
        && (call.receiver().is_none() || call.call_operator_loc().is_some())
        && !call_arguments(&call).is_empty()
}

/// RuboCop's `square_brackets?`: an index call whose receiver is, or
/// contains, a call without arguments, a string, array, hash, constant or
/// variable.
fn is_square_brackets(call: &CallNode<'_>) -> bool {
    if call.name().as_slice() != b"[]" {
        return false;
    }
    let Some(receiver) = call.receiver() else {
        return false;
    };
    struct Search {
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Search {
        fn enter(&mut self, node: Node<'pr>) {
            self.found |= match &node {
                Node::CallNode { .. } => call_arguments(&node.as_call_node().unwrap()).is_empty(),
                Node::StringNode { .. }
                | Node::ArrayNode { .. }
                | Node::HashNode { .. }
                | Node::KeywordHashNode { .. }
                | Node::ConstantReadNode { .. }
                | Node::ConstantPathNode { .. } => true,
                _ => ast::is_variable(&node),
            };
        }
    }
    let mut search = Search { found: false };
    walk(&mut search, &receiver);
    search.found
}

/// Whether a heredoc inside `node` ends before `offset`.
fn heredoc_ends_before(node: Option<&Node<'_>>, offset: usize) -> bool {
    let Some(node) = node else {
        return false;
    };
    struct Search {
        offset: usize,
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Search {
        fn enter(&mut self, node: Node<'pr>) {
            if let Some(end) = heredoc_end(&node) {
                self.found |= end <= self.offset;
            }
        }
    }
    let mut search = Search {
        offset,
        found: false,
    };
    walk(&mut search, node);
    search.found
}

/// The end offset of a heredoc's terminator line, for a heredoc node.
fn heredoc_end(node: &Node<'_>) -> Option<usize> {
    let (opening, closing) = match node {
        Node::StringNode { .. } => {
            let string = node.as_string_node().unwrap();
            (string.opening_loc()?, string.closing_loc()?)
        }
        Node::InterpolatedStringNode { .. } => {
            let string = node.as_interpolated_string_node().unwrap();
            (string.opening_loc()?, string.closing_loc()?)
        }
        Node::XStringNode { .. } => {
            let string = node.as_x_string_node().unwrap();
            (string.opening_loc(), string.closing_loc())
        }
        Node::InterpolatedXStringNode { .. } => {
            let string = node.as_interpolated_x_string_node().unwrap();
            (string.opening_loc(), string.closing_loc())
        }
        _ => return None,
    };
    opening
        .as_slice()
        .starts_with(b"<<")
        .then(|| closing.end_offset())
}

/// The text after `offset` (a closing parenthesis) on its line, without
/// leading whitespace.
fn text_after(file: &SourceFile, offset: usize) -> &[u8] {
    let line = file.line_of(offset);
    let line_end = file.line_start(line) + file.line_text(line).len();
    let rest = file
        .bytes
        .get(offset + 1..line_end.max(offset + 1))
        .unwrap_or(&[]);
    let start = rest
        .iter()
        .position(|byte| !is_space(*byte))
        .unwrap_or(rest.len());
    &rest[start..]
}

/// Ruby's `\s` (and `String#lstrip`'s whitespace, but NUL).
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x0b')
}

/// RuboCop's `ParenthesesCorrector.correct`: the edits that remove a pair
/// of parentheses, with the whitespace the corrector takes along.
fn correction(
    file: &SourceFile,
    paren: &Node<'_>,
    children: &[Node<'_>],
    chain: &[Parent<'_, '_>],
) -> Vec<Edit> {
    let bytes = &file.bytes;
    let parentheses = paren.as_parentheses_node().unwrap();
    let open = parentheses.opening_loc().start_offset();
    let close = parentheses.closing_loc().start_offset();
    let mut edits = Vec::new();

    // `(` and all whitespace after it.
    let mut end = open + 1;
    while bytes.get(end).is_some_and(|byte| is_space(*byte)) {
        end += 1;
    }
    edits.push(Edit {
        range: open..end,
        text: b"",
    });

    // `)` and the spaces, then line breaks, before it.
    let last_child = children.last();
    let heredoc_before = heredoc_ends_before(last_child, close);
    let after = text_after(file, close);
    let skip_left = |newlines: bool| {
        let mut start = close;
        while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        while newlines && start > 0 && bytes[start - 1] == b'\n' {
            start -= 1;
        }
        start
    };
    if heredoc_before && (after.starts_with(b".") || after.starts_with(b"&.")) {
        edits.push(Edit {
            range: skip_left(true)..close + 1,
            text: b"\n",
        });
    } else {
        // Keep the line break when a comment above `)` would swallow a call
        // chained after it.
        let comment_swallows_chain = last_child.is_some_and(|child| {
            let body_end = child.location().end_offset();
            body_end < close
                && has_comment(&bytes[body_end..close])
                && !after.is_empty()
                && !after.starts_with(b"#")
        });
        edits.push(Edit {
            range: skip_left(!comment_swallows_chain)..close + 1,
            text: b"",
        });
    }

    // A comma left alone on its line goes up with the closing parenthesis.
    let line = file.line_of(close);
    let line_text = file.line_text(line);
    let leading = line_text
        .iter()
        .position(|byte| !is_space(*byte))
        .unwrap_or(line_text.len());
    let close_first_on_line = file.line_start(line) + leading == close;
    let comma_follows = after.starts_with(b",");
    if close_first_on_line && comma_follows {
        // Whitespace and line continuations before `)`.
        let mut start = close;
        loop {
            if start > 0 && is_space(bytes[start - 1]) {
                start -= 1;
            } else if start > 1 && &bytes[start - 2..start] == b"\\\n" {
                start -= 2;
            } else {
                break;
            }
        }
        let last_is_heredoc = last_child.is_some_and(|child| heredoc_end(child).is_some());
        let mut end = close + 1;
        if last_is_heredoc {
            // The comma moves to the heredoc's opening.
            while bytes.get(end).is_some_and(|byte| is_space(*byte)) {
                end += 1;
            }
            end += 1;
            let child_end = last_child.unwrap().location().end_offset();
            edits.push(Edit {
                range: child_end..child_end,
                text: b",",
            });
        }
        edits.push(Edit {
            range: start..end,
            text: b"",
        });
    }

    // `(x)? y : z` needs a space before the `?`.
    let ternary_condition = matches!(
        chain.first(),
        Some(Parent::Node(node)) if node.as_if_node().is_some_and(|if_node| if_node.if_keyword_loc().is_none())
    );
    if ternary_condition && bytes.get(close + 1) == Some(&b'?') {
        edits.push(Edit {
            range: close + 1..close + 1,
            text: b" ",
        });
    }
    edits
}

/// Whether some text holds a comment (`#` to the end of a line).
fn has_comment(text: &[u8]) -> bool {
    text.iter()
        .enumerate()
        .any(|(index, &byte)| byte == b'#' && text[index..].contains(&b'\n'))
}

/// Apply edits to a source; `None` when they clash (RuboCop's corrector
/// raises, and the candidates do not verify).
fn apply(source: &[u8], edits: &[Edit]) -> Option<Vec<u8>> {
    let mut edits: Vec<&Edit> = edits.iter().collect();
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    let mut out = Vec::with_capacity(source.len());
    let mut cursor = 0;
    for edit in edits {
        let Range { start, end } = edit.range.clone();
        if start < cursor {
            // Overlapping deletions merge; anything else clashes.
            if !edit.text.is_empty() || (start == end && start < cursor) {
                return None;
            }
            cursor = cursor.max(end);
            continue;
        }
        out.extend_from_slice(&source[cursor..start]);
        out.extend_from_slice(edit.text);
        cursor = end;
    }
    out.extend_from_slice(&source[cursor..]);
    Some(out)
}

/// Preserve the existing corrector's overlapping-deletion behavior, but emit
/// one atomic replacement including any comma/whitespace outside the highlight.
fn atomic_correction(file: &SourceFile, edits: &[Edit]) -> Option<Correction> {
    let start = edits.iter().map(|edit| edit.range.start).min()?;
    let end = edits.iter().map(|edit| edit.range.end).max()?;
    if super::correction::touches_directive(file, start..end) {
        return None;
    }
    let relative: Vec<_> = edits
        .iter()
        .map(|edit| Edit {
            range: edit.range.start - start..edit.range.end - start,
            text: edit.text,
        })
        .collect();
    let correction = Correction {
        range: start..end,
        replacement: String::from_utf8(apply(&file.bytes[start..end], &relative)?).ok()?,
    };
    Some(correction)
}

/// The indices of the candidates whose removal leaves the parse unchanged,
/// grouping aside (RuboCop's `verified_by_reparse`), through the shared
/// scope-grouped verification (`cops::reparse`).
fn verified(file: &SourceFile, program: &Node<'_>, candidates: &[Candidate]) -> Vec<usize> {
    let spans: Vec<Range<usize>> = candidates
        .iter()
        .map(|candidate| {
            let start = candidate.edits.iter().map(|edit| edit.range.start).min();
            let end = candidate.edits.iter().map(|edit| edit.range.end).max();
            // A candidate without edits is held by no scope.
            start
                .zip(end)
                .map_or(usize::MAX..usize::MAX, |(start, end)| start..end)
        })
        .collect();
    let apply_group = |source: &[u8], start: usize, group: &[usize]| {
        let edits: Vec<Edit> = group
            .iter()
            .flat_map(|&index| candidates[index].edits.iter())
            .map(|edit| Edit {
                range: edit.range.start - start..edit.range.end - start,
                text: edit.text,
            })
            .collect();
        apply(source, &edits)
    };
    reparse::verified(
        file,
        program,
        &spans,
        &apply_group,
        &reparse::Equivalence {
            fingerprint: fingerprint_ignoring_grouping,
            preprocess: reparse::unchanged,
        },
    )
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/redundant_parentheses_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! configuration (`Style/TernaryParentheses` with
    //! `require_no_parentheses`, `Style/ParenthesesAroundCondition` without
    //! `AllowInMultilineConditions`).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantParentheses")
            .collect()
    }

    /// (line, col, what the parentheses hold) of each offense.
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| {
                let what = o
                    .message
                    .strip_prefix("Don't use parentheses around ")
                    .and_then(|rest| rest.strip_suffix('.'))
                    .unwrap_or(&o.message)
                    .to_string();
                (o.line, o.col, what)
            })
            .collect()
    }

    /// RuboCop's `redundant` shared example: one offense at 1:1.
    fn assert_redundant(source: &str, what: &str) {
        assert_eq!(
            found(source),
            [(1, 1, what.to_string())],
            "{source:?} should be flagged as {what}"
        );
    }

    fn assert_accepted(source: &str) {
        assert!(
            found(source).is_empty(),
            "should accept {source:?}: {:?}",
            found(source)
        );
    }

    #[test]
    fn corrects_verified_parentheses() {
        // RuboCop expect_correction examples (MIT), plus one-pass nesting.
        for (before, after) in [
            ("(1)\n", "1\n"),
            ("x = (a && b)\n", "x = a && b\n"),
            ("(x)? y : z\n", "x ? y : z\n"),
            ("foo((1))\n", "foo(1)\n"),
            ("((1))\n", "(1)\n"),
            ("[\n  (1\n  ),\n  2\n]\n", "[\n  1,\n  2\n]\n"),
            ("(\n  foo # comment\n).bar\n", "foo # comment\n.bar\n"),
            (
                "x = 1\ncase 1\nin ^(x)\nend\n",
                "x = 1\ncase 1\nin ^x\nend\n",
            ),
            (
                "x = 1\ncase 1\nin ^( x )\nend\n",
                "x = 1\ncase 1\nin ^x\nend\n",
            ),
            ("x = (<<~A\n  a\nA\n)\n", "x = <<~A\n  a\nA\n"),
        ] {
            crate::cops::testing::assert_correction(&RedundantParentheses, before, after);
        }
    }

    #[test]
    fn leaves_multiline_parentheses_when_magic_line_values_outside_scope_would_change() {
        for source in [
            "def foo\n  (\n    1\n  )\nend\np __LINE__\n",
            "class Foo\n  (\n    1\n  )\nend\np __LINE__\n",
            "def foo\r\n  (\r\n    1\r\n  )\r\nend\np __LINE__\r\n",
            "def foo\n  (\n    1\n  )\nend\ndef bar\n  __LINE__\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        crate::cops::testing::assert_correction(
            &RedundantParentheses,
            "def foo\n  (1)\nend\np __LINE__\n",
            "def foo\n  1\nend\np __LINE__\n",
        );
    }

    #[test]
    fn leaves_parentheses_that_would_change_other_directives_scope() {
        for source in [
            "x = (\n# rubocop:disable Lint/Debugger\n1\n)\n",
            "(\nfail\n) # rustocop:disable Style/SignalException\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn flags_parentheses_closed_after_a_heredoc_body() {
        for (source, what) in [
            ("x = (<<~A\n  a\nA\n)\n", "a literal"),
            ("x = (\n  <<~A\n    a\nA\n)\n", "a literal"),
            ("x = (<<~A.strip\n  a\nA\n)\n", "a method call"),
            ("x = (foo(<<~A)\n  a\nA\n)\n", "a method call"),
            ("x = (<<~A\n  a #{b}\nA\n)\n", "a literal"),
            ("x = (<<~`A`\n  a\nA\n)\n", "a literal"),
        ] {
            assert_eq!(found(source), [(1, 5, what.to_string())], "{source:?}");
        }
        assert_accepted("x = (<<~A + <<~B\n  a\nA\n  b\nB\n)\n");
    }

    #[test]
    fn verifies_candidates_within_their_def_or_class() {
        // Verified inside the innermost def/class/module/sclass, as RuboCop's
        // `ReparsedEquivalence#reparse_scope`.
        let source = "class A\n  def f(i)\n    x = (-2) ** i\n    y = (1)\n  end\n\n  \
                      class << self\n    def g = (__LINE__)\n  end\n  z = (2)\nend\nw = (3)\n";
        assert_eq!(
            found(source),
            [
                (4, 9, "a literal".to_string()),
                (8, 13, "a keyword".to_string()),
                (10, 7, "a literal".to_string()),
                (12, 5, "a literal".to_string()),
            ]
        );
        // As in RuboCop, a scope whose fragment does not parse alone (the
        // heredoc body lies past the endless def) verifies nothing.
        assert_accepted("def f = (<<~A)\n  a\nA\n");
        // Many failing candidates across many methods.
        let many: String = (0..200)
            .map(|i| format!("  def m{i}(i)\n    x = (-2) ** i\n    (1)\n  end\n"))
            .collect();
        let source = format!("class A\n{many}end\n");
        let lines: Vec<u32> = found(&source).iter().map(|(line, _, _)| *line).collect();
        assert_eq!(lines, (0..200).map(|i| 4 * i + 4).collect::<Vec<u32>>());
    }

    #[test]
    fn flags_with_message() {
        let found = offenses("x = 1; (x)\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 8));
        assert_eq!(found[0].message, "Don't use parentheses around a variable.");
    }

    #[test]
    fn flags_literals_keywords_variables_and_constants() {
        for (source, what) in [
            ("(\"x\")", "a literal"),
            ("(\"#{x}\")", "a literal"),
            ("(:x)", "a literal"),
            ("(:\"#{x}\")", "a literal"),
            ("(1)", "a literal"),
            ("(1.2)", "a literal"),
            ("({})", "a literal"),
            ("([])", "a literal"),
            ("(nil)", "a literal"),
            ("(true)", "a literal"),
            ("(false)", "a literal"),
            ("(/regexp/)", "a literal"),
            ("(\"x\"; \"y\")", "a literal"),
            ("(1; 2)", "a literal"),
            ("(1i)", "a literal"),
            ("(1r)", "a literal"),
            ("((1..42))", "a literal"),
            ("((1...42))", "a literal"),
            ("((1..))", "a literal"),
            ("((..42))", "a literal"),
            ("(__FILE__)", "a keyword"),
            ("(__LINE__)", "a keyword"),
            ("(__ENCODING__)", "a keyword"),
            ("(self)", "a keyword"),
            ("(super)", "a keyword"),
            ("(super())", "a keyword"),
            ("(super(1, 2))", "a keyword"),
            ("(return)", "a keyword"),
            ("(return())", "a keyword"),
            ("(return(1))", "a keyword"),
            ("(defined?(:A))", "a keyword"),
            ("(@x)", "a variable"),
            ("(@@x)", "a variable"),
            ("($x)", "a variable"),
            ("(X)", "a constant"),
            ("(-> { x })", "an expression"),
            ("(lambda { x })", "an expression"),
            ("(proc { x })", "an expression"),
        ] {
            assert_redundant(source, what);
        }
    }

    #[test]
    fn flags_calls_and_operators() {
        for (source, what) in [
            ("(x)", "a method call"),
            ("(x y)", "a method call"),
            ("(x(1, 2))", "a method call"),
            ("(\"x\".to_sym)", "a method call"),
            ("(\"x\"&.to_sym)", "a method call"),
            ("(x[:y])", "a method call"),
            ("(@x[:y])", "a method call"),
            ("(@@x[:y])", "a method call"),
            ("($x[:y])", "a method call"),
            ("(X[:y])", "a method call"),
            ("(\"foo\"[0])", "a method call"),
            ("(foo[0][0])", "a method call"),
            ("([\"foo\"][0])", "a method call"),
            ("({0 => :a}[0])", "a method call"),
            ("(x; y)", "a method call"),
            ("(x.select { |item| item })", "a method call"),
            ("(x.select { _1 })", "a method call"),
            ("(x.select { it })", "a method call"),
            ("(x && y)", "a logical expression"),
            ("(x || y)", "a logical expression"),
            ("(x and y)", "a logical expression"),
            ("(x or y)", "a logical expression"),
            ("(x &&\n y)", "a logical expression"),
            ("(x or\n y)", "a logical expression"),
            ("(x == y)", "a comparison expression"),
            ("(x === y)", "a comparison expression"),
            ("(x != y)", "a comparison expression"),
            ("(x > y)", "a comparison expression"),
            ("(x >= y)", "a comparison expression"),
            ("(x < y)", "a comparison expression"),
            ("(x <= y)", "a comparison expression"),
            ("(var = 42)", "an assignment"),
            ("(@var = 42)", "an assignment"),
            ("(@@var = 42)", "an assignment"),
            ("($var = 42)", "an assignment"),
            ("(CONST = 42)", "an assignment"),
            ("(!x)", "a unary operation"),
            ("(~x)", "a unary operation"),
            ("(-x)", "a unary operation"),
            ("(+x)", "a unary operation"),
            ("(!x arg)", "a unary operation"),
            ("(!x.m arg)", "a unary operation"),
            ("(!super arg)", "a unary operation"),
            ("(!defined? arg)", "a unary operation"),
            ("(x y)", "a method call"),
            ("(expression in pattern)", "a one-line pattern matching"),
            ("(expression => pattern)", "a one-line pattern matching"),
        ] {
            assert_redundant(source, what);
        }
    }

    #[test]
    fn accepts_plausible_parentheses() {
        for source in [
            "(return 1, 2)",
            "(yield 1, 2)",
            "(super 1, 2)",
            "(defined? :A)",
            "(alias a b)",
            "(not 1)",
            "(a until b)",
            "(a while b)",
            "if (var = 42); end",
            "unless (var = 42); end",
            "while (var = 42); end",
            "until (var = 42); end",
            "if (foo.bar = 42); end",
            "if (foo&.bar = 42); end",
            "while (foo[0] = 42); end",
            "baz if (foo[0] = 42)",
            "(var + 42) > do_something",
            "foo((bar rescue baz))",
            "foo((x and y))",
            "foo((x or y))",
            "(!x arg) && foo",
            "(!x.m arg) && foo",
            "(!super arg) && foo",
            "(!yield arg) && foo",
            "(!defined? arg) && foo",
            "foo && (!x arg)",
            "foo && (!x.m arg)",
            "foo && (!super arg)",
            "foo && (!yield arg)",
            "foo && (!defined? arg)",
            "(!x).y",
            "-(1.foo)",
            "+(1.foo)",
            "-(1.foo.bar)",
            "+(1.foo.bar)",
            "foo(*(bar & baz))",
            "foo(*(bar + baz))",
            "foo(**(bar + baz))",
            "foo + (bar baz)",
            "()",
            "(-2)**2",
            "(-2.1)**2",
            "x = (foo; bar)",
            "x += (foo; bar)",
            "x + (foo; bar)",
            "x((foo; bar))",
            "(foo[key] & bar.baz).any?",
            "def foo((bar, baz)); end",
        ] {
            assert_accepted(&format!("{source}\n"));
        }
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, expected) in [
            (
                "(X) ? Y : N\n(X)? Y : N\n",
                vec![(1, 1, "a constant"), (2, 1, "a constant")],
            ),
            ("(x).y(z)\n", vec![(1, 1, "a method call")]),
            ("x.y((z))\n", vec![(1, 5, "a method argument")]),
            ("x&.y((z))\n", vec![(1, 6, "a method argument")]),
            ("x.y(z, (w))\n", vec![(1, 8, "a method argument")]),
            ("x.y((z + w))\n", vec![(1, 5, "a method argument")]),
            ("x.y((a..b))\n", vec![(1, 5, "a method argument")]),
            (
                "x.y((foo &&\n  bar\n))\n",
                vec![(1, 5, "a method argument")],
            ),
            (
                "(\n  ['a', 'b', 'c'].include?(foo)\n) && bar\n",
                vec![(1, 1, "a method call")],
            ),
            (
                "member_ids = (\n  [1, 2, 3].map { |i| i } # comment\n).uniq\n",
                vec![(1, 14, "a method call")],
            ),
            (
                "member_ids = (\n  [1, 2, 3].map { |i| i } # comment\n  ).uniq\n",
                vec![(1, 14, "a method call")],
            ),
            (
                "x = (\n  foo.bar # comment\n)\n",
                vec![(1, 5, "a method call")],
            ),
            ("if ((var = 42))\nend\n", vec![(1, 5, "an assignment")]),
            ("\"#{(foo)}\"\n", vec![(1, 4, "an interpolated expression")]),
            ("[(1)]\n", vec![(1, 2, "a literal")]),
            ("[(1\n)]\n", vec![(1, 2, "a literal")]),
            ("[(foo + bar)]\n", vec![(1, 2, "a method call")]),
            ("[\n  (\n    1\n  )\n]\n", vec![(2, 3, "a literal")]),
            ("[(1\n),]\n", vec![(1, 2, "a literal")]),
            ("[\n  (\n    1\n  ),\n  2\n]\n", vec![(2, 3, "a literal")]),
            (
                "[\n  x = (\n    1\n  ),\n  y = 2\n]\n",
                vec![(2, 7, "a literal")],
            ),
            ("{a: (1)}\n", vec![(1, 5, "a literal")]),
            ("{a: (1\n)}\n", vec![(1, 5, "a literal")]),
            ("{ a: (1\n),}\n", vec![(1, 6, "a literal")]),
            (
                "(0)**2\n(2)**2\n",
                vec![(1, 1, "a literal"), (2, 1, "a literal")],
            ),
            ("(2.1)**2\n", vec![(1, 1, "a literal")]),
            ("2**(-2)\n", vec![(1, 4, "a literal")]),
            ("2**(2)\n", vec![(1, 4, "a literal")]),
            (
                "scope :my_scope, (-> do\n  where(column: :value)\nend)\n",
                vec![(1, 18, "an expression")],
            ),
            (
                "scope :my_scope, (lambda {\n  where(column: :value)\n})\n",
                vec![(1, 18, "an expression")],
            ),
            (
                "scope :my_scope, (proc {\n  where(column: :value)\n})\n",
                vec![(1, 18, "an expression")],
            ),
            ("foo bar: (baz {\n}.qux)\n", vec![(1, 10, "a method call")]),
            (
                "foo bar: (baz {\n  do_something(_1)\n}.qux)\n",
                vec![(1, 10, "a method call")],
            ),
            (
                "foo (x.select { |item| item }).y\n",
                vec![(1, 5, "a method call")],
            ),
            (
                "(\n  x.select { |item| item.foo }\n).map(&:bar)\n",
                vec![(1, 1, "a method call")],
            ),
            ("def x\n  (foo; bar)\nend\n", vec![(2, 3, "a method call")]),
            (
                "def self.x\n  (foo; bar)\nend\n",
                vec![(2, 3, "a method call")],
            ),
            (
                "class A\n  ROOT = __FILE__\n  def same\n    (foo)\n  end\nend\n",
                vec![(4, 5, "a method call")],
            ),
            (
                "class A\n  def same\n    (foo)\n  end\n  LINE = __LINE__\nend\n",
                vec![(3, 5, "a method call")],
            ),
            (
                "class A\n  def same\n    (foo)\n  end\nend\nclass B\n  def same\n    (foo)\n  end\nend\n",
                vec![(3, 5, "a method call"), (8, 5, "a method call")],
            ),
            (
                "def x\n  baz\n  (foo; bar)\nend\n",
                vec![(3, 3, "a method call")],
            ),
            ("x do\n  (foo; bar)\nend\n", vec![(2, 3, "block body")]),
            ("let(:foo) { (bar + baz) }\n", vec![(1, 13, "block body")]),
            ("x do\n  (_1; bar)\nend\n", vec![(2, 3, "a variable")]),
            (
                "x do\n  baz\n  (foo; bar)\nend\n",
                vec![(3, 3, "a method call")],
            ),
            ("something do\n  (a..b)\nend\n", vec![(2, 3, "block body")]),
            ("something do\n  (a...b)\nend\n", vec![(2, 3, "block body")]),
            ("something { (a..b) }\n", vec![(1, 13, "block body")]),
            ("something do\n  (..b)\nend\n", vec![(2, 3, "block body")]),
            (
                "something do\n  (..b)\n  x\nend\n",
                vec![(2, 3, "block body")],
            ),
            ("something do\n  (a..)\nend\n", vec![(2, 3, "block body")]),
            (
                "something do\n  x\n  (a..)\nend\n",
                vec![(3, 3, "block body")],
            ),
            ("var = (foo && bar)\n", vec![(1, 7, "a logical expression")]),
            ("var = (foo || bar)\n", vec![(1, 7, "a logical expression")]),
            (
                "def foo\n  (x || y)\nend\n",
                vec![(2, 3, "a logical expression")],
            ),
            (
                "def foo\n  (x && y)\nend\n",
                vec![(2, 3, "a logical expression")],
            ),
            ("(x && y) && z\n", vec![(1, 1, "a logical expression")]),
            ("x && (y && z)\n", vec![(1, 6, "a logical expression")]),
            ("x && (y && z) && w\n", vec![(1, 6, "a logical expression")]),
            ("x && (y && z) || w\n", vec![(1, 6, "a logical expression")]),
            ("x || (y && z) && w\n", vec![(1, 6, "a logical expression")]),
            (
                "if cond\n  x\nelse\n  (y || z)\nend\n",
                vec![(4, 3, "a logical expression")],
            ),
            ("if x; y else (1) end\n", vec![(1, 14, "a literal")]),
            ("x == (42)\n", vec![(1, 6, "a literal")]),
            ("x > (42)\n", vec![(1, 5, "a literal")]),
            ("x >= (42)\n", vec![(1, 6, "a literal")]),
            ("x < (42)\n", vec![(1, 5, "a literal")]),
            ("x <= (42)\n", vec![(1, 6, "a literal")]),
            ("x =~ (/regexp/)\n", vec![(1, 6, "a literal")]),
            (
                "def foo\n  (bar in 0 | 1)\nend\n",
                vec![(2, 3, "a one-line pattern matching")],
            ),
            (
                "def foo\n  (bar => 0 | 1)\nend\n",
                vec![(2, 3, "a one-line pattern matching")],
            ),
            (
                "def self.foo\n  (bar in 0 | 1)\nend\n",
                vec![(2, 3, "a one-line pattern matching")],
            ),
            (
                "def self.foo\n  (bar => 0 | 1)\nend\n",
                vec![(2, 3, "a one-line pattern matching")],
            ),
            ("x(({ y: 1 }), z)\n", vec![(1, 3, "a literal")]),
            ("x ({ y: 1 }), ({ y: 1 })\n", vec![(1, 15, "a literal")]),
            (
                "(foo rescue bar)\n(foo rescue bar)\n",
                vec![(1, 1, "a one-line rescue"), (2, 1, "a one-line rescue")],
            ),
            (
                "if cond\n  (foo rescue bar)\nelse\n  42\nend\n",
                vec![(2, 3, "a one-line rescue")],
            ),
            ("return (42)\n", vec![(1, 8, "a literal")]),
            ("return (foo + bar)\n", vec![(1, 8, "a method call")]),
            (
                "foo(\n  (\n    1\n  ),\n  2\n)\n",
                vec![(2, 3, "a literal")],
            ),
            (
                "x = (<<-STRING\n  foo\nSTRING\n).strip\n",
                vec![(1, 5, "a literal")],
            ),
            (
                "x = (<<-STRING\n  foo\nSTRING\n\n).strip\n",
                vec![(1, 5, "a literal")],
            ),
            (
                "x = (<<-STRING).strip\n  foo\nSTRING\n",
                vec![(1, 5, "a literal")],
            ),
            (
                "x = (<<-STRING)&.strip\n  foo\nSTRING\n",
                vec![(1, 5, "a literal")],
            ),
            (
                "x = (<<-STRING) + 'bar'\n  foo\nSTRING\n",
                vec![(1, 5, "a literal")],
            ),
            (
                "x = (<<-STRING)[0]\n  foo\nSTRING\n",
                vec![(1, 5, "a literal")],
            ),
            (
                "[\n  (\n  <<-STRING\n    foo\n  STRING\n  ) ,\n  (\n  <<-STRING\n    bar\n  STRING\n  )\n]\n",
                vec![(2, 3, "a literal"), (7, 3, "a literal")],
            ),
        ] {
            let expected: Vec<(u32, u32, String)> = expected
                .into_iter()
                .map(|(line, col, what)| (line, col, what.to_string()))
                .collect();
            assert_eq!(found(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "x(1, (2 if y?))\n",
            "x(1, (2 unless y?))\n",
            "x(1, (2 while y?))\n",
            "x(1, (2 until y?))\n",
            "x(1, (y arg))\n",
            "x(1, (y&.z arg))\n",
            "x(1, (y.+ arg))\n",
            "x(1, (y&.+ arg))\n",
            "scope :my_scope, (lambda do\n  where(column: :value)\nend)\n",
            "scope :my_scope, (proc do\n  where(column: :value)\nend)\n",
            "foo bar: (baz do\nend.qux)\n",
            "foo bar: (baz do\nend.qux.quux)\n",
            "foo bar: (baz do\n  do_something(_1)\nend.qux)\n",
            "obj&.foo bar: (baz do\nend.qux)\n",
            "obj&.foo bar: (baz do\n  do_something(_1)\nend.qux)\n",
            "foo (x.select do |item| item end).y\n",
            "foo bar: (baz..)\nqux\n",
            "foo bar: (baz...)\nqux\n",
            "something do\n  x\n  (..b)\nend\n",
            "something do\n  (a..)\n  x\nend\n",
            "var = (a..)\nx\n",
            "(..1).cover?(0)\n",
            "(1..).cover?(42)\n",
            "var = (foo or bar)\n",
            "var = (foo and bar)\n",
            "(a 1, 2) && (1 + 1)\n",
            "(a)..(b)\n",
            "(a)...(b)\n",
            "(a..b)\n",
            "(a...b)\n",
            "do_something\n(a..b)\n",
            "do_something\n(a...b)\n",
            "(a || b)..c\n",
            "(a || b)...c\n",
            "a..(b || c)\n",
            "a...(b || c)\n",
            "/regexp/ =~ (b || c)\n",
            "regexp =~ (b || c)\n",
            "(x && y) || z\n",
            "x || (y && z)\n",
            "x || (y && z) || w\n",
            "(x || y) && z\n",
            "x && (y || z)\n",
            "x && (y || z) && w\n",
            "x - (y || z)\n",
            "(1 and 2) and (3 or 4)\n",
            "(1 or 2) or (3 and 4)\n",
            "x && (y == z)\n",
            "x = *(y || z)\n",
            "x = *(case true when true then false end)\n",
            "x = *(case when rand > 0.5 then 1 end)\n",
            "x(**(y || z))\n",
            "cond ? x : (y || z)\n",
            "(x || y).z\n",
            "(x == y).zero?\n",
            "x((prepare; perform))\n",
            "begin\n  do_something\nend while(bar)\n",
            "begin\n  do_something\nend until(bar)\n",
            "if x; y else(1) end\n",
            "if x; y else (1)end\n",
            "foo((bar in baz))\n",
            "obj&.foo((bar in baz))\n",
            "(foo in bar) && (baz in qux)\n",
            "(foo in bar) || (baz in qux)\n",
            "foo = (bar in baz)\n",
            "foo ||= (bar in baz)\n",
            "def foo = (bar in 0 | 1)\n",
            "def foo = (bar => 0 | 1)\n",
            "def self.foo = (bar in 0 | 1)\n",
            "def self.foo = (bar => 0 | 1)\n",
            "x ({ y: 1 }), z\n",
            "x ({ y: 1 }).merge({ y: 2 }), z\n",
            "x ({ y: 1 }.merge({ y: 2 })), z\n",
            "x ({ y: 1 }.merge({ y: 2 }).merge({ y: 3 })), z\n",
            "x&.y ({ z: 1 }), w\n",
            "method (:arg) { blah }\n",
            "method (arg)\n",
            "begin\n  some_method\nrescue(StandardError)\nend\n",
            "if (foo rescue bar)\n  do_something\nend\n",
            "while (foo rescue bar)\n  do_something\nend\n",
            "case (foo rescue bar)\nwhen foo\n  do_something\nend\n",
            "[\n  (foo rescue bar)\n]\n",
            "{\n  key: (foo rescue bar)\n}\n",
            "cond ? (foo rescue bar) : 42\n",
            "case foo\nwhen(Const)\n  bar\nend\n",
            "super ({\n  foo: bar,\n})\n",
            "yield ({\n  foo: bar,\n})\n",
            "super (\n  42\n)\n",
            "yield (\n  42\n)\n",
            "return (\n  42\n)\n",
        ] {
            assert_accepted(source);
        }
    }

    #[test]
    fn counts_children_as_rubocop_does() {
        // RuboCop's `children.one?` skips `nil`s: an `if` whose branches are
        // empty has one child, so its condition's parentheses are "singular".
        for (source, expected) in [
            ("if (test == 10)\nend\n", vec![(1, 4, "a method call")]),
            (
                "if (x > 10)\nelsif (x < 3)\nend\n",
                vec![(2, 7, "a method call")],
            ),
            ("unless (x > 10)\nend\n", vec![(1, 8, "a method call")]),
            ("while (x > 10)\nend\n", vec![(1, 7, "a method call")]),
            ("x += 1 if (x < 10)\n", vec![]),
            ("if (x > 10)\n  y\nend\n", vec![]),
        ] {
            let expected: Vec<(u32, u32, String)> = expected
                .into_iter()
                .map(|(line, col, what)| (line, col, what.to_string()))
                .collect();
            assert_eq!(found(source), expected, "{source:?}");
        }
    }

    #[test]
    fn handles_pins() {
        for variable in ["var", "@var", "@@var", "$var"] {
            let source = format!("var = 0\nfoo in {{ bar: ^({variable}) }}\n");
            assert_eq!(
                found(&source),
                [(2, 16, "a variable".to_string())],
                "{source:?}"
            );
        }
        for expression in [
            "meth",
            "meth(true)",
            "var.to_i",
            "@var.to_i",
            "@@var.to_i",
            "$var.to_i",
            "var + 1",
            "[1, 2]",
            "{ baz: 2 }",
            "1..2",
            "1",
        ] {
            assert_accepted(&format!("var = 0\nfoo in {{ bar: ^({expression}) }}\n"));
        }
    }

    #[test]
    fn flags_yield_inside_a_method() {
        // Prism rejects `yield` outside a method; RuboCop's spec has it at
        // the top level.
        for (source, what) in [
            ("(yield)", "a keyword"),
            ("(yield())", "a keyword"),
            ("(yield(1, 2))", "a keyword"),
        ] {
            assert_eq!(
                found(&format!("def m\n  {source}\nend\n")),
                [(2, 3, what.to_string())],
                "{source:?}"
            );
        }
        for source in ["(yield 1, 2)", "(!yield arg) && foo", "foo && (!yield arg)"] {
            assert_accepted(&format!("def m\n  {source}\nend\n"));
        }
    }

    #[test]
    fn verification_rejects_changed_meaning() {
        // The rules pick these, but without the parentheses Ruby reads them
        // differently: `-1` is a literal, not `1.-@`.
        assert_accepted("-(1)\n");
        assert_accepted("(-2)**2\n");
        // `-(2)**2` and `-2**2` both negate `2**2`.
        assert_eq!(found("-(2)**2\n"), [(1, 2, "a literal".to_string())]);
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("(x) # rubocop:disable Style/RedundantParentheses\n").is_empty());
    }
}
