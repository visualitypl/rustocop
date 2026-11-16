//! `Lint/Void` — flags operators, literals, variables, constants, `self`,
//! `defined?`, lambdas and procs whose value is thrown away.
//!
//! Matches RuboCop's semantics with its default
//! `CheckForMethodsWithNoSideEffects: false` (fixed), which are
//! phrased over the parser gem's `begin` nodes. Prism has no such node; the
//! cop maps them:
//!
//! - a statement sequence of two or more (`StatementsNode`) is a `begin`;
//! - parentheses are always one (`(a)`, `()`), and so is an explicit
//!   `begin`…`end` without `rescue`/`ensure` (a `kwbegin`), each holding its
//!   statements directly.
//!
//! In each, every statement but the last is checked. The last one is also
//! checked when the sequence is the whole body of an `initialize` method, a
//! `tap` block, a `for` loop or an `ensure` clause (RuboCop's void
//! contexts; setter methods are excluded), unless the nearest enclosing
//! block is an `each` block. Inside an `each` block, operators are never
//! reported: `enumerator.each { |x| x >= 2 }` can filter. A `tap` block or
//! `ensure` clause with a single statement checks it too (the `ensure` one
//! skips operators, as RuboCop's `on_ensure` does); a lone statement of an
//! `initialize` method or `for` loop is not, unless it is parenthesized or a
//! `begin` block (RuboCop has no `begin` node otherwise).
//!
//! A checked statement is reported if it is:
//!
//! - an operator call (`a + b`, `!a`, `a.==(b)`; not `a.+` without
//!   arguments), at the operator, possibly inside parentheses;
//! - a literal built entirely of literals (`1`, `"#{x}"`, `[1, [2]]`,
//!   `{a: 1}`, `'a'.freeze`), but not a range, `nil` or a backtick command;
//! - a variable, constant, `__ENCODING__` or `self`;
//! - `defined?(…)`, a lambda (`-> {}`, `lambda {}`) or a proc (`proc {}`,
//!   `Proc.new {}`, `Proc.new`; as in RuboCop, not with numbered or `it`
//!   parameters);
//! - an `if`/`unless`/ternary, or a `case`, whose branch body is a single
//!   such statement (only the keyword branch of an `if`; every branch and the
//!   `else` of a `case`).
//!
//! Messages quote the whole reference source buffer, including multiline
//! expressions and its encoding/CRLF projection.
//!
//! A single ordered walk from the program maintains synthetic call/block
//! contexts and statement-list ownership; it never rescans the root per node.

use std::collections::HashSet;

use ruby_prism::{
    BeginNode, BlockNode, CallNode, DefNode, EnsureNode, ForNode, ForwardingSuperNode, LambdaNode,
    Node, ParenthesesNode, StatementsNode, SuperNode, Visit,
};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{is_const, is_literal, is_variable},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Lint/Void` cop.
pub struct Void;

impl Cop for Void {
    fn name(&self) -> &'static str {
        "Lint/Void"
    }

    fn safe_fixes(&self) -> bool {
        true
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
        let Some(program) = node.as_program_node() else {
            return;
        };
        let mut walker = Walker {
            cop: self,
            file,
            out,
            corrections: cx.corrections_requested(),
            setters: Vec::new(),
            blocks: Vec::new(),
            void_lists: HashSet::new(),
            void_units: HashSet::new(),
            inner_bodies: HashSet::new(),
            reported: HashSet::new(),
        };
        walker.visit_program_node(&program);
    }
}

const BINARY_OPERATORS: [&[u8]; 13] = [
    b"*", b"/", b"%", b"+", b"-", b"==", b"===", b"!=", b"<", b">", b"<=", b">=", b"<=>",
];

const UNARY_OPERATORS: [&[u8]; 4] = [b"+@", b"-@", b"~", b"!"];

/// A node's byte range: identifies a node within one parse.
type Span = (usize, usize);

fn span(node: &Node<'_>) -> Span {
    let location = node.location();
    (location.start_offset(), location.end_offset())
}

/// A block the walk is inside (RuboCop's `any_block`; as in its AST, it
/// encloses the call's receiver and arguments too).
#[derive(Debug, Clone, Copy)]
struct Block {
    /// An `each` block.
    each: bool,
    /// An `each` or `tap` block: its body is a void context.
    void: bool,
}

const OTHER_BLOCK: Block = Block {
    each: false,
    void: false,
};

/// Walks the whole tree, checking the parser gem's `begin` equivalents.
struct Walker<'a> {
    cop: &'a Void,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    corrections: bool,
    /// Enclosing method definitions: whether each is a setter.
    setters: Vec<bool>,
    /// Enclosing blocks, innermost last.
    blocks: Vec<Block>,
    /// Sequences that are the whole body of a void context, by span; marked
    /// by the context before they are visited.
    void_lists: HashSet<Span>,
    /// Likewise for a lone parenthesized or `begin` statement. (Kept apart
    /// from `void_lists`: a one-statement list spans its statement.)
    void_units: HashSet<Span>,
    /// Statement lists of parentheses and `begin`s, which are checked with
    /// their owner rather than on their own.
    inner_bodies: HashSet<Span>,
    /// Offenses reported so far, by offset and message: a statement can be
    /// checked twice (`(a + b; c)` as a statement and as a sequence), and
    /// RuboCop keeps one offense per location.
    reported: HashSet<(usize, String)>,
}

impl<'pr> Visit<'pr> for Walker<'_> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        let block = node
            .block()
            .is_some_and(|block| block.as_block_node().is_some());
        if !block {
            ruby_prism::visit_call_node(self, node);
            return;
        }
        let name = node.name().as_slice();
        self.blocks.push(Block {
            each: name == b"each",
            void: name == b"each" || name == b"tap",
        });
        ruby_prism::visit_call_node(self, node);
        self.blocks.pop();
    }

    fn visit_super_node(&mut self, node: &SuperNode<'pr>) {
        let block = node
            .block()
            .is_some_and(|block| block.as_block_node().is_some());
        self.with_block(block, |walker| ruby_prism::visit_super_node(walker, node));
    }

    fn visit_forwarding_super_node(&mut self, node: &ForwardingSuperNode<'pr>) {
        let block = node.block().is_some();
        self.with_block(block, |walker| {
            ruby_prism::visit_forwarding_super_node(walker, node)
        });
    }

    fn visit_lambda_node(&mut self, node: &LambdaNode<'pr>) {
        self.with_block(true, |walker| ruby_prism::visit_lambda_node(walker, node));
    }

    fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
        // The enclosing call pushed this block.
        let block = self.blocks.last().copied().unwrap_or(OTHER_BLOCK);
        if block.void
            && let Some(body) = node.body()
            && let Some(statements) = body.as_statements_node()
        {
            match self.void_body(&statements) {
                // RuboCop's `on_block`, for a `tap` block's lone statement.
                Some(statement) if !block.each => {
                    self.check_void_op(&statement);
                    self.check_expression(&statement);
                }
                _ => {}
            }
        }
        ruby_prism::visit_block_node(self, node);
    }

    fn visit_def_node(&mut self, node: &DefNode<'pr>) {
        // `initialize` is a void context; setters are too, but RuboCop then
        // skips their last statement anyway.
        if node.receiver().is_none()
            && node.name().as_slice() == b"initialize"
            && let Some(body) = node.body()
            && let Some(statements) = body.as_statements_node()
        {
            self.void_body(&statements);
        }
        let name = node.name().as_slice();
        self.setters
            .push(name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"===" | b"<=" | b">="));
        ruby_prism::visit_def_node(self, node);
        self.setters.pop();
    }

    fn visit_for_node(&mut self, node: &ForNode<'pr>) {
        if let Some(statements) = node.statements() {
            self.void_body(&statements);
        }
        ruby_prism::visit_for_node(self, node);
    }

    fn visit_ensure_node(&mut self, node: &EnsureNode<'pr>) {
        if let Some(statements) = node.statements()
            && let Some(statement) = self.void_body(&statements)
        {
            // RuboCop's `on_ensure`: no operator check.
            self.check_expression(&statement);
        }
        ruby_prism::visit_ensure_node(self, node);
    }

    fn visit_statements_node(&mut self, node: &StatementsNode<'pr>) {
        let key = span(&node.as_node());
        if !self.inner_bodies.remove(&key) {
            let void = self.void_lists.remove(&key);
            let body = node.body();
            if body.len() >= 2 {
                self.check_begin(body.iter(), body.len(), void);
            }
        }
        ruby_prism::visit_statements_node(self, node);
    }

    fn visit_parentheses_node(&mut self, node: &ParenthesesNode<'pr>) {
        let void = self.void_units.remove(&span(&node.as_node()));
        match node.body() {
            None => {}
            Some(body) => match body.as_statements_node() {
                Some(list) => {
                    self.inner_bodies.insert(span(&body));
                    let list = list.body();
                    self.check_begin(list.iter(), list.len(), void);
                }
                None => self.check_begin(std::iter::once(body), 1, void),
            },
        }
        ruby_prism::visit_parentheses_node(self, node);
    }

    fn visit_begin_node(&mut self, node: &BeginNode<'pr>) {
        // A `kwbegin` holding its statements directly; with `rescue` or
        // `ensure` it holds only that clause, which is never reported.
        if node.begin_keyword_loc().is_some()
            && node.rescue_clause().is_none()
            && node.ensure_clause().is_none()
        {
            let void = self.void_units.remove(&span(&node.as_node()));
            if let Some(list) = node.statements() {
                self.inner_bodies.insert(span(&list.as_node()));
                let list = list.body();
                self.check_begin(list.iter(), list.len(), void);
            }
        }
        ruby_prism::visit_begin_node(self, node);
    }
}

impl<'pr> Walker<'_> {
    fn with_block(&mut self, block: bool, visit: impl FnOnce(&mut Self)) {
        if block {
            self.blocks.push(OTHER_BLOCK);
        }
        visit(self);
        if block {
            self.blocks.pop();
        }
    }

    /// Marks the parser gem's node for a void context's body: a sequence of
    /// two or more, or a lone parenthesized or `begin` statement. Returns any
    /// other lone statement.
    fn void_body(&mut self, statements: &StatementsNode<'pr>) -> Option<Node<'pr>> {
        let mut body = statements.body().iter();
        let first = body.next()?;
        if body.next().is_some() {
            self.void_lists.insert(span(&statements.as_node()));
            return None;
        }
        let unit = match &first {
            Node::ParenthesesNode { .. } => true,
            Node::BeginNode { .. } => first.as_begin_node().unwrap().begin_keyword_loc().is_some(),
            _ => false,
        };
        if unit {
            self.void_units.insert(span(&first));
        }
        // RuboCop's `on_block`/`on_ensure` skip only parentheses.
        (first.as_parentheses_node().is_none()).then_some(first)
    }

    /// RuboCop's `check_begin`, over a sequence's `len` statements.
    fn check_begin(&mut self, statements: impl Iterator<Item = Node<'pr>>, len: usize, void: bool) {
        let inside_each = self.blocks.last().is_some_and(|block| block.each);
        let checked = if void && !inside_each {
            len
        } else {
            len.saturating_sub(1)
        };
        for statement in statements.take(checked) {
            if !inside_each {
                self.check_void_op(&statement);
            }
            self.check_expression(&statement);
        }
    }

    /// An operator call, possibly parenthesized: reported at the operator.
    fn check_void_op(&mut self, node: &Node<'pr>) {
        if let Some(parentheses) = node.as_parentheses_node() {
            // The parser gem's `begin`: look at its first child.
            let first = parentheses
                .body()
                .and_then(|body| match body.as_statements_node() {
                    Some(list) => list.body().iter().next(),
                    None => Some(body),
                });
            if let Some(first) = first {
                self.check_void_op(&first);
            }
            return;
        }
        let Some(call) = node.as_call_node() else {
            return;
        };
        let name = call.name().as_slice();
        let unary = UNARY_OPERATORS.contains(&name);
        if !unary && !BINARY_OPERATORS.contains(&name) {
            return;
        }
        if call
            .block()
            .is_some_and(|block| block.as_block_node().is_some())
        {
            return;
        }
        // `a.+` without arguments reads as a method call.
        if !unary && call.call_operator_loc().is_some() && call.arguments().is_none() {
            return;
        }
        let at = call
            .message_loc()
            .map_or(call.location().start_offset(), |loc| loc.start_offset());
        let correction = self
            .corrections
            .then(|| self.void_op_correction(&call))
            .flatten();
        self.report(
            at,
            at + call.message_loc().map_or(1, |loc| loc.as_slice().len()),
            format!(
                "Operator `{}` used in void context.",
                String::from_utf8_lossy(name)
            ),
            correction,
        );
    }

    /// RuboCop's `autocorrect_void_op`: a unary operation becomes its
    /// operand; a binary one loses its dot, and its operator with the blanks
    /// around it becomes a line break.
    fn void_op_correction(&self, call: &CallNode<'pr>) -> Option<Correction> {
        let bytes = &self.file.bytes;
        if call.arguments().is_none() {
            let receiver = call.receiver()?;
            return Some(Correction {
                range: call.location().start_offset()..call.location().end_offset(),
                replacement: std::str::from_utf8(receiver.location().as_slice())
                    .ok()?
                    .into(),
            });
        }
        let selector = call.message_loc()?;
        let mut start = selector.start_offset();
        while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        let mut end = selector.end_offset();
        while matches!(bytes.get(end), Some(b' ' | b'\t')) {
            end += 1;
        }
        let mut edits = vec![Correction {
            range: start..end,
            replacement: "\n".into(),
        }];
        if let Some(dot) = call.call_operator_loc() {
            edits.push(Correction {
                range: dot.start_offset()..dot.end_offset(),
                replacement: String::new(),
            });
        }
        crate::cops::style::correction::combine(self.file, edits)
    }

    /// RuboCop's `check_expression`: conditionals are looked into.
    fn check_expression(&mut self, node: &Node<'pr>) {
        self.check_branch_expression(node, false);
    }

    /// `branch`: `node` is a branch of a conditional, whose expression
    /// RuboCop does not remove.
    fn check_branch_expression(&mut self, node: &Node<'pr>, branch: bool) {
        match node {
            Node::IfNode { .. } => {
                if let Some(statement) = node.as_if_node().unwrap().statements().and_then(lone) {
                    self.check_void_expression_nodes(&statement, true);
                }
            }
            Node::UnlessNode { .. } => {
                if let Some(statement) = node.as_unless_node().unwrap().statements().and_then(lone)
                {
                    self.check_void_expression_nodes(&statement, true);
                }
            }
            Node::CaseNode { .. } => {
                let case = node.as_case_node().unwrap();
                for branch in case.conditions().iter() {
                    if let Some(statement) = branch
                        .as_when_node()
                        .and_then(|when| when.statements())
                        .and_then(lone)
                    {
                        self.check_branch_expression(&statement, true);
                    }
                }
                self.check_else(case.else_clause());
            }
            Node::CaseMatchNode { .. } => {
                let case = node.as_case_match_node().unwrap();
                for branch in case.conditions().iter() {
                    if let Some(statement) = branch
                        .as_in_node()
                        .and_then(|pattern| pattern.statements())
                        .and_then(lone)
                    {
                        self.check_branch_expression(&statement, true);
                    }
                }
                self.check_else(case.else_clause());
            }
            _ => self.check_void_expression_nodes(node, branch),
        }
    }

    fn check_else(&mut self, else_clause: Option<ruby_prism::ElseNode<'pr>>) {
        if let Some(statement) = else_clause
            .and_then(|clause| clause.statements())
            .and_then(lone)
        {
            self.check_branch_expression(&statement, true);
        }
    }

    /// Literals, variables, constants, `self`, `defined?`, lambdas, procs.
    fn check_void_expression_nodes(&mut self, node: &Node<'pr>, branch: bool) {
        // RuboCop's `autocorrect_void_expression`: not a constant (loading
        // it can have effects), a conditional's branch or in a setter; the
        // expression goes with the blanks and line breaks before it.
        let correction = (self.corrections
            && !branch
            && !(is_const(node) && node.as_source_encoding_node().is_none())
            && self.setters.last() != Some(&true))
        .then(|| {
            let mut end = node.location().end_offset();
            let heredoc = match node {
                Node::StringNode { .. } => node.as_string_node().unwrap().opening_loc(),
                Node::InterpolatedStringNode { .. } => {
                    node.as_interpolated_string_node().unwrap().opening_loc()
                }
                _ => None,
            }
            .is_some_and(|o| o.as_slice().starts_with(b"<<"));
            if heredoc {
                let mut bodies = Vec::new();
                crate::cops::style::file_write::heredoc_bodies(node, &mut bodies);
                if let Some(body) = bodies.first() {
                    end = end.max(body.end);
                }
            }
            let bytes = &self.file.bytes;
            let mut start = node.location().start_offset();
            while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
                start -= 1;
            }
            while start > 0 && bytes[start - 1] == b'\n' {
                start -= 1;
            }
            Correction {
                range: start..end,
                replacement: String::new(),
            }
        });
        // Classified first: the source is only copied for a report.
        let kind = if is_entirely_literal(node)
            && !matches!(
                node,
                Node::XStringNode { .. }
                    | Node::InterpolatedXStringNode { .. }
                    | Node::RangeNode { .. }
                    | Node::NilNode { .. }
            ) {
            "Literal "
        } else if node.as_source_encoding_node().is_some() || is_variable(node) {
            "Variable "
        } else if is_const(node) {
            "Constant "
        } else if node.as_self_node().is_some() {
            self.report(
                node.location().start_offset(),
                node.location().end_offset(),
                "`self` used in void context.".to_string(),
                correction,
            );
            return;
        } else if node.as_defined_node().is_some() || is_lambda_or_proc(node) {
            ""
        } else {
            return;
        };
        let source = quoted_source(node, self.file);
        self.report(
            node.location().start_offset(),
            node.location().end_offset(),
            format!("{kind}`{source}` used in void context."),
            correction,
        );
    }

    fn report(
        &mut self,
        offset: usize,
        end: usize,
        message: String,
        correction: Option<Correction>,
    ) {
        if !self.reported.insert((offset, message.clone())) {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(offset, self.file);
        self.out.push(Offense {
            correction,
            cop: self.cop.name(),
            severity: self.cop.severity(),
            message,
            line,
            end_line: self.file.line_of(end.saturating_sub(1).max(offset)),
            col,
        });
    }
}

/// RuboCop quotes the complete source range, not a first-line preview.
fn quoted_source(node: &Node<'_>, file: &SourceFile) -> String {
    file.match_pattern_source(node.location().as_slice())
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().replace("\r\n", "\n");
            for byte in chunk.invalid() {
                text.push_str(&format!("\\x{byte:02X}"));
            }
            text
        })
        .collect()
}

/// The statement of a one-statement body: the parser gem's branch node. A
/// longer body is a `begin`, which none of the checks match.
fn lone<'pr>(statements: StatementsNode<'pr>) -> Option<Node<'pr>> {
    let mut body = statements.body().iter();
    let first = body.next()?;
    body.next().is_none().then_some(first)
}

/// RuboCop's `entirely_literal?`.
fn is_entirely_literal(node: &Node<'_>) -> bool {
    match node {
        Node::ArrayNode { .. } => node
            .as_array_node()
            .unwrap()
            .elements()
            .iter()
            .all(|element| is_entirely_literal(&element)),
        Node::HashNode { .. } => pairs_entirely_literal(node.as_hash_node().unwrap().elements()),
        Node::KeywordHashNode { .. } => {
            pairs_entirely_literal(node.as_keyword_hash_node().unwrap().elements())
        }
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            call.name().as_slice() == b"freeze"
                && call
                    .block()
                    .is_none_or(|block| block.as_block_node().is_none())
                && call
                    .receiver()
                    .is_some_and(|receiver| is_entirely_literal(&receiver))
        }
        _ => is_literal(node),
    }
}

/// Keys and values of a hash's pairs (double splats are not pairs).
fn pairs_entirely_literal(elements: ruby_prism::NodeList<'_>) -> bool {
    elements.iter().all(|element| {
        element.as_assoc_node().is_none_or(|pair| {
            is_entirely_literal(&pair.key()) && is_entirely_literal(&pair.value())
        })
    })
}

/// RuboCop's `lambda_or_proc?`: `-> {}`, `lambda {}` (any receiver), and
/// `proc {}`, `Proc.new {}` with explicit or no parameters, or `Proc.new`.
fn is_lambda_or_proc(node: &Node<'_>) -> bool {
    if node.as_lambda_node().is_some() {
        return true;
    }
    let Some(call) = node.as_call_node() else {
        return false;
    };
    let name = call.name().as_slice();
    let block = call.block().and_then(|block| block.as_block_node());
    if name == b"lambda" && block.is_some() {
        return true;
    }
    if call.arguments().is_some() {
        return false;
    }
    // `proc?` matches `send` only: never `Proc&.new`.
    let proc_new = name == b"new"
        && !call.is_safe_navigation()
        && call
            .receiver()
            .is_some_and(|receiver| is_global_proc(&receiver));
    match (call.block(), block) {
        // `Proc.new` without a block.
        (None, _) => proc_new,
        (Some(_), Some(block)) => {
            let explicit = block
                .parameters()
                .is_none_or(|parameters| parameters.as_block_parameters_node().is_some());
            explicit && (proc_new || (name == b"proc" && call.receiver().is_none()))
        }
        // A block argument (`&blk`).
        (Some(_), None) => false,
    }
}

/// `Proc` or `::Proc`.
fn is_global_proc(node: &Node<'_>) -> bool {
    match node {
        Node::ConstantReadNode { .. } => {
            node.as_constant_read_node().unwrap().name().as_slice() == b"Proc"
        }
        Node::ConstantPathNode { .. } => {
            let path = node.as_constant_path_node().unwrap();
            path.parent().is_none() && path.name().is_some_and(|name| name.as_slice() == b"Proc")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/void_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The
    //! `CheckForMethodsWithNoSideEffects: true` offense examples do not apply
    //! (rustocop fixes the default, `false`); its no-offense examples are
    //! kept.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let result = ruby_prism::parse(source.as_bytes());
        assert!(
            result.errors().next().is_none(),
            "fixture must parse: {source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/Void")
            .collect()
    }

    /// (line, col, message) triples.
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn op(line: u32, col: u32, op: &str) -> (u32, u32, String) {
        (line, col, format!("Operator `{op}` used in void context."))
    }

    fn var(line: u32, col: u32, var: &str) -> (u32, u32, String) {
        (line, col, format!("Variable `{var}` used in void context."))
    }

    fn constant(line: u32, col: u32, name: &str) -> (u32, u32, String) {
        (
            line,
            col,
            format!("Constant `{name}` used in void context."),
        )
    }

    fn lit(line: u32, col: u32, lit: &str) -> (u32, u32, String) {
        (line, col, format!("Literal `{lit}` used in void context."))
    }

    fn expr(line: u32, col: u32, expression: &str) -> (u32, u32, String) {
        (line, col, format!("`{expression}` used in void context."))
    }

    fn assert_accepts(source: &str) {
        assert_eq!(found(source), vec![], "should not flag {source:?}");
    }

    const BINARY_OPERATORS: [&str; 13] = [
        "*", "/", "%", "+", "-", "==", "===", "!=", "<", ">", "<=", ">=", "<=>",
    ];

    #[test]
    fn flags_binary_operators_unless_last() {
        for o in BINARY_OPERATORS {
            assert_eq!(
                found(&format!("a {o} b\na {o} b\na {o} b\n")),
                vec![op(1, 3, o), op(2, 3, o)],
                "for {o}"
            );
            assert_accepts(&format!("something\na {o} b\n"));
            assert_accepts(&format!("a {o} b\n"));
            assert_accepts(&format!("a.{o}\nsomething\n"));
            assert_accepts(&format!("a&.{o}\nsomething\n"));
        }
    }

    #[test]
    fn flags_parenthesized_binary_operators_unless_last() {
        for o in BINARY_OPERATORS {
            assert_eq!(
                found(&format!("(a {o} b)\n((a {o} b))\n(((a {o} b)))\n")),
                vec![op(1, 4, o), op(2, 5, o)],
                "for {o}"
            );
            assert_accepts(&format!("something\n(a {o} b)\n"));
            assert_accepts(&format!("(a {o} b)\n"));
        }
    }

    #[test]
    fn flags_binary_operators_called_with_a_dot() {
        for o in BINARY_OPERATORS {
            assert_eq!(found(&format!("a.{o}(b)\nnil\n")), vec![op(1, 3, o)]);
            assert_eq!(found(&format!("a&.{o}(b)\nnil\n")), vec![op(1, 4, o)]);
        }
    }

    #[test]
    fn flags_unary_operators_unless_last() {
        for (o, name) in [("+", "+@"), ("-", "-@"), ("~", "~"), ("!", "!")] {
            assert_eq!(
                found(&format!("{o}b\n{o}b\n{o}b\n")),
                vec![op(1, 1, name), op(2, 1, name)],
                "for {o}"
            );
            assert_accepts(&format!("something\n{o}b\n"));
            assert_accepts(&format!("{o}b\n"));
        }
        for o in ["~", "!"] {
            assert_eq!(found(&format!("b.{o}\nb.{o}\n")), vec![op(1, 3, o)]);
            assert_eq!(found(&format!("b&.{o}\nb&.{o}\n")), vec![op(1, 4, o)]);
        }
        assert_eq!(found("not b\nc\n"), vec![op(1, 1, "!")]);
        assert_eq!(found("a.-@\nb\n"), vec![op(1, 3, "-@")]);
    }

    #[test]
    fn flags_variables_unless_last() {
        for v in ["var", "@var", "@@var", "$var"] {
            assert_eq!(
                found(&format!("{v} = 5\n{v}\ntop\n")),
                vec![var(2, 1, v)],
                "for {v}"
            );
            assert_eq!(
                found(&format!("{v} = 5\n{v} unless condition\ntop\n")),
                vec![var(2, 1, v)]
            );
            assert_eq!(
                found(&format!("{v} = 5\nunless condition\n  {v}\nend\ntop\n")),
                vec![var(3, 3, v)]
            );
            assert_eq!(
                found(&format!("{v} = 5\ncondition ? {v} : nil\ntop\n")),
                vec![var(2, 13, v)]
            );
        }
    }

    #[test]
    fn flags_constants_unless_last() {
        assert_eq!(
            found("CONST = 5\nCONST\ntop\n"),
            vec![constant(2, 1, "CONST")]
        );
        assert_eq!(
            found("Foo::Bar\n\nclass Foo::Bar\nend\n"),
            vec![constant(1, 1, "Foo::Bar")]
        );
        assert_eq!(
            found("CONST = 5\nCONST unless condition\ntop\n"),
            vec![constant(2, 1, "CONST")]
        );
        assert_eq!(
            found("CONST = 5\nunless condition\n  CONST\nend\ntop\n"),
            vec![constant(3, 3, "CONST")]
        );
        assert_eq!(
            found("CONST = 5\ncondition ? CONST : nil\ntop\n"),
            vec![constant(2, 13, "CONST")]
        );
        assert_eq!(found("::Foo\ntop\n"), vec![constant(1, 1, "::Foo")]);
    }

    #[test]
    fn flags_literals_unless_last() {
        for l in ["1", "2.0", ":test", "/test/", "[1]", "{}"] {
            assert_eq!(found(&format!("{l}\ntop\n")), vec![lit(1, 1, l)], "for {l}");
            assert_eq!(
                found(&format!("{l} unless condition\ntop\n")),
                vec![lit(1, 1, l)]
            );
            assert_eq!(
                found(&format!("unless condition\n  {l}\nend\ntop\n")),
                vec![lit(2, 3, l)]
            );
            assert_eq!(
                found(&format!("condition ? {l} : nil\ntop\n")),
                vec![lit(1, 13, l)]
            );
        }
    }

    #[test]
    fn flags_void_heredocs_at_their_opening() {
        assert_eq!(
            found("<<~CSS\n  /* comment */\nCSS\ntop\n"),
            vec![lit(1, 1, "<<~CSS")]
        );
        assert_eq!(
            found("def foo\n  <<-CSS\n  /* comment */\n  CSS\n  1\nend\n"),
            vec![lit(2, 3, "<<-CSS")]
        );
    }

    #[test]
    fn flags_self_and_defined() {
        assert_eq!(
            found("self; top\n"),
            vec![(1, 1, "`self` used in void context.".to_string())]
        );
        assert_eq!(found("defined?(x)\ntop\n"), vec![expr(1, 1, "defined?(x)")]);
    }

    #[test]
    fn flags_lambdas_and_procs_unless_last() {
        for l in [
            "-> { bar }",
            "lambda { bar }",
            "proc { bar }",
            "Proc.new { bar }",
        ] {
            assert_eq!(
                found(&format!("def foo\n  {l}\n  top\nend\n")),
                vec![expr(2, 3, l)],
                "for {l}"
            );
            assert_accepts(&format!("def foo\n  top\n  {l}\nend\n"));
            assert_accepts(&format!("def foo\n  {l}.call\n  top\nend\n"));
        }
    }

    #[test]
    fn quotes_complete_multi_line_sources() {
        assert_eq!(
            found("def foo\n  lambda do |x|\n    x\n  end\n  top\nend\n"),
            vec![expr(2, 3, "lambda do |x|\n    x\n  end")]
        );
        assert_eq!(
            found("[\n  1,\n  2\n]\nx\n"),
            vec![lit(1, 1, "[\n  1,\n  2\n]")]
        );
    }

    #[test]
    fn lambda_and_proc_shapes_follow_rubocop() {
        assert_eq!(
            found("foo.lambda { }\nx\n"),
            vec![expr(1, 1, "foo.lambda { }")]
        );
        assert_eq!(
            found("lambda { _1 }\nx\n"),
            vec![expr(1, 1, "lambda { _1 }")]
        );
        assert_eq!(found("Proc.new\nx\n"), vec![expr(1, 1, "Proc.new")]);
        assert_eq!(
            found("::Proc.new { }\nx\n"),
            vec![expr(1, 1, "::Proc.new { }")]
        );
        // RuboCop's `proc?` only matches blocks with explicit parameters.
        assert_accepts("proc { _1 }\nx\n");
        assert_accepts("proc { it }\nx\n");
        assert_accepts("lambda(&blk)\nx\n");
        assert_accepts("Proc.new(&blk)\nx\n");
        assert_accepts("foo.proc { }\nx\n");
        // `proc?` matches `send` only, never safe navigation.
        assert_accepts("Proc&.new\nx\n");
        assert_accepts("Proc&.new { }\nx\n");
        assert_accepts("::Proc&.new { |a| }\nx\n");
    }

    #[test]
    fn ignores_nonmutating_methods() {
        // `CheckForMethodsWithNoSideEffects` is off.
        assert_accepts("x.sort\ntop(x)\n");
        assert_accepts("[1,2,3].collect do |n|\n  n.to_s\nend\n\"done\"\n");
        assert_accepts("foo = bar\nbaz\n");
        assert_accepts("def merge\nend\n\n42\n");
    }

    #[test]
    fn checks_collections_entirely_of_literals() {
        for source in [
            "def something\n  [foo, bar]\n  baz\nend\n",
            "def something\n  [1, 2, [foo, bar]]\n  baz\nend\n",
            "def something\n  foo.freeze\n  baz\nend\n",
            "def something\n  [1, [foo.freeze]]\n  baz\nend\n",
            "def something\n  {k1: foo, k2: bar}\n  baz\nend\n",
            "def something\n  {k0: {k1: foo, k2: bar}}\n  baz\nend\n",
            "def something\n  {foo => 1, bar => 2}\n  baz\nend\n",
            "def something\n  {foo: 1, bar: {baz => 2}}\n  baz\nend\n",
            "def something\n  {[foo, bar] => :foo}\n  baz\nend\n",
        ] {
            assert_accepts(source);
        }
        for l in [
            "[1, 2]",
            "[1, 2, [3]]",
            "'foo'.freeze",
            "'foo'&.freeze",
            "[1, ['foo'.freeze]]",
            "{k1: 1, k2: 2}",
            "{x: {k1: 1, k2: 2}}",
            "{[1, 2] => :foo}",
        ] {
            assert_eq!(
                found(&format!("def something\n  {l}\n  baz\nend\n")),
                vec![lit(2, 3, l)],
                "for {l}"
            );
        }
    }

    #[test]
    fn literal_shapes_follow_rubocop() {
        assert_eq!(found("{**foo}\nx\n"), vec![lit(1, 1, "{**foo}")]);
        assert_eq!(found("\"#{foo}\"\nx\n"), vec![lit(1, 1, "\"#{foo}\"")]);
        assert_eq!(found("__FILE__\nx\n"), vec![lit(1, 1, "__FILE__")]);
        assert_eq!(found("%w[a b]\nx\n"), vec![lit(1, 1, "%w[a b]")]);
        assert_eq!(
            found("true; false; x\n"),
            vec![lit(1, 1, "true"), lit(1, 7, "false")]
        );
        for source in [
            "nil\nx\n",
            "1..2\nx\n",
            "`ls`\nx\n",
            "[*a]\nx\n",
            "'a'.freeze { }\nx\n",
            "$1\nx\n",
        ] {
            assert_accepts(source);
        }
    }

    #[test]
    fn checks_every_statement_of_initialize() {
        assert_eq!(
            found("def something\n  42\n  42\nend\n"),
            vec![lit(2, 3, "42")]
        );
        assert_eq!(
            found("def initialize\n  42\n  42\nend\n"),
            vec![lit(2, 3, "42"), lit(3, 3, "42")]
        );
        // A lone statement is no `begin` for RuboCop; parentheses are.
        assert_accepts("def initialize\n  42\nend\n");
        assert_eq!(
            found("def initialize\n  (42)\nend\n"),
            vec![lit(2, 4, "42")]
        );
        assert_eq!(
            found("def initialize\n  begin\n    1\n    2\n  end\nend\n"),
            vec![lit(3, 5, "1"), lit(4, 5, "2")]
        );
        // With `rescue`, the body is not the method's last child.
        assert_eq!(
            found("def initialize\n  1\n  2\nrescue\n  3\nend\n"),
            vec![lit(2, 3, "1")]
        );
        assert_eq!(
            found("def self.initialize\n  1\n  2\nend\n"),
            vec![lit(2, 3, "1")]
        );
    }

    #[test]
    fn skips_the_value_of_setter_methods() {
        assert_eq!(
            found("def foo=(rhs)\n  42\n  42\nend\n"),
            vec![lit(2, 3, "42")]
        );
        assert_eq!(
            found("def self.foo=(rhs)\n  42\n  42\nend\n"),
            vec![lit(2, 3, "42")]
        );
        assert_accepts("def name=(name)\n  @name = name\n  reset\n  name\nend\n");
    }

    #[test]
    fn each_blocks_keep_their_last_value() {
        assert_eq!(
            found("array.each do |_item|\n  42\n  42\nend\n"),
            vec![lit(2, 3, "42")]
        );
        for source in [
            "array.each do |_item|\n  CONST\nend\n",
            "array.each do |_item|\n  42\nend\n",
            "enumerator_as_filter.each do |item|\n  # comment\n  item == 42\nend\n",
            "enumerator_as_filter.each do |item|\n  puts item\n\n  item == 42\nend\n",
            "enumerator_as_filter.each do\n  puts _1\n\n  _1 == 42\nend\n",
            "array.each { |_item| }\n",
            // Operators are never reported inside an `each` block.
            "array.each do |item|\n  item == 1\n  item == 2\nend\n",
            "array.each do |item|\n  if a\n    item == 1\n    b\n  end\nend\n",
        ] {
            assert_accepts(source);
        }
    }

    #[test]
    fn tap_blocks_check_every_statement() {
        assert_eq!(
            found("foo.tap do\n  _1\n  42\nend\n"),
            vec![var(2, 3, "_1"), lit(3, 3, "42")]
        );
        assert_eq!(
            found("foo.tap do |x|\n  42\n  42\nend\n"),
            vec![lit(2, 3, "42"), lit(3, 3, "42")]
        );
        assert_eq!(found("foo.tap { |x| x }\n"), vec![var(1, 15, "x")]);
        assert_eq!(found("foo.tap { a + b }\n"), vec![op(1, 13, "+")]);
        assert_eq!(found("foo.tap { (a + b) }\n"), vec![op(1, 14, "+")]);
    }

    #[test]
    fn for_and_ensure_bodies_check_every_statement() {
        assert_eq!(
            found("for _item in array do\n  42\n  42\nend\n"),
            vec![lit(2, 3, "42"), lit(3, 3, "42")]
        );
        assert_eq!(
            found("def foo\nensure\n  bar\n  42\n  42\nend\n"),
            vec![lit(4, 3, "42"), lit(5, 3, "42")]
        );
        assert_eq!(
            found("def foo\n  bar\nensure\n  [1, 2, [3]]\nend\n"),
            vec![lit(4, 3, "[1, 2, [3]]")]
        );
        // `on_ensure` checks a lone statement, but not as an operator.
        assert_accepts("begin\n  foo\nensure\n  a + b\nend\n");
        assert_eq!(
            found("begin\n  foo\nensure\n  (a + b)\nend\n"),
            vec![op(4, 6, "+")]
        );
        // The main body of a `begin` with `ensure` keeps its last value.
        assert_eq!(
            found("x = begin\n  1\n  2\nensure\n  3\nend\n"),
            vec![lit(2, 3, "1"), lit(5, 3, "3")]
        );
    }

    #[test]
    fn checks_explicit_begin_and_parentheses() {
        assert_eq!(found("begin\n 1\n 2\nend\n"), vec![lit(2, 2, "1")]);
        assert_eq!(found("y = (1; b)\n"), vec![lit(1, 6, "1")]);
        assert_accepts("((); 1)\n");
        // Checked as a statement and as the first of a sequence: reported once.
        assert_eq!(found("(1 + 2; 3)\n4\n"), vec![op(1, 4, "+")]);
    }

    #[test]
    fn checks_case_branches() {
        assert_eq!(
            found("case foo\nwhen 1 then 2\nend\nputs 3\n"),
            vec![lit(2, 13, "2")]
        );
        assert_eq!(
            found("case foo\nwhen 1 then 2\nwhen 3 then 4\nelse 5\nend\nputs 6\n"),
            vec![lit(2, 13, "2"), lit(3, 13, "4"), lit(4, 6, "5")]
        );
        assert_eq!(
            found("x = 1\ncase foo\nwhen 1 then x\nend\nputs 3\n"),
            vec![var(3, 13, "x")]
        );
        assert_eq!(
            found("case foo\nin 1 then 2\nin 2 then 3\nelse 4\nend\nputs 5\n"),
            vec![lit(2, 11, "2"), lit(3, 11, "3"), lit(4, 6, "4")]
        );
        assert_eq!(
            found("x = 1\ncase foo\nin 1 then x\nend\nputs 3\n"),
            vec![var(3, 11, "x")]
        );
        // Nested conditionals in a branch are followed.
        assert_eq!(
            found("case foo\nwhen 1\n  2 if a\nend\nputs 3\n"),
            vec![lit(3, 3, "2")]
        );
        for source in [
            "case foo\nwhen 1 then do_something\nend\nputs 3\n",
            "case foo\nwhen 1\n  nil\nend\nputs 3\n",
            "case foo\nwhen 1 then 2\nend\n",
            "case foo\nwhen 1\nend\nputs :ok\n",
            "case foo\nin 1 then do_something\nend\nputs 3\n",
            "case foo\nin 1 then 2\nend\n",
            "case foo\nin 1\nend\nputs :ok\n",
        ] {
            assert_accepts(source);
        }
    }

    #[test]
    fn checks_only_the_keyword_branch_of_conditionals() {
        assert_eq!(
            found("if a\n  1\nelse\n  2\nend\nx\n"),
            vec![lit(2, 3, "1")]
        );
        assert_eq!(
            found("if a\n  1\nelsif b\n  2\nend\nx\n"),
            vec![lit(2, 3, "1")]
        );
        assert_accepts("if some_condition\nend\n\nputs :ok\n");
        // A longer branch is a sequence of its own.
        assert_eq!(found("if a\n  1\n  b\nend\nx\n"), vec![lit(2, 3, "1")]);
        assert_eq!(
            found("case foo\nwhen 1\n  2\n  b\nend\nputs 3\n"),
            vec![lit(3, 3, "2")]
        );
    }

    #[test]
    fn accepts_values_in_use() {
        for source in [
            "lambda.(a)\ntop\n",
            "`touch x`\nnil\n",
            "x = if condition\n      42\n    end\nnil\n",
            "x = (42 if condition)\nnil\n",
            "x = condition ? 42 : nil\nnil\n",
            "%x(touch x)\nnil\n",
            "def foo\n  1..100.times.each { puts 1 }\n  do_something\nend\n",
            "def foo\n  1...100.times.each { puts 1 }\n  do_something\nend\n",
            "foo(1 + 2)\nbar\n",
            "-> { 1 }\n",
            "x = [1, 2]\ny\n",
        ] {
            assert_accepts(source);
        }
    }

    #[test]
    fn flags_special_encoding_as_a_variable() {
        assert_eq!(
            found("def foo\n  __ENCODING__\n  42\nend\n"),
            vec![var(2, 3, "__ENCODING__")]
        );
    }

    #[test]
    fn checks_nested_sequences() {
        assert_eq!(
            found("class A\n  1\n  def m\n    2\n    3\n  end\nend\n"),
            vec![lit(2, 3, "1"), lit(4, 5, "2")]
        );
        assert_eq!(
            found("foo.map do |x|\n  x\n  1\nend\n"),
            vec![var(2, 3, "x")]
        );
        assert_eq!(found("\"#{1; a}\"\n"), vec![lit(1, 4, "1")]);
        assert_eq!(found("super do |x|\n  x\n  1\nend\n"), vec![var(2, 3, "x")]);
        assert_eq!(found("-> do\n  1\n  2\nend\n"), vec![lit(2, 3, "1")]);
    }
}
