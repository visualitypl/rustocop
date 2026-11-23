//! `Layout/MultilineMethodCallIndentation` — fixed `aligned` style.
//!
//! For every `send`/`csend` with a dot whose selector (with the dot when on
//! the same line) begins its line, the selector must align with a base: a
//! dot directly above, the continuation of a multi-line block chain, the
//! first call of the chain, or syntactically the keyword expression,
//! assignment value or operand holding the call. Inside a hash pair the
//! first dot of the chain is the base. Without a base it must be indented
//! one step (two for a prefix keyword expression) from its chain's first
//! line. Port of RuboCop's cop and its `MultilineExpressionIndentation`
//! mixin over the parser-gem ancestor view of [`super::multiline_expression`].
use super::multiline_expression::{
    Anc, Parens, Span, any_block, argument_in_method_call, assignment_method, block_delimiters,
    call_type, column, correct_indentation, dot, grouped_expression, inside_arg_list_parentheses,
    keyword_node, kwbegin, line_indentation, literal_block, method_name, node_span,
    not_for_this_cop, operation_description, parser_ancestors, parser_span, part_of_assignment_rhs,
    receiver, selector, span, ternary, within,
};
use crate::{
    cops::{
        Cop, NodeContext,
        layout::alignment::{Alignment, shift_lines, starts_line},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MultilineMethodCallIndentation;

impl Cop for MultilineMethodCallIndentation {
    fn name(&self) -> &'static str {
        "Layout/MultilineMethodCallIndentation"
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
        if !call_type(node) || method_name(node) == b"[]" {
            return;
        }
        let (Some(_), Some(dot)) = (receiver(node), dot(node)) else {
            return;
        };
        let Some(rhs) = right_hand_side(node, &dot) else {
            return;
        };
        if !starts_line(file, rhs.start) {
            return;
        }
        let prism: Vec<&Node<'pr>> = cx.ancestors().collect();
        let ancestors = parser_ancestors(node, &prism);
        let check = Check {
            file,
            node,
            span: parser_span(node),
            chain: chain(node),
            ancestors: &ancestors,
            rhs: rhs.clone(),
        };
        let Some(found) = check.offending_range() else {
            return;
        };
        let message = match &found.base {
            Some(base) => format!(
                "Align `{}` with `{}` on line {}.",
                text(file, &rhs),
                first_line_text(file, base),
                file.line_of(base.start)
            ),
            None => {
                let lhs = check.left_hand_side();
                format!(
                    "Use {} (not {}) spaces for indenting {} spanning multiple lines.",
                    correct_indentation(&check.span, &ancestors),
                    column(file, rhs.start) as i64 - line_indentation(file, lhs.start) as i64,
                    operation_description(&check.span, &rhs, &ancestors),
                )
            }
        };
        let (line, col) = file.line_col(rhs.start);
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = prism.last().copied().unwrap_or(node);
                check.correction(root, found.delta)
            })
            .flatten();
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(rhs.end.saturating_sub(1).max(rhs.start)),
        });
    }
}

fn text<'a>(file: &'a SourceFile, span: &Span) -> std::borrow::Cow<'a, str> {
    String::from_utf8_lossy(&file.bytes[span.clone()])
}

fn first_line_text<'a>(file: &'a SourceFile, span: &Span) -> std::borrow::Cow<'a, str> {
    let bytes = &file.bytes[span.clone()];
    let end = bytes
        .iter()
        .position(|b| *b == b'\n')
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
}

/// `right_hand_side`: the dot and selector when on one line (`.`/`&.`), the
/// selector alone, or `.(` for an implicit `call`.
fn right_hand_side(node: &Node<'_>, dot: &ruby_prism::Location<'_>) -> Option<Span> {
    let selector = selector(node);
    let file_line = |offset: usize, other: usize| {
        // Same line: no line break between the two offsets.
        !node_bytes_contain_newline(node, offset, other)
    };
    match selector {
        Some(selector)
            if matches!(dot.as_slice(), b"." | b"&.")
                && file_line(dot.end_offset(), selector.start_offset()) =>
        {
            Some(dot.start_offset()..selector.end_offset())
        }
        Some(selector) => Some(span(&selector)),
        None if method_name(node) == b"call" => {
            let open = node.as_call_node()?.opening_loc()?;
            Some(dot.start_offset()..open.end_offset())
        }
        None => None,
    }
}

/// Whether the source between two offsets of `node` holds a line break.
fn node_bytes_contain_newline(node: &Node<'_>, from: usize, to: usize) -> bool {
    let location = node.location();
    let base = location.start_offset();
    let bytes = location.as_slice();
    let (from, to) = (from.min(to), from.max(to));
    bytes
        .get(from.saturating_sub(base)..to.saturating_sub(base))
        .is_some_and(|b| b.contains(&b'\n'))
}

struct Found {
    base: Option<Span>,
    delta: i64,
}

struct Check<'c, 'b, 'pr> {
    file: &'c SourceFile,
    node: &'b Node<'pr>,
    span: Span,
    /// `node`, its receiver, the receiver's receiver, ... down to the base
    /// receiver (`find_base_receiver`).
    chain: Vec<Node<'pr>>,
    ancestors: &'c [Anc<'b, 'pr>],
    rhs: Span,
}

/// `node` and its receivers, through calls, index calls and blocks.
fn chain<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    let mut out = Vec::new();
    let mut next = receiver(node);
    while let Some(current) = next {
        next = receiver(&current);
        out.push(current);
    }
    out
}

/// `dot.join(selector)` of a dispatch with both.
fn dot_selector(node: &Node<'_>) -> Option<Span> {
    let (dot, selector) = (dot(node)?, selector(node)?);
    Some(dot.start_offset()..selector.end_offset())
}

impl<'pr> Check<'_, '_, 'pr> {
    fn line(&self, offset: usize) -> u32 {
        self.file.line_of(offset)
    }

    fn last_line(&self, span: &Span) -> u32 {
        self.line(span.end.saturating_sub(1).max(span.start))
    }

    /// `BlockNode#single_line?` of a parser block.
    fn block_single_line(&self, node: &Node<'_>) -> bool {
        block_delimiters(node).is_some_and(|(open, close)| self.line(open) == self.line(close))
    }

    fn rhs_starts_with_dot(&self) -> bool {
        let bytes = &self.file.bytes[self.rhs.clone()];
        bytes.starts_with(b".") || bytes.starts_with(b"&.")
    }

    /// The chain element at `index`: 0 is `node`, then its receivers.
    fn at(&self, index: usize) -> &Node<'pr> {
        if index == 0 {
            self.node
        } else {
            &self.chain[index - 1]
        }
    }

    fn base_index(&self) -> usize {
        self.chain.len()
    }

    /// `left_hand_side(node.receiver)`: climb from the receiver through
    /// dotted, non-assignment calls.
    fn left_hand_side(&self) -> Span {
        let climbs =
            |n: &Node<'_>| call_type(n) && dot(n).is_some() && !assignment_method(method_name(n));
        if !climbs(self.node) {
            return parser_span(self.at(1));
        }
        let mut lhs = self.span.clone();
        for a in self.ancestors {
            match a {
                Anc::Call(call) if climbs(call) => lhs = a.span(),
                _ => break,
            }
        }
        lhs
    }

    fn offending_range(&self) -> Option<Found> {
        if let Some(pair) = self.find_pair_ancestor() {
            return self.check_hash_pair_indentation(pair);
        }
        if not_for_this_cop(&self.span, self.ancestors) {
            return None;
        }
        let base = self
            .semantic_alignment_base()
            .or_else(|| self.syntactic_alignment_base());
        let correct = match &base {
            Some(base) => column(self.file, base.start),
            None => {
                line_indentation(self.file, self.left_hand_side().start)
                    + correct_indentation(&self.span, self.ancestors)
            }
        };
        self.delta(base, correct)
    }

    fn delta(&self, base: Option<Span>, correct: usize) -> Option<Found> {
        let delta = correct as i64 - column(self.file, self.rhs.start) as i64;
        (delta != 0).then_some(Found { base, delta })
    }

    /// `find_pair_ancestor`: the index of the hash pair holding the call,
    /// unless a grouping, argument parentheses or a body comes first.
    fn find_pair_ancestor(&self) -> Option<usize> {
        for (index, a) in self.ancestors.iter().enumerate() {
            if matches!(a, Anc::Other(Node::AssocNode { .. })) {
                return Some(index);
            }
            if grouped_expression(a) || inside_arg_list_parentheses(&self.span, a) {
                return None;
            }
            let body = match a {
                Anc::Block { body, .. } => *body,
                Anc::Other(n) => {
                    matches!(n, Node::IfNode { .. } | Node::UnlessNode { .. }) && !ternary(n)
                        || kwbegin(n)
                        || n.as_while_node().is_some_and(|w| !w.is_begin_modifier())
                        || n.as_until_node().is_some_and(|w| !w.is_begin_modifier())
                        || n.as_for_node().is_some()
                }
                Anc::Call(_) => false,
            };
            if body {
                return None;
            }
        }
        None
    }

    fn check_hash_pair_indentation(&self, pair: usize) -> Option<Found> {
        let mut base = self.find_hash_pair_alignment_base();
        if base.is_none() && self.inside_multiline_chain_arg(pair) {
            return None;
        }
        if base.is_none() {
            base = Some(
                self.first_dot_alignment_base()
                    .unwrap_or_else(|| self.left_hand_side()),
            );
        }
        if self.aligned_with_first_line_dot() {
            return None;
        }
        let correct = column(self.file, base.as_ref().unwrap().start);
        self.delta(base, correct)
    }

    /// `find_hash_pair_alignment_base`: a chain on a hash literal aligns with
    /// its first dot.
    fn find_hash_pair_alignment_base(&self) -> Option<Span> {
        if !matches!(self.at(self.base_index()), Node::HashNode { .. }) {
            return None;
        }
        let first = self.at(self.first_call_has_a_dot());
        self.dot_and_selector(first)
    }

    /// `dot.join(selector)`, or the dot alone without a selector.
    fn dot_and_selector(&self, node: &Node<'_>) -> Option<Span> {
        let dot = dot(node)?;
        Some(dot.start_offset()..selector(node).map_or(dot.end_offset(), |s| s.end_offset()))
    }

    /// `inside_multiline_chain_arg?`.
    fn inside_multiline_chain_arg(&self, pair: usize) -> bool {
        let (Some(hash), Some(call)) = (self.ancestors.get(pair + 1), self.ancestors.get(pair + 2))
        else {
            return false;
        };
        let Anc::Call(call) = call else {
            return false;
        };
        let Some(call_receiver) = receiver(call) else {
            return false;
        };
        if !call_type(call) || dot(call).is_none() || node_span(&call_receiver) == hash.span() {
            return false;
        }
        selector(call).is_none_or(|s| {
            self.line(s.start_offset()) != self.line(call_receiver.location().start_offset())
        })
    }

    /// `first_call_has_a_dot`: the index of the lowest chain call with a dot.
    fn first_call_has_a_dot(&self) -> usize {
        let mut index = self.base_index().saturating_sub(1);
        while index > 0 && dot(self.at(index)).is_none() {
            index -= 1;
        }
        index
    }

    /// `first_dot_alignment_base`.
    fn first_dot_alignment_base(&self) -> Option<Span> {
        if !self.rhs_starts_with_dot() {
            return None;
        }
        let index = self.first_call_has_a_dot();
        let first = self.at(index);
        let dot = dot(first)?;
        if index == 0 {
            return None;
        }
        if let Some(base) = self.after_multiline_block_base(index) {
            return Some(base);
        }
        let first_receiver = receiver(first)?;
        if self.line(dot.start_offset()) != self.line(first_receiver.location().start_offset()) {
            return None;
        }
        self.dot_and_selector(first)
    }

    /// `after_multiline_block_base`.
    fn after_multiline_block_base(&self, index: usize) -> Option<Span> {
        let first = self.at(index);
        literal_block(first)?;
        if self.block_single_line(first) || index < 2 {
            return None;
        }
        let after = self.at(index - 1);
        (call_type(after) && dot(after).is_some())
            .then(|| self.dot_and_selector(after))
            .flatten()
    }

    /// `aligned_with_first_line_dot?`.
    fn aligned_with_first_line_dot(&self) -> bool {
        if !self.rhs_starts_with_dot() {
            return false;
        }
        let index = self.first_call_has_a_dot();
        if index == 1 {
            return false;
        }
        let Some(dot) = dot(self.at(index)) else {
            return false;
        };
        self.line(dot.start_offset()) == self.line(self.span.start)
            && column(self.file, dot.start_offset()) == column(self.file, self.rhs.start)
    }

    /// `semantic_alignment_base`.
    fn semantic_alignment_base(&self) -> Option<Span> {
        if !self.rhs_starts_with_dot() {
            return None;
        }
        self.semantic_alignment_node()?
    }

    /// `semantic_alignment_node`, as the found node's `dot.join(selector)`
    /// (`None` inside when it has no dot or selector).
    fn semantic_alignment_node(&self) -> Option<Option<Span>> {
        if argument_in_method_call(&self.span, Parens::Required, self.ancestors).is_some() {
            return None;
        }
        self.get_dot_right_above()
            .or_else(|| self.find_multiline_block_chain_node())
            .or_else(|| self.first_call_alignment_node())
    }

    /// `get_dot_right_above`: an ancestor whose dot is on the line above, at
    /// the same column.
    fn get_dot_right_above(&self) -> Option<Option<Span>> {
        let own = dot(self.node)?;
        let line = self.line(own.start_offset());
        let col = column(self.file, own.start_offset());
        self.ancestors.iter().find_map(|a| {
            let Anc::Call(call) = a else { return None };
            let d = dot(call)?;
            (self.line(d.start_offset()) + 1 == line && column(self.file, d.start_offset()) == col)
                .then(|| dot_selector(call))
        })
    }

    /// `find_multiline_block_chain_node`.
    fn find_multiline_block_chain_node(&self) -> Option<Option<Span>> {
        if literal_block(self.node).is_some() {
            self.find_continuation_node()
        } else {
            self.handle_descendant_block()
        }
    }

    fn single_line_block_receiver(&self) -> bool {
        let receiver = self.at(1);
        any_block(receiver) && self.block_single_line(receiver)
    }

    /// `leftmost_call_on_same_line(node.receiver)`.
    fn leftmost_call_on_same_line(&self) -> Option<Option<Span>> {
        let mut index = 1;
        loop {
            let current = self.at(index);
            let Some(current_dot) = dot(current) else {
                break;
            };
            if index + 1 > self.base_index() {
                break;
            }
            let next = self.at(index + 1);
            match dot(next) {
                Some(next_dot)
                    if call_type(next)
                        && self.line(next_dot.start_offset())
                            == self.line(current_dot.start_offset()) =>
                {
                    index += 1;
                }
                _ => break,
            }
        }
        Some(dot_selector(self.at(index)))
    }

    /// `find_continuation_node`.
    fn find_continuation_node(&self) -> Option<Option<Span>> {
        if self.single_line_block_receiver() {
            return self.leftmost_call_on_same_line();
        }
        let receiver = self.at(1);
        if !call_type(receiver) || literal_block(receiver).is_some() {
            return None;
        }
        let receiver_dot = dot(receiver)?;
        let inner = self.chain.get(1)?;
        if matches!(inner, Node::ParenthesesNode { .. }) && self.block_single_line(self.node) {
            return Some(dot_selector(receiver));
        }
        (self.line(receiver_dot.start_offset()) > self.last_line(&node_span(inner)))
            .then(|| dot_selector(receiver))
    }

    /// `handle_descendant_block`: with a multi-line block in the call's
    /// receiver or arguments, the receiver call (or that block's parent).
    fn handle_descendant_block(&self) -> Option<Option<Span>> {
        if self.single_line_block_receiver() {
            return self.leftmost_call_on_same_line();
        }
        let ((open, close), parent) = first_descendant_block(self.node)?;
        if self.line(open) == self.line(close) {
            return None;
        }
        let receiver = self.at(1);
        if call_type(receiver) && literal_block(receiver).is_none() {
            Some(dot_selector(receiver))
        } else {
            Some(parent)
        }
    }

    /// `first_call_alignment_node`.
    fn first_call_alignment_node(&self) -> Option<Option<Span>> {
        let first = self.at(self.first_call_has_a_dot());
        let base = self.at(self.base_index());
        let dot = dot(first)?;
        let dot_line = self.line(dot.start_offset());
        let base_end_line = self.line(base.location().end_offset());
        if dot_line == base_end_line && matches!(base, Node::ArrayNode { .. }) {
            return Some(dot_selector(first));
        }
        if dot_line != self.line(first.location().start_offset()) {
            return None;
        }
        if dot_line == base_end_line && matches!(base, Node::ParenthesesNode { .. }) {
            return None;
        }
        Some(dot_selector(first))
    }

    /// `syntactic_alignment_base`.
    fn syntactic_alignment_base(&self) -> Option<Span> {
        if let Some((_, expression)) = keyword_node(&self.span, self.ancestors) {
            return Some(expression);
        }
        if let Some(rhs) = part_of_assignment_rhs(Some(&self.rhs), self.ancestors) {
            return Some(node_span(&rhs));
        }
        self.operation_rhs()
    }

    /// `operation_rhs`: the first operand of an operator `send` whose first
    /// argument holds the receiver.
    fn operation_rhs(&self) -> Option<Span> {
        let receiver = node_span(self.at(1));
        let own = std::iter::once(Anc::Call(self.node));
        own.chain(self.ancestors.iter().copied()).find_map(|a| {
            let Anc::Call(call) = a else { return None };
            if !a.send() || !crate::cops::ast::is_operator_method(method_name(call)) {
                return None;
            }
            let first = super::multiline_expression::arguments(call)
                .into_iter()
                .next()?;
            let first = node_span(&first);
            within(&receiver, &first).then_some(first)
        })
    }

    /// Upstream's autocorrect: the selector alone, or with a literal block
    /// its selector line and the block's body and closing lines.
    fn correction(&self, root: &Node<'_>, delta: i64) -> Option<crate::offense::Correction> {
        let Some(block) = literal_block(self.node) else {
            return Alignment {
                expression: self.rhs.clone(),
                delta,
            }
            .correction(self.file, root);
        };
        let selector_line = self.line(self.rhs.start);
        let mut lines = vec![selector_line];
        if let Some(body) = block.body() {
            let body = node_span(&body);
            lines.extend(self.line(body.start)..=self.last_line(&body));
        }
        let closing = block.closing_loc();
        lines.push(self.line(closing.start_offset()));
        lines.sort_unstable();
        lines.dedup();
        shift_lines(self.file, root, lines, closing.end_offset(), delta)
    }
}

/// The first parser block (in pre-order) in `node`'s receiver and arguments:
/// its delimiters and its parent's `dot.join(selector)`.
fn first_descendant_block(node: &Node<'_>) -> Option<((usize, usize), Option<Span>)> {
    struct Finder<'pr> {
        stack: Vec<Node<'pr>>,
        found: Option<((usize, usize), Option<Span>)>,
    }
    impl<'pr> NodeVisitor<'pr> for Finder<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            if self.found.is_none()
                && let Some(delimiters) = block_delimiters(&node)
            {
                self.found = Some((delimiters, self.parent()));
            }
            self.stack.push(node);
        }
        fn leave(&mut self) {
            self.stack.pop();
        }
    }
    impl Finder<'_> {
        /// The parser parent of the node about to be entered.
        fn parent(&self) -> Option<Span> {
            let refs: Vec<&Node<'_>> = self.stack.iter().rev().collect();
            let mut probe = refs.into_iter();
            // Skip transparent Prism wrappers: argument lists and bodies.
            for n in probe.by_ref() {
                match n {
                    Node::ArgumentsNode { .. } => continue,
                    Node::StatementsNode { .. }
                        if n.as_statements_node().unwrap().body().len() < 2 =>
                    {
                        continue;
                    }
                    _ if call_type(n) => return dot_selector(n),
                    _ => return None,
                }
            }
            None
        }
    }
    let mut finder = Finder {
        stack: Vec::new(),
        found: None,
    };
    let call = node.as_call_node()?;
    // The search root's own parent is `node`.
    if let Some(receiver) = call.receiver() {
        finder.stack.push(clone_node(node));
        walk(&mut finder, &receiver);
        finder.stack.clear();
    }
    if finder.found.is_none() {
        for argument in super::multiline_expression::arguments(node) {
            finder.stack.push(clone_node(node));
            walk(&mut finder, &argument);
            finder.stack.clear();
            if finder.found.is_some() {
                break;
            }
        }
    }
    finder.found
}

/// A second handle to a call node.
fn clone_node<'pr>(node: &Node<'pr>) -> Node<'pr> {
    node.as_call_node().unwrap().as_node()
}

#[cfg(test)]
mod tests;
