//! `Style/SafeNavigation` — guarded receiver calls.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense, space_right},
    source::SourceFile,
};
use ruby_prism::{AndNode, CallNode, Node, StatementsNode};
use std::{collections::HashSet, ops::Range};

const MSG: &str =
    "Use safe navigation (`&.`) instead of checking if an object exists before calling the method.";

// Test-only counters prove clause/receiver work stays bounded on wide sources.
#[cfg(test)]
thread_local! {
    static STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct SafeNavigation;
impl Cop for SafeNavigation {
    fn name(&self) -> &'static str {
        "Style/SafeNavigation"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_and_node().is_some()
            || node.as_if_node().is_some()
            || node.as_unless_node().is_some()
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_and_node().is_some() {
            // Upstream's on_and runs for every conjunction, but one sees the
            // clauses of the conjunctions below it: visit them all from the
            // outermost one, sharing the reported ranges.
            if !cx.ancestors().any(|a| a.as_and_node().is_some()) {
                self.check_conjunctions(node, cx, file, out);
            }
            return;
        }
        let Some((checked, body, ternary)) = if_parts(node) else {
            return;
        };
        let Some(calls) = candidate_call(&checked, &body, ternary, file, &|| {
            unsafe_enclosing_send(node, cx.ancestors(), ternary)
        }) else {
            return;
        };
        let start = node.location().start_offset();
        let end = node.location().end_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| if_correction(file, node, &checked, &body, &calls))
                .flatten(),
        });
    }
}

impl SafeNavigation {
    fn check_conjunctions<'pr>(
        &self,
        root: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut collector = Conjunctions {
            outer: cx.ancestors().collect(),
            outer_block: block_ancestor(cx.ancestors()),
            stack: Vec::new(),
            blocks: 0,
            ands: Vec::new(),
        };
        walk(&mut collector, root);
        let ands = collector.ands;
        let mut reported = HashSet::new();
        for (index, and) in ands.iter().enumerate() {
            let mut list = and.parts.clone();
            if !and.block_above {
                for inner in ands[index + 1..]
                    .iter()
                    .take_while(|inner| inner.start < and.end)
                {
                    if !inner.block_above {
                        list.extend_from_slice(&inner.parts);
                    }
                }
            }
            list.sort_by_key(Item::start);
            let slices: Vec<&[Item]> = list.chunks(2).collect();
            let mut ignored = false;
            for pair in slices.windows(2) {
                let (Item::Clause(left), Some(Item::Op(op)), Item::Clause(right)) =
                    (&pair[0][0], pair[0].get(1), &pair[1][0])
                else {
                    continue;
                };
                let lhs = left.node(&ands);
                let rhs = right.node(&ands);
                // ConvertCodeThatCanStartToReturnNil is false.
                if not_nil_check(&lhs) {
                    continue;
                }
                let Some(body) = strip_begin(right.node(&ands)) else {
                    continue;
                };
                let enclosing = ands[right.and].unsafe_enclosing;
                let Some(calls) = candidate_call(&lhs, &body, false, file, &|| enclosing) else {
                    continue;
                };
                let start = lhs.location().start_offset();
                let end = rhs.location().end_offset();
                if reported.insert((start, end)) {
                    let (line, col) = file.line_col(start);
                    let correct =
                        cx.corrections_requested() && !ignored && !and_with_rhs_or(&and.node);
                    out.push(Offense {
                        cop: self.name(),
                        severity: self.severity(),
                        message: MSG.into(),
                        line,
                        col,
                        end_line: file.line_of(end.saturating_sub(1)),
                        correction: correct
                            .then(|| {
                                and_correction(file, &and.node, &lhs, op.clone(), &rhs, &calls)
                            })
                            .flatten(),
                    });
                }
                ignored = true;
            }
        }
    }
}

/// An entry of upstream's `collect_and_clauses` list: a clause or an `&&`
/// operator.
#[derive(Clone)]
enum Item {
    Clause(Clause),
    Op(Range<usize>),
}
impl Item {
    fn start(&self) -> usize {
        match self {
            Item::Clause(clause) => clause.start,
            Item::Op(range) => range.start,
        }
    }
}
/// A side of one of the collected conjunctions.
#[derive(Clone)]
struct Clause {
    and: usize,
    right: bool,
    start: usize,
}
impl Clause {
    fn node<'pr>(&self, ands: &[Conjunction<'pr>]) -> Node<'pr> {
        let and = &ands[self.and].node;
        if self.right { and.right() } else { and.left() }
    }
}
struct Conjunction<'pr> {
    node: AndNode<'pr>,
    start: usize,
    end: usize,
    /// A `block` ancestor (upstream's `each_ancestor(:block)`), anywhere up
    /// to the root: its clauses are then left to its own visit.
    block_above: bool,
    /// Whether an enclosing send makes a call guarded here unsafe, for the
    /// RHS shapes whose upstream ancestor scan runs past the clause.
    unsafe_enclosing: bool,
    /// `and_parts`.
    parts: Vec<Item>,
}
struct Conjunctions<'a, 'pr> {
    outer: Vec<&'a Node<'pr>>,
    outer_block: bool,
    stack: Vec<Node<'pr>>,
    /// The `block` nodes (in parser's tree) on the stack.
    blocks: usize,
    ands: Vec<Conjunction<'pr>>,
}
impl<'pr> NodeVisitor<'pr> for Conjunctions<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(and) = node.as_and_node() {
            let index = self.ands.len();
            let ancestors = self.stack.iter().rev().chain(self.outer.iter().copied());
            let mut parts = vec![Item::Op(range(&and.operator_loc()))];
            let (lhs, rhs) = (and.left(), and.right());
            if !and_inside_begin(&rhs) {
                parts.push(Item::Clause(Clause {
                    and: index,
                    right: true,
                    start: rhs.location().start_offset(),
                }));
            }
            if lhs.as_and_node().is_none() && !and_inside_begin(&lhs) {
                parts.push(Item::Clause(Clause {
                    and: index,
                    right: false,
                    start: lhs.location().start_offset(),
                }));
            }
            self.ands.push(Conjunction {
                start: node.location().start_offset(),
                end: node.location().end_offset(),
                block_above: self.outer_block || self.blocks > 0,
                unsafe_enclosing: unsafe_enclosing_send(&node, ancestors, false),
                parts,
                node: and,
            });
        }
        self.blocks += usize::from(parser_block(&node));
        self.stack.push(node);
    }
    fn leave(&mut self) {
        if let Some(node) = self.stack.pop() {
            self.blocks -= usize::from(parser_block(&node));
        }
    }
}

fn range(location: &ruby_prism::Location<'_>) -> Range<usize> {
    location.start_offset()..location.end_offset()
}

/// Whether `node` is or holds (as parser nests it) a `block` node: a
/// literal block with ordinary parameters (numbered and `it` parameters make
/// `numblock`/`itblock`), wrapping its call, or a lambda.
fn parser_block(node: &Node<'_>) -> bool {
    let ordinary = |parameters: Option<Node<'_>>| {
        parameters.is_none_or(|p| {
            p.as_numbered_parameters_node().is_none() && p.as_it_parameters_node().is_none()
        })
    };
    let literal = |block: Option<Node<'_>>| {
        block
            .and_then(|b| b.as_block_node())
            .is_some_and(|b| ordinary(b.parameters()))
    };
    if let Some(block) = node.as_block_node() {
        ordinary(block.parameters())
    } else if let Some(lambda) = node.as_lambda_node() {
        ordinary(lambda.parameters())
    } else if let Some(call) = node.as_call_node() {
        literal(call.block())
    } else if let Some(call) = node.as_super_node() {
        literal(call.block())
    } else if let Some(call) = node.as_forwarding_super_node() {
        literal(call.block().map(|b| b.as_node()))
    } else {
        false
    }
}
fn block_ancestor<'a, 'pr: 'a>(mut ancestors: impl Iterator<Item = &'a Node<'pr>>) -> bool {
    ancestors.any(parser_block)
}

/// `` `(begin and ...) ``: the node or a descendant is a parser `begin`
/// (parentheses, interpolation, or a body of several statements) whose first
/// child is a conjunction.
fn and_inside_begin(node: &Node<'_>) -> bool {
    struct Search<'pr> {
        stack: Vec<Node<'pr>>,
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Search<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            if !self.found
                && let Some(statements) = node.as_statements_node()
            {
                let body = statements.body();
                let begin = match self.stack.last() {
                    Some(p)
                        if p.as_parentheses_node().is_some()
                            || p.as_embedded_statements_node().is_some() =>
                    {
                        true
                    }
                    Some(p)
                        if p.as_begin_node().is_some_and(|b| {
                            b.rescue_clause().is_none()
                                && b.else_clause().is_none()
                                && b.ensure_clause().is_none()
                        }) =>
                    {
                        false
                    }
                    _ => body.len() > 1,
                };
                self.found = begin
                    && body
                        .iter()
                        .next()
                        .is_some_and(|n| n.as_and_node().is_some());
            }
            self.stack.push(node);
        }
        fn leave(&mut self) {
            self.stack.pop();
        }
    }
    let mut search = Search {
        stack: Vec::new(),
        found: false,
    };
    walk(&mut search, node);
    search.found
}

/// `(and _ {or (begin or)})`.
fn and_with_rhs_or(and: &AndNode<'_>) -> bool {
    let rhs = and.right();
    rhs.as_or_node().is_some()
        || rhs.as_parentheses_node().is_some_and(|p| {
            p.body()
                .and_then(|b| b.as_statements_node())
                .is_some_and(|s| {
                    s.body().len() == 1 && s.body().iter().all(|n| n.as_or_node().is_some())
                })
        })
}

/// `(send (send $_ :nil?) :!)`.
fn not_nil_check(node: &Node<'_>) -> bool {
    fn bare<'pr>(node: &Node<'pr>, name: &[u8]) -> Option<CallNode<'pr>> {
        node.as_call_node().filter(|call| {
            call.name().as_slice() == name
                && !call.is_safe_navigation()
                && call.arguments().is_none()
                && call.block().is_none()
        })
    }
    bare(node, b"!")
        .and_then(|call| call.receiver())
        .is_some_and(|receiver| bare(&receiver, b"nil?").is_some())
}

/// `{ (begin $!begin) $!(begin) }`.
fn strip_begin(node: Node<'_>) -> Option<Node<'_>> {
    let Some(paren) = node.as_parentheses_node() else {
        return Some(node);
    };
    single(paren.body().and_then(|n| n.as_statements_node()))
        .filter(|inner| inner.as_parentheses_node().is_none())
}

/// The `&` upstream's `add_safe_nav_to_all_methods_in_chain` inserts: before
/// the dot of each plain, non-operator call of the chain.
fn chain_edits<'a, 'pr: 'a>(
    calls: impl Iterator<Item = &'a CallNode<'pr>>,
    edits: &mut Vec<(Range<usize>, String)>,
) {
    for call in calls {
        if !call.is_safe_navigation()
            && !ast::is_operator_method(call.name().as_slice())
            && let Some(dot) = call.call_operator_loc()
        {
            edits.push((dot.start_offset()..dot.start_offset(), "&".into()));
        }
    }
}

/// `handle_comments`: the comments on the lines of `node` outside its child
/// nodes (parser's, in its order), inserted before `target`.
fn comment_edit(
    file: &SourceFile,
    node: &Node<'_>,
    children: &[&Node<'_>],
    target: usize,
    edits: &mut Vec<(Range<usize>, String)>,
) {
    let mut ranges = Vec::new();
    let mut begin = file.line_of(node.location().start_offset());
    for child in children {
        ranges.push(begin..file.line_of(child.location().start_offset()));
        begin = file.line_of(child.location().end_offset());
    }
    ranges.push(begin..file.line_of(node.location().end_offset()));
    let comments: Vec<_> = ranges
        .iter()
        .flat_map(|lines| {
            file.comments()
                .iter()
                .filter(|c| lines.contains(&file.line_of(c.start)))
                .map(|c| String::from_utf8_lossy(&file.bytes[c.clone()]).into_owned())
        })
        .collect();
    if !comments.is_empty() {
        edits.push((target..target, format!("{}\n", comments.join("\n"))));
    }
}

fn if_correction(
    file: &SourceFile,
    node: &Node<'_>,
    checked: &Node<'_>,
    body: &Node<'_>,
    calls: &[CallNode<'_>],
) -> Option<Correction> {
    let (method, outer) = calls.split_last()?;
    let receiver = method.receiver()?;
    let body_range = range(&body.location());
    let node_range = range(&node.location());
    let mut edits = vec![
        (node_range.start..body_range.start, String::new()),
        (body_range.end..node_range.end, String::new()),
    ];
    if checked
        .as_call_node()
        .is_some_and(|c| c.is_safe_navigation())
    {
        edits.push((
            range(&receiver.location()),
            String::from_utf8(checked.location().as_slice().to_vec()).ok()?,
        ));
    }
    if !method.is_safe_navigation() {
        let dot = method.call_operator_loc()?.start_offset();
        edits.push((dot..dot, "&".into()));
    }
    let predicate;
    let mut children = Vec::new();
    if let Some(n) = node.as_if_node() {
        predicate = n.predicate();
        children.push(&predicate);
    } else {
        predicate = node.as_unless_node()?.predicate();
        children.push(&predicate);
    }
    let then = node.as_if_node().map_or_else(
        || node.as_unless_node().and_then(|n| single(n.statements())),
        |n| single(n.statements()),
    );
    let otherwise = node
        .as_if_node()
        .and_then(|n| n.subsequent())
        .and_then(|n| n.as_else_node())
        .and_then(|n| single(n.statements()));
    children.extend(then.iter());
    children.extend(otherwise.iter());
    comment_edit(file, node, &children, body_range.start, &mut edits);
    chain_edits(outer.iter().rev(), &mut edits);
    Correction::from_edits(&file.bytes, edits)
}

fn and_correction(
    file: &SourceFile,
    and: &AndNode<'_>,
    lhs: &Node<'_>,
    op: Range<usize>,
    rhs: &Node<'_>,
    calls: &[CallNode<'_>],
) -> Option<Correction> {
    let receiver = calls.last()?.receiver()?;
    let lhs_range = range(&lhs.location());
    let rhs_start = rhs.location().start_offset();
    let mut edits = vec![
        (
            lhs_range.start..space_right(&file.bytes, lhs_range.end),
            String::new(),
        ),
        (op.start..space_right(&file.bytes, op.end), String::new()),
    ];
    let (left, right) = (and.left(), and.right());
    comment_edit(
        file,
        &and.as_node(),
        &[&left, &right],
        rhs_start,
        &mut edits,
    );
    edits.push((
        range(&receiver.location()),
        String::from_utf8(lhs.location().as_slice().to_vec()).ok()?,
    ));
    chain_edits(calls.iter().rev(), &mut edits);
    Correction::from_edits(&file.bytes, edits)
}

fn single(statements: Option<StatementsNode<'_>>) -> Option<Node<'_>> {
    let body = statements?.body();
    (body.len() == 1).then(|| body.iter().next().unwrap())
}
fn unary_receiver<'pr>(node: &Node<'pr>, name: &[u8]) -> Option<Node<'pr>> {
    let call = node.as_call_node()?;
    (call.name().as_slice() == name
        && ast::call_arguments(&call).is_empty()
        && call.block().is_none()
        && !call.is_safe_navigation())
    .then(|| call.receiver())
    .flatten()
}
fn if_parts<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>, bool)> {
    if let Some(n) = node.as_unless_node() {
        if n.else_clause().is_some() {
            return None;
        }
        let condition = n.predicate();
        let checked =
            unary_receiver(&condition, b"nil?").or_else(|| unary_receiver(&condition, b"!"))?;
        return Some((checked, single(n.statements())?, false));
    }
    let n = node.as_if_node()?;
    let ternary = n.if_keyword_loc().is_none();
    if n.if_keyword_loc().is_some_and(|l| l.as_slice() == b"elsif")
        || (!ternary && n.subsequent().is_some())
    {
        return None;
    }
    let condition = n.predicate();
    let yes = single(n.statements())?;
    let no = n
        .subsequent()
        .and_then(|n| n.as_else_node())
        .and_then(|n| single(n.statements()));
    if ternary && yes.as_nil_node().is_some() {
        let checked =
            unary_receiver(&condition, b"nil?").or_else(|| unary_receiver(&condition, b"!"))?;
        return Some((checked, no?, true));
    }
    if ternary && !no.is_some_and(|n| n.as_nil_node().is_some()) {
        return None;
    }
    let checked = unary_receiver(&condition, b"!")
        .and_then(|inner| unary_receiver(&inner, b"nil?"))
        .unwrap_or(condition);
    Some((checked, yes, ternary))
}
// Normalize equivalent call spellings and regexp option sets at every depth.
// Only the receiver spine may ignore csend/send differences; arguments and
// blocks retain that distinction. These fingerprints never prove fix safety.
fn matching(left: &Node<'_>, right: &Node<'_>, file: &SourceFile) -> bool {
    #[cfg(test)]
    STEPS.set(STEPS.get() + 1);
    if let (Some(a), Some(b)) = (left.as_call_node(), right.as_call_node()) {
        a.name().as_slice() == b.name().as_slice()
            && match (a.receiver(), b.receiver()) {
                (Some(a), Some(b)) => matching(&a, &b, file),
                (None, None) => true,
                _ => false,
            }
            && ast::call_arguments(&a)
                .iter()
                .map(|n| ast::fingerprint_for_detection(n, file))
                .eq(ast::call_arguments(&b)
                    .iter()
                    .map(|n| ast::fingerprint_for_detection(n, file)))
            && a.block().map(|n| ast::fingerprint_for_detection(&n, file))
                == b.block().map(|n| ast::fingerprint_for_detection(&n, file))
    } else {
        ast::fingerprint_for_detection(left, file) == ast::fingerprint_for_detection(right, file)
    }
}
fn candidate_call<'pr>(
    checked: &Node<'pr>,
    body: &Node<'pr>,
    ternary: bool,
    file: &SourceFile,
    unsafe_enclosing: &dyn Fn() -> bool,
) -> Option<Vec<CallNode<'pr>>> {
    let mut current = None;
    let mut calls = Vec::new();
    // At most two calls after the repeated receiver; stop as soon as that
    // fixed bound is exceeded, rather than walking an arbitrary chain.
    for _ in 0..2 {
        let call = current.as_ref().unwrap_or(body).as_call_node()?;
        let receiver = call.receiver()?;
        calls.push(call);
        if matching(checked, &receiver, file) {
            // A literal block wraps its send in RuboCop's AST. For ternaries,
            // the extra dotless-operator scan stops at that wrapper; ordinary
            // if/AND safety checks still examine all outer sends.
            let block_boundary = calls
                .last()
                .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()));
            let candidate = calls.iter().enumerate().all(|(index, call)| {
                let dot = call.call_operator_loc().map(|l| l.as_slice());
                (dot != Some(b"::".as_slice()) || (ternary && index + 1 != calls.len()))
                    && (dot.is_some() || (ternary && block_boundary && index + 1 != calls.len()))
                    && (call.is_safe_navigation() || call.name().as_slice() != b"!")
                    && (ternary || !call.is_attribute_write())
                    && (index == calls.len() - 1
                        || call.is_safe_navigation()
                        || !nil_method(call.name().as_slice()))
            }) && (body.as_call_node().is_none_or(|c| {
                c.is_safe_navigation()
                    || c.name().as_slice() != b"empty?"
                    || c.block().is_some_and(|b| b.as_block_node().is_some())
            })) && (!(calls.len() == 1
                || body.as_call_node().is_some_and(|c| {
                    c.is_safe_navigation() || c.block().is_some_and(|b| b.as_block_node().is_some())
                }))
                || !unsafe_enclosing());
            return candidate.then_some(calls);
        }
        current = Some(receiver);
    }
    None
}
// Upstream's ancestor-send safety loop stops when it reaches a *subsequent*
// send equal to the RHS. For a single call, csend or block-wrapped RHS it instead
// reaches enclosing sends. `ancestors` are the node's, innermost first.
fn unsafe_enclosing_send<'a, 'pr: 'a>(
    node: &'a Node<'pr>,
    ancestors: impl Iterator<Item = &'a Node<'pr>>,
    ternary: bool,
) -> bool {
    let mut child = node;
    for parent in ancestors {
        #[cfg(test)]
        STEPS.set(STEPS.get() + 1);
        if let Some(call) = parent.as_call_node()
            && !call.is_safe_navigation()
            && child.as_block_node().is_none()
            && (call.name().as_slice() == b"!"
                || nil_method(call.name().as_slice())
                || (!ternary
                    && (call.is_attribute_write()
                        || call
                            .call_operator_loc()
                            .is_none_or(|l| l.as_slice() != b"."))))
        {
            return true;
        }
        child = parent;
    }
    false
}
// Fixed Ruby 3.4 NilClass public methods, plus to_d and the pinned
// AllowedMethods. Only subsequent ordinary calls read this set; the first
// guarded call may itself be a nil method (e.g. x && x.to_i).
fn nil_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"!" | b"!="
            | b"!~"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b"^"
            | b"__id__"
            | b"__send__"
            | b"class"
            | b"clone"
            | b"define_singleton_method"
            | b"display"
            | b"dup"
            | b"enum_for"
            | b"eql?"
            | b"equal?"
            | b"extend"
            | b"freeze"
            | b"frozen?"
            | b"hash"
            | b"inspect"
            | b"instance_eval"
            | b"instance_exec"
            | b"instance_of?"
            | b"instance_variable_defined?"
            | b"instance_variable_get"
            | b"instance_variable_set"
            | b"instance_variables"
            | b"is_a?"
            | b"itself"
            | b"kind_of?"
            | b"method"
            | b"methods"
            | b"nil?"
            | b"object_id"
            | b"private_methods"
            | b"protected_methods"
            | b"public_method"
            | b"public_methods"
            | b"public_send"
            | b"rationalize"
            | b"remove_instance_variable"
            | b"respond_to?"
            | b"send"
            | b"singleton_class"
            | b"singleton_method"
            | b"singleton_methods"
            | b"tap"
            | b"then"
            | b"to_a"
            | b"to_c"
            | b"to_enum"
            | b"to_f"
            | b"to_h"
            | b"to_i"
            | b"to_r"
            | b"to_s"
            | b"yield_self"
            | b"|"
            | b"to_d"
            | b"present?"
            | b"blank?"
            | b"presence"
            | b"try"
            | b"try!"
    )
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `style/safe_navigation_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;
    fn fix(source: &str) -> String {
        crate::cops::testing::fix_until_stable(&SafeNavigation, source)
    }
    #[test]
    fn conjunctions_in_blocks_and_one_correction_per_conjunction_as_rubocop_does() {
        // Verified against RuboCop 1.92.0: under a `block`, a conjunction
        // sees only its own clauses, so `c && c.d` goes unreported there.
        assert_eq!(offenses("foo do\n  a && a.b && c && c.d\nend\n").len(), 1);
        assert_eq!(offenses("-> { a && a.b && c && c.d }").len(), 1);
        assert_eq!(offenses("items.each { _1 && _1.x && y && y.z }").len(), 2);
        assert_eq!(offenses("a && a.b && c && c.d").len(), 2);
        assert!(offenses("x && foo(a && a.b)").is_empty());
        for (source, fixed) in [
            ("a && a.b && c && c.d\n", "a&.b && c&.d\n"),
            ("a && a.b && (c || d)\n", "a && a.b && (c || d)\n"),
            ("z = (q && q.r) && (s && s.t)\n", "z = (q&.r) && (s&.t)\n"),
            (
                "foo &&\n  # comment\n  foo.bar\n",
                "  # comment\n  # comment\nfoo&.bar\n",
            ),
            (
                "foo.bar do\n  # inner\nend if foo\n",
                "# inner\nfoo&.bar do\n  # inner\nend\n",
            ),
        ] {
            assert_eq!(fix(source), fixed, "{source}");
        }
    }
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SafeNavigation")
            .collect()
    }
    #[test]
    fn every_applicable_pinned_call_and_correction_matches() {
        use crate::cops::testing::correction_offenses;
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/safe_navigation_pinned.json"
        ))
        .unwrap();
        let mut fixes = 0;
        let mut invalid_yields = 0;
        assert_eq!(cases.as_array().unwrap().len(), 707);
        for case in cases.as_array().unwrap() {
            let mut source = case["source"].as_str().unwrap().to_string();
            if case["spec_line"] == 222 && source.contains("yield ") {
                invalid_yields += 1;
                source = format!("def f\n{source}end\n");
            }
            let found = correction_offenses(&SafeNavigation, &source);
            let mut actual: Vec<_> = found.iter().map(|o| (o.line, o.col)).collect();
            let mut expected: Vec<_> = case["positions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_u64().unwrap() as u32, p[1].as_u64().unwrap() as u32))
                .collect();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected, "{case}");
            assert!(found.iter().all(|o| o.message == MSG), "{case}");
            if let Some(expected) = case["correction"].as_str() {
                fixes += 1;
                assert_eq!(fix(&source), expected, "{case}");
            }
        }
        assert_eq!((fixes, invalid_yields), (493, 6));
    }
    #[test]
    fn ruby_23_minimum_is_explicit_and_active_at_fixed_34() {
        use crate::cops::policy::{DEFAULT_RUBY_VERSION, RubyVersion};
        let versions = SafeNavigation.ruby_versions();
        assert_eq!(versions.minimum, Some(RubyVersion(2, 3)));
        assert!(!versions.supports(RubyVersion(2, 2)));
        assert!(versions.supports(RubyVersion(2, 3)));
        assert!(versions.supports(DEFAULT_RUBY_VERSION));
        assert!(!SafeNavigation.safe_fixes());
        assert!(SafeNavigation.unsafe_fixes());
        assert!(SafeNavigation.default_enabled());
    }
    #[test]
    fn receiver_structure_arguments_binding_and_scopes_are_not_name_only() {
        for source in [
            "x = nil; x && x.foo",
            "self.x && self.x.foo",
            "obj[1] && obj[1].foo",
            "obj(a: 1) && obj(a: 1).foo",
            "@x.foo unless @x.nil?",
            "x&.foo && x.foo.bar",
            "x && x.foo? && x.bar?",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "x && self.x.foo",
            "self.x && x.foo",
            "obj[1] && obj[2].foo",
            "obj(a: 1) && obj(a: 2).foo",
            "x && x(x = nil).foo",
            "def f(x); y && x.foo; end",
            "x && proc { |x| x.foo }",
            "foo && bar { foo && foo.baz }",
        ] {
            // The inner block's own guard is checked, not paired with the
            // unrelated outer clause.
            let count = usize::from(source == "foo && bar { foo && foo.baz }");
            assert_eq!(offenses(source).len(), count, "{source}");
        }
    }
    #[test]
    fn equivalent_receiver_spellings_match_through_arguments_and_blocks() {
        for source in [
            "fetch(/foo/im) && fetch(/foo/mi).bar",
            "fetch(obj[1]) && fetch(obj.[](1)).bar",
            "/foo/im && /foo/mi.bar",
            "fetch(/#{pattern}/im) && fetch(/#{pattern}/mi).bar",
            "fetch([obj[1], /foo/im]) && fetch([obj.[](1), /foo/mi]).bar",
            "fetch { obj[1] } && fetch { obj.[](1) }.bar",
            "obj[1].fetch(/foo/im) && obj.[](1).fetch(/foo/mi).bar",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1), "{source}");
            assert_eq!(found[0].message, MSG, "{source}");
        }
        for source in [
            "fetch(/foo/im) && fetch(/foo/i).bar",
            "fetch(/foo/im) && fetch(/bar/mi).bar",
            "fetch(obj[1]) && fetch(obj.[](2)).bar",
            "fetch(obj[1]) && fetch(other.[](1)).bar",
            "fetch(obj[1]) && fetch(obj&.[](1)).bar",
            "fetch(/#{pattern}/im) && fetch(/#{other}/mi).bar",
            "fetch { obj[1] } && fetch { obj.[](2) }.bar",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn fixed_chain_length_nil_methods_and_operator_assignment_exemptions() {
        for source in [
            "x && x.a.b",
            "x && x.to_i",
            "x && x.nil?",
            "x && x.a&.nil?",
            "x ? x.*(2) : nil",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "x && x.a.b.c",
            "x && x.a.nil?",
            "x && x.a.to_d",
            "x && x.a.blank?",
            "x && x.empty?",
            "x.empty? if x",
            "x && x[0]",
            "x.bar = y if x",
            "x + y if x",
            "x ? x.foo[0] : nil",
            "if x; x.foo; x.bar; end",
            "if x; x.foo; else; nil; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn wide_conjunctions_and_large_argument_receivers_have_bounded_main_ast_work() {
        use crate::cops::{ast::CALL_ARGUMENT_VISITS, testing::correction_offenses};
        for count in [100, 1000] {
            let source = format!(
                "[{}]",
                std::iter::repeat_n("foo && ((foo.bar && bar) || qux)", count)
                    .collect::<Vec<_>>()
                    .join(",\n")
            );
            STEPS.set(0);
            assert_eq!(correction_offenses(&SafeNavigation, &source).len(), count);
            assert!(STEPS.get() < count * 30, "{} steps", STEPS.get());
            let args = std::iter::repeat_n("1", count)
                .collect::<Vec<_>>()
                .join(", ");
            CALL_ARGUMENT_VISITS.set(0);
            assert_eq!(
                correction_offenses(&SafeNavigation, &format!("foo({args}) && foo({args}).bar"))
                    .len(),
                1
            );
            assert!(CALL_ARGUMENT_VISITS.get() <= count * 4);
        }
    }
    #[test]
    fn source_line_receivers_compare_runtime_line_values_not_token_spelling() {
        for source in [
            "__LINE__ && __LINE__.foo",
            "__LINE__ && 1.foo",
            "f(__LINE__) && f(1).bar",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "__LINE__ &&\n__LINE__.foo",
            "f(__LINE__) &&\nf(__LINE__).bar",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn reports_guarded_calls_at_the_whole_expression() {
        for source in [
            "foo && foo.bar",
            "foo.bar if foo",
            "foo ? foo.bar : nil",
            "foo.bar unless foo.nil?",
            "foo.bar if !foo.nil?",
            "foo.bar unless !foo",
        ] {
            let found = offenses(&format!("def f\n  {source}\nend\n"));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                "Use safe navigation (`&.`) instead of checking if an object exists before calling the method."
            );
        }
    }
    #[test]
    fn parenthesized_checks_and_ternary_outer_colon_calls() {
        for source in [
            "foo && ((foo.bar && bar) || qux)",
            "(foo) && (foo).bar",
            "foo ? foo.one::two : nil",
            "foo.nil? ? nil : foo.one::two",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "foo && foo.one::two",
            "foo.one::two if foo",
            "foo ? foo::one.two : nil",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn adjacent_conjunction_clauses_and_parenthesized_descendants() {
        for (source, columns) in [
            ("foo && foo.bar && foo.bar.baz", vec![1, 8]),
            ("bar && foo && foo.bar", vec![8]),
            ("foo && foo.x? && bar && bar.y?", vec![1, 18]),
            ("foo && (foo.bar? && (foo.baz? && foo.quux?))", vec![1]),
            ("foo && (foo.bar? || (foo.baz? && foo.quux?))", vec![1]),
            ("foo && (bar && (bar.y?))", vec![9]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                columns,
                "{source}"
            );
        }
    }
    #[test]
    fn prism_literal_blocks_csend_negation_and_single_begin_stripping() {
        for source in [
            "foo && foo.empty? {}",
            "foo.empty? {} if foo",
            "foo && foo&.!",
            "foo ? foo&.one&.! : nil",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "foo && ((foo.bar))",
            "foo && foo.empty?(&:x)",
            "foo && foo.!",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn disjunction_requires_grouped_conjunction_and_blocks_bound_ternary_operator_checks() {
        assert!(offenses("foo && (bar || foo.baz && other)").is_empty());
        for source in ["foo ? foo.one {}[0] : nil", "foo ? foo.one {} + 1 : nil"] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("foo && foo.one {}[0]").is_empty());
    }
    #[test]
    fn enclosing_send_safety_uses_context_and_respects_prism_block_ownership() {
        for source in [
            "foo && f(x && x.bar)",
            "outer(x.bar if x)",
            "(x && x.bar).nil?",
            "outer(x && x.bar {})",
            "outer(x && x.a&.b)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "outer(x && x.a.b)",
            "object.outer(x && x.bar)",
            "outer(x ? x.bar : nil)",
            "foo && bar { x && x.bar }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn conjunction_highlight_stops_at_the_adjacent_clause() {
        let found = offenses("foo && (\n  foo.bar &&\n  unrelated\n)");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
        let found = offenses("foo && (\n  foo.bar\n)");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end_line, 3);
    }
    #[test]
    fn ignores_unguarded_different_or_nil_return_changing_checks() {
        for source in [
            "foo.bar",
            "foo&.bar",
            "nil&.bar",
            "foo && bar.baz",
            "foo.nil? || foo.bar",
            "!foo.nil? && foo.bar",
            "foo.bar if foo.respond_to?(:bar)",
            "foo.bar unless foo",
            "foo.bar if !foo",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
