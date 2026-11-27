//! `Layout/IndentationWidth` — two spaces for bodies, not continuation alignment.

use std::{collections::BTreeMap, ops::Range};

use ruby_prism::{Location, Node};

use super::alignment::{EndAlignWith, assignment_owner};

use crate::{
    cops::{
        Companions, Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::{SourceFile, literal_interior},
};

#[derive(Default)]
pub struct IndentationWidth {
    /// Layout/EndAlignment's `enforced_style_align_with`: the base of the
    /// body of an `if`, `unless`, `while` or `until` that is the value of an
    /// assignment.
    end_align_with: EndAlignWith,
}

impl Cop for IndentationWidth {
    fn name(&self) -> &'static str {
        "Layout/IndentationWidth"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(IndentationWidth {
            end_align_with: companions.end_align_with,
        }))
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        checks(node, cx, file, self.end_align_with, &mut |check| {
            let (line, col) = file.line_col(check.highlight);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!("Use 2 (not {}) spaces for indentation.", check.indentation),
                line,
                end_line: line,
                col,
            });
        });
    }

    fn wants_leave_node(&self) -> bool {
        true
    }

    fn leave_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none()
            || !cx.corrections_requested()
            || !out.iter().any(|o| o.cop == self.name())
        {
            return;
        }
        // Correction-only planning is one typed-child-aware walk per file.
        // Select disjoint expression spans BEFORE scanning their lines: nested
        // errors must not cause quadratic rescans or interfering shifts.
        struct Plans<'a, 'pr> {
            file: &'a SourceFile,
            end_align_with: EndAlignWith,
            ancestors: Vec<Node<'pr>>,
            checks: Vec<Check>,
            interiors: Vec<Range<usize>>,
            pairs: Vec<(usize, usize)>,
        }
        impl<'pr> NodeVisitor<'pr> for Plans<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                let cx = NodeContext {
                    ancestors: &self.ancestors,
                    corrections_requested: true,
                    require_order_reports: &Default::default(),
                };
                checks(&node, &cx, self.file, self.end_align_with, &mut |c| {
                    self.checks.push(c)
                });
                if let Some(range) = literal_interior(&node, self.file) {
                    self.interiors.push(range);
                }
                keyword_pairs(&node, &mut self.pairs);
                self.ancestors.push(node);
            }
            fn leave(&mut self) {
                self.ancestors.pop();
            }
        }
        let mut plans = Plans {
            file,
            end_align_with: self.end_align_with,
            ancestors: Vec::new(),
            checks: Vec::new(),
            interiors: Vec::new(),
            pairs: Vec::new(),
        };
        walk(&mut plans, node);
        plans
            .checks
            .sort_unstable_by_key(|c| (c.expression.start, std::cmp::Reverse(c.expression.end)));
        plans.interiors.sort_unstable_by_key(|r| (r.start, r.end));
        plans
            .pairs
            .sort_unstable_by_key(|&(opener, keyword)| (keyword, opener));
        let mut interiors: Vec<Range<usize>> = Vec::new();
        for range in plans.interiors {
            if let Some(last) = interiors.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
            } else {
                interiors.push(range);
            }
        }
        let mut fixes = BTreeMap::new();
        let mut covered_until = 0;
        for check in plans.checks {
            if check.expression.start < covered_until {
                continue;
            }
            // Reserve even withheld spans, so descendants of a block comment
            // or a disabled region cannot receive partial subtree corrections.
            covered_until = check.expression.end;
            if let Some(fix) = correction(file, node, &check, &interiors, &plans.pairs) {
                fixes.insert(file.line_col(check.highlight), fix);
            }
        }
        for offense in out.iter_mut().filter(|o| o.cop == self.name()) {
            offense.correction = fixes.remove(&(offense.line, offense.col));
        }
    }
}

struct Check {
    highlight: usize,
    expression: Range<usize>,
    indentation: i64,
}

/// Map Prism's transparent statement lists and implicit rescue/ensure wrapper
/// to the first expression RuboCop checks. Ordinary bodies check only their
/// first statement; classes/modules check each member separately.
fn body(node: &Node<'_>) -> Option<Range<usize>> {
    if let Some(statements) = node.as_statements_node() {
        return body(&statements.body().iter().next()?);
    }
    if let Some(begin) = node.as_begin_node()
        && begin.begin_keyword_loc().is_none()
    {
        return protected_body(&begin);
    }
    Some(node.location().start_offset()..node.location().end_offset())
}

fn protected_body(begin: &ruby_prism::BeginNode<'_>) -> Option<Range<usize>> {
    let statements = begin.statements()?;
    let start = statements.location().start_offset();
    if begin.rescue_clause().is_some() || begin.ensure_clause().is_some() {
        let end = begin
            .end_keyword_loc()
            .map_or(begin.location().end_offset(), |l| l.start_offset());
        Some(start..end)
    } else {
        body(&statements.as_node())
    }
}

pub(super) fn bare_access<'a, 'pr: 'a>(
    node: &Node<'pr>,
    ancestors: impl Iterator<Item = &'a Node<'pr>>,
) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.receiver().is_none()
            && call.arguments().is_none()
            && call.block().is_none()
            && matches!(
                call.name().as_slice(),
                b"public" | b"protected" | b"private" | b"module_function"
            )
            && in_macro_scope(node, ancestors)
    })
}

/// Port of RuboCop AST's `in_macro_scope?`. Statement lists and grouping
/// nodes are transparent; loops, methods, rescue and ordinary calls are not.
/// Prism puts a block inside its call, whereas RuboCop puts the call inside
/// the block, so skip that call when following a block's enclosing scope.
pub(super) fn in_macro_scope<'a, 'pr: 'a>(
    node: &Node<'pr>,
    ancestors: impl Iterator<Item = &'a Node<'pr>>,
) -> bool {
    let mut child_start = node.location().start_offset();
    let mut ancestors = ancestors.peekable();
    while let Some(parent) = ancestors.next() {
        match parent {
            Node::ProgramNode { .. }
            | Node::ClassNode { .. }
            | Node::ModuleNode { .. }
            | Node::SingletonClassNode { .. } => return true,
            Node::StatementsNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::ElseNode { .. }
            | Node::LambdaNode { .. } => {}
            Node::BeginNode { .. } => {
                let begin = parent.as_begin_node().unwrap();
                if begin.rescue_clause().is_some() || begin.ensure_clause().is_some() {
                    return false;
                }
            }
            Node::IfNode { .. } => {
                if parent
                    .as_if_node()
                    .unwrap()
                    .predicate()
                    .location()
                    .start_offset()
                    == child_start
                {
                    return false;
                }
            }
            Node::UnlessNode { .. } => {
                if parent
                    .as_unless_node()
                    .unwrap()
                    .predicate()
                    .location()
                    .start_offset()
                    == child_start
                {
                    return false;
                }
            }
            Node::BlockNode { .. } => {
                if let Some(call) = ancestors.peek().and_then(|n| n.as_call_node()) {
                    if class_constructor(&call) {
                        return true;
                    }
                    child_start = ancestors.next().unwrap().location().start_offset();
                    continue;
                }
            }
            Node::CallNode { .. } if class_constructor(&parent.as_call_node().unwrap()) => {
                return true;
            }
            _ => return false,
        }
        child_start = parent.location().start_offset();
    }
    true
}

fn class_constructor(call: &ruby_prism::CallNode<'_>) -> bool {
    use crate::cops::ast::is_global_const;
    !call.is_safe_navigation()
        && call
            .receiver()
            .is_some_and(|receiver| match call.name().as_slice() {
                b"new" => is_global_const(&receiver, &["Class", "Module", "Struct"]),
                b"define" => is_global_const(&receiver, &["Data"]),
                _ => false,
            })
}

fn starts_with_access<'pr>(
    body_node: &Node<'pr>,
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
) -> bool {
    body_node.as_statements_node().is_some_and(|s| {
        s.body().len() > 1
            && s.body().iter().next().is_some_and(|first| {
                bare_access(&first, [body_node, node].into_iter().chain(cx.ancestors()))
            })
    })
}

fn starts_line(file: &SourceFile, offset: usize) -> bool {
    let start = file.line_start(file.line_of(offset));
    let prefix = &file.bytes[start..offset];
    let prefix = if start == 0 {
        prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix)
    } else {
        prefix
    };
    prefix
        .iter()
        .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c'))
}

fn checks<'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
    end_align_with: EndAlignWith,
    emit: &mut impl FnMut(Check),
) {
    // `check_assignment`: a conditional or loop that is the value of an
    // assignment, or the last argument of a call (`&.` too), takes the
    // start of that assignment as the base when Layout/EndAlignment does
    // not align with the keyword and nothing breaks the line between them.
    let owner = || {
        assignment_owner(node, cx.ancestors(), false, true)
            .filter(|owner| end_align_with.aligns_with_owner(file, owner, node))
            .map(|owner| owner.location().start_offset())
    };
    let mut check_range = |base: usize, expression: Option<Range<usize>>| {
        let Some(expression) = expression else {
            return;
        };
        if file.line_of(base) == file.line_of(expression.start)
            || !starts_line(file, expression.start)
        {
            return;
        }
        let indentation =
            i64::from(file.line_col(expression.start).1) - i64::from(file.line_col(base).1);
        if indentation == 2 {
            return;
        }
        emit(Check {
            highlight: if indentation > 0 {
                expression.start - indentation as usize
            } else {
                expression.start
            },
            expression,
            indentation,
        });
    };
    let mut check = |base: usize, body_node: Option<Node<'pr>>| {
        // A multi-statement body starting with an access modifier belongs to
        // the access-modifier/consistency cops, except class/module members.
        let skip = body_node
            .as_ref()
            .is_some_and(|b| starts_with_access(b, node, cx));
        if !skip {
            check_range(base, body_node.as_ref().and_then(body));
        }
    };
    match node {
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            if let Some(keyword) = n.if_keyword_loc()
                && keyword.start_offset() == n.location().start_offset()
            {
                let base = owner().unwrap_or(keyword.start_offset());
                check(base, n.statements().map(|s| s.as_node()));
            }
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node().unwrap();
            if n.keyword_loc().start_offset() == n.location().start_offset() {
                check(
                    owner().unwrap_or(n.keyword_loc().start_offset()),
                    n.statements().map(|s| s.as_node()),
                );
            }
        }
        Node::ElseNode { .. } => {
            let n = node.as_else_node().unwrap();
            // Ternaries also have ElseNode children, but their branches are
            // continuation expressions, not indentation-controlled bodies.
            if cx
                .parent()
                .and_then(Node::as_if_node)
                .is_some_and(|n| n.if_keyword_loc().is_none())
            {
                return;
            }
            let base = match cx.parent() {
                Some(Node::CaseNode { .. }) => cx
                    .parent()
                    .unwrap()
                    .as_case_node()
                    .unwrap()
                    .conditions()
                    .iter()
                    .last()
                    .and_then(|n| n.as_when_node().map(|n| n.keyword_loc())),
                Some(Node::CaseMatchNode { .. }) => cx
                    .parent()
                    .unwrap()
                    .as_case_match_node()
                    .unwrap()
                    .conditions()
                    .iter()
                    .last()
                    .and_then(|n| n.as_in_node().map(|n| n.in_loc())),
                _ => Some(n.else_keyword_loc()),
            };
            if let Some(base) = base {
                check(base.start_offset(), n.statements().map(|s| s.as_node()));
            }
        }
        Node::WhileNode { .. } => {
            let n = node.as_while_node().unwrap();
            if n.keyword_loc().start_offset() == n.location().start_offset()
                && file.line_of(n.predicate().location().start_offset())
                    == file.last_line_of(n.predicate().location())
            {
                check(
                    owner().unwrap_or(n.keyword_loc().start_offset()),
                    n.statements().map(|s| s.as_node()),
                );
            }
        }
        Node::UntilNode { .. } => {
            let n = node.as_until_node().unwrap();
            if n.keyword_loc().start_offset() == n.location().start_offset()
                && file.line_of(n.predicate().location().start_offset())
                    == file.last_line_of(n.predicate().location())
            {
                check(
                    owner().unwrap_or(n.keyword_loc().start_offset()),
                    n.statements().map(|s| s.as_node()),
                );
            }
        }
        Node::ForNode { .. } => {
            let n = node.as_for_node().unwrap();
            check(
                n.for_keyword_loc().start_offset(),
                n.statements().map(|s| s.as_node()),
            );
        }
        Node::WhenNode { .. } => {
            let n = node.as_when_node().unwrap();
            check(
                n.keyword_loc().start_offset(),
                n.statements().map(|s| s.as_node()),
            );
        }
        Node::InNode { .. } => {
            let n = node.as_in_node().unwrap();
            check(
                n.in_loc().start_offset(),
                n.statements().map(|s| s.as_node()),
            );
        }
        Node::RescueNode { .. } => {
            let n = node.as_rescue_node().unwrap();
            check(
                n.keyword_loc().start_offset(),
                n.statements().map(|s| s.as_node()),
            );
        }
        Node::EnsureNode { .. } => {
            let n = node.as_ensure_node().unwrap();
            check(
                n.ensure_keyword_loc().start_offset(),
                n.statements().map(|s| s.as_node()),
            );
        }
        Node::DefNode { .. } => {
            let n = node.as_def_node().unwrap();
            let mut base = n.def_keyword_loc().start_offset();
            // An adjacent modifier is a receiverless call with exactly one
            // argument, the definition (parentheses are allowed). Once found,
            // start_of_line uses its outermost enclosing send as the base.
            let mut ancestors = cx.ancestors();
            if let Some(args) = ancestors.next().and_then(Node::as_arguments_node)
                && args.arguments().len() == 1
                && let Some(call) = ancestors.next().and_then(Node::as_call_node)
                && call.receiver().is_none()
            {
                base = call.location().start_offset();
                while let Some(parent) = ancestors.next() {
                    let parent = if parent.as_arguments_node().is_some() {
                        ancestors.next()
                    } else {
                        Some(parent)
                    };
                    let Some(call) = parent.and_then(Node::as_call_node) else {
                        break;
                    };
                    base = call.location().start_offset();
                }
            }
            let skip = n
                .body()
                .as_ref()
                .is_some_and(|b| starts_with_access(b, node, cx));
            if !skip {
                check_range(base, n.body().as_ref().and_then(body));
            }
        }
        Node::BlockNode { .. } => {
            let n = node.as_block_node().unwrap();
            if starts_line(file, n.closing_loc().start_offset()) {
                check(n.closing_loc().start_offset(), n.body());
            }
        }
        Node::LambdaNode { .. } => {
            let n = node.as_lambda_node().unwrap();
            if starts_line(file, n.closing_loc().start_offset()) {
                check(n.closing_loc().start_offset(), n.body());
            }
        }
        Node::BeginNode { .. } => {
            let n = node.as_begin_node().unwrap();
            if n.begin_keyword_loc().is_some()
                && let Some(end) = n.end_keyword_loc()
                && starts_line(file, end.start_offset())
            {
                check_range(end.start_offset(), protected_body(&n));
            }
        }
        Node::ParenthesesNode { .. } => {
            let n = node.as_parentheses_node().unwrap();
            if starts_line(file, n.closing_loc().start_offset()) {
                let start = file.line_start(file.line_of(n.opening_loc().start_offset()));
                let prefix = file.bytes[start..n.opening_loc().start_offset()]
                    .iter()
                    .take_while(|b| matches!(b, b' ' | b'\t'))
                    .count();
                check_range(start + prefix, n.body().as_ref().and_then(body));
            }
        }
        Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. } => {
            let (base, members) = if let Some(n) = node.as_class_node() {
                (n.class_keyword_loc(), n.body())
            } else if let Some(n) = node.as_module_node() {
                (n.module_keyword_loc(), n.body())
            } else {
                let n = node.as_singleton_class_node().unwrap();
                (n.class_keyword_loc(), n.body())
            };
            let Some(members) = members else {
                return;
            };
            if body(&members)
                .is_none_or(|b| file.line_of(b.start) == file.line_of(base.start_offset()))
            {
                return;
            }
            if let Some(statements) = members.as_statements_node() {
                for (index, member) in statements.body().iter().enumerate() {
                    if index != 0
                        && bare_access(&member, [&members, node].into_iter().chain(cx.ancestors()))
                    {
                        continue;
                    }
                    check_range(base.start_offset(), body(&member));
                }
            } else {
                check_range(base.start_offset(), body(&members));
            }
        }
        _ => {}
    }
}

/// Record the keywords native Ruby 3.4 compares for its "mismatched
/// indentations" warning as `(earlier, later keyword)` offsets, as its
/// parser pairs them: each `elsif`, `when` and `in` and a conditional's `else`
/// with the construct's opening keyword; each `rescue` and a protected body's
/// `else` with the opener (`begin`, `def`, `class`, `module`, `class << self`
/// or a stabby lambda's `->`); `ensure` and `end` (or a lambda's `}`) with a
/// preceding `else` when there is one and otherwise with the opener. A `case`
/// `else` is recorded too, conservatively; Ruby compares no block keywords.
fn keyword_pairs(node: &Node<'_>, out: &mut Vec<(usize, usize)>) {
    fn pair(out: &mut Vec<(usize, usize)>, opener: usize, keyword: Option<Location<'_>>) {
        if let Some(keyword) = keyword {
            out.push((opener, keyword.start_offset()));
        }
    }
    /// A protected body's clauses and closing keyword.
    fn protected(
        body: Option<Node<'_>>,
        opener: usize,
        end: Option<Location<'_>>,
        out: &mut Vec<(usize, usize)>,
    ) {
        let begin = body
            .as_ref()
            .and_then(Node::as_begin_node)
            .filter(|b| b.begin_keyword_loc().is_none());
        let Some(begin) = begin else {
            return pair(out, opener, end);
        };
        clauses(&begin, opener, end, out);
    }
    fn clauses(
        begin: &ruby_prism::BeginNode<'_>,
        opener: usize,
        end: Option<Location<'_>>,
        out: &mut Vec<(usize, usize)>,
    ) {
        let mut rescue = begin.rescue_clause();
        while let Some(clause) = rescue {
            pair(out, opener, Some(clause.keyword_loc()));
            rescue = clause.subsequent();
        }
        let mut last = opener;
        if let Some(clause) = begin.else_clause() {
            pair(out, opener, Some(clause.else_keyword_loc()));
            last = clause.else_keyword_loc().start_offset();
        }
        if let Some(clause) = begin.ensure_clause() {
            pair(out, last, Some(clause.ensure_keyword_loc()));
        }
        pair(out, last, end);
    }
    match node {
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            // `elsif` nodes are recorded by their `if`.
            let Some(opener) = n.if_keyword_loc().filter(|k| k.as_slice() == b"if") else {
                return;
            };
            let opener = opener.start_offset();
            let mut last = opener;
            let mut next = n.subsequent();
            while let Some(clause) = next {
                if let Some(elsif) = clause.as_if_node() {
                    pair(out, opener, elsif.if_keyword_loc());
                    next = elsif.subsequent();
                } else {
                    let keyword = clause.as_else_node().unwrap().else_keyword_loc();
                    last = keyword.start_offset();
                    pair(out, opener, Some(keyword));
                    next = None;
                }
            }
            pair(out, last, n.end_keyword_loc());
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node().unwrap();
            let opener = n.keyword_loc().start_offset();
            let mut last = opener;
            if let Some(clause) = n.else_clause() {
                last = clause.else_keyword_loc().start_offset();
                pair(out, opener, Some(clause.else_keyword_loc()));
            }
            pair(out, last, n.end_keyword_loc());
        }
        Node::WhileNode { .. } => {
            let n = node.as_while_node().unwrap();
            pair(out, n.keyword_loc().start_offset(), n.closing_loc());
        }
        Node::UntilNode { .. } => {
            let n = node.as_until_node().unwrap();
            pair(out, n.keyword_loc().start_offset(), n.closing_loc());
        }
        Node::ForNode { .. } => {
            let n = node.as_for_node().unwrap();
            pair(
                out,
                n.for_keyword_loc().start_offset(),
                Some(n.end_keyword_loc()),
            );
        }
        Node::CaseNode { .. } => {
            let n = node.as_case_node().unwrap();
            let opener = n.case_keyword_loc().start_offset();
            for when in n.conditions().iter().filter_map(|c| c.as_when_node()) {
                pair(out, opener, Some(when.keyword_loc()));
            }
            pair(out, opener, n.else_clause().map(|e| e.else_keyword_loc()));
            pair(out, opener, Some(n.end_keyword_loc()));
        }
        Node::CaseMatchNode { .. } => {
            let n = node.as_case_match_node().unwrap();
            let opener = n.case_keyword_loc().start_offset();
            for clause in n.conditions().iter().filter_map(|c| c.as_in_node()) {
                pair(out, opener, Some(clause.in_loc()));
            }
            pair(out, opener, n.else_clause().map(|e| e.else_keyword_loc()));
            pair(out, opener, Some(n.end_keyword_loc()));
        }
        Node::BeginNode { .. } => {
            let n = node.as_begin_node().unwrap();
            if let Some(opener) = n.begin_keyword_loc() {
                clauses(&n, opener.start_offset(), n.end_keyword_loc(), out);
            }
        }
        Node::DefNode { .. } => {
            let n = node.as_def_node().unwrap();
            let opener = n.def_keyword_loc().start_offset();
            protected(n.body(), opener, n.end_keyword_loc(), out);
        }
        Node::ClassNode { .. } => {
            let n = node.as_class_node().unwrap();
            let opener = n.class_keyword_loc().start_offset();
            protected(n.body(), opener, Some(n.end_keyword_loc()), out);
        }
        Node::ModuleNode { .. } => {
            let n = node.as_module_node().unwrap();
            let opener = n.module_keyword_loc().start_offset();
            protected(n.body(), opener, Some(n.end_keyword_loc()), out);
        }
        Node::SingletonClassNode { .. } => {
            let n = node.as_singleton_class_node().unwrap();
            let opener = n.class_keyword_loc().start_offset();
            protected(n.body(), opener, Some(n.end_keyword_loc()), out);
        }
        // Unlike blocks, a stabby lambda's `->` is compared with its closing
        // `end` or `}` and its clauses.
        Node::LambdaNode { .. } => {
            let n = node.as_lambda_node().unwrap();
            let opener = n.operator_loc().start_offset();
            protected(n.body(), opener, Some(n.closing_loc()), out);
        }
        _ => {}
    }
}

/// The column native Ruby (Prism) reports for its mismatched-indentation
/// warning after `bytes`: a tab advances to the next multiple of 8, every
/// other byte by one.
fn native_column<'a>(bytes: impl Iterator<Item = &'a u8>) -> usize {
    bytes.fold(0, |column, &b| match b {
        b'\t' => (column / 8 + 1) * 8,
        _ => column + 1,
    })
}

fn correction(
    file: &SourceFile,
    root: &Node<'_>,
    check: &Check,
    interiors: &[Range<usize>],
    pairs: &[(usize, usize)],
) -> Option<Correction> {
    let first = file.line_of(check.expression.start);
    let last = file.line_of(check.expression.end.saturating_sub(1));
    let start = file.line_start(first);
    let end = check.expression.end;
    let end = file.bytes[..end]
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(end, |i| i + 1);
    let comments = file.comments();
    let index = comments.partition_point(|c| c.end <= start);
    if comments[index..]
        .iter()
        .take_while(|c| c.start < end)
        .any(|c| file.bytes[c.clone()].starts_with(b"=begin"))
    {
        return None;
    }
    // Moving a nested expression inside a scope containing a block comment
    // is also deliberately withheld, matching the upstream scope exception.
    let delta = 2 - check.indentation;
    let mut replacement = Vec::new();
    let mut cursor = start;
    let mut moved = Vec::new();
    let mut partial = false;
    for line in first..=last {
        let line_start = file.line_start(line);
        let text = file.line_text(line);
        let prefix = text
            .iter()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count();
        if prefix == text.len() || line_start + prefix > end {
            continue;
        }
        let range = line_start..line_start + prefix;
        let index = interiors.partition_point(|r| r.end <= range.start);
        if interiors.get(index).is_some_and(|r| r.start <= range.start)
            || file.overlaps_literal(root, &range)
        {
            continue;
        }
        let target = prefix as i64 + delta;
        // Like upstream, do not remove tokens from less-indented clause lines.
        if target < 0 || (target == 0 && &text[prefix..] == b"__END__") {
            partial |= target < 0;
            continue;
        }
        replacement.extend_from_slice(&file.bytes[cursor..range.start]);
        let kept = if delta > 0 {
            replacement.extend_from_slice(&file.bytes[range.clone()]);
            replacement.extend(std::iter::repeat_n(b' ', delta as usize));
            range.clone()
        } else if line == first {
            range.start..range.end - (-delta) as usize
        } else {
            range.start + (-delta) as usize..range.end
        };
        if delta < 0 {
            replacement.extend_from_slice(&file.bytes[kept.clone()]);
        }
        moved.push((line, kept, delta.max(0) as usize, range.end));
        cursor = range.end;
    }
    let moved_line = |offset: usize| {
        moved
            .binary_search_by_key(&file.line_of(offset), |m| m.0)
            .ok()
    };
    // A partial negative shift leaves less-indented lines behind. When that
    // moves only one keyword of a natively compared pair, such as an opener
    // whose less-indented `end` stays, it would erase or introduce a native
    // mismatched-indentation warning: withhold the complete edit.
    // Pairs are indexed by their later keyword, which lies in the span
    // whenever the opener does (nested constructs end inside it).
    let first_pair = pairs.partition_point(|&(_, keyword)| keyword < start);
    let span_pairs = || {
        pairs[first_pair..]
            .iter()
            .take_while(|&&(_, keyword)| keyword < end)
    };
    if partial {
        let is_moved = |offset: usize| moved_line(offset).is_some();
        if span_pairs().any(|&(opener, keyword)| is_moved(opener) != is_moved(keyword)) {
            return None;
        }
    }
    // Native Ruby expands tabs to 8-column stops. Removing trailing prefix
    // bytes on the first line and leading ones on the others can then move
    // compared keywords by different amounts: withhold the edit when any
    // pair's native column order would change, erasing or introducing the
    // warning. Space-only spans move uniformly and skip this; a
    // space-only clause moved away from an outside opener is not guarded yet.
    if file.bytes[start..end].contains(&b'\t') {
        let column = |offset: usize, shifted: bool| {
            let line = file.line_of(offset);
            let mut line_start = file.line_start(line);
            if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
                line_start += 3;
            }
            let (prefix, added, rest) = match moved_line(offset).map(|i| &moved[i]) {
                Some((_, kept, added, prefix_end)) if shifted => {
                    (kept.clone(), *added, *prefix_end..offset)
                }
                _ => (line_start..line_start, 0, line_start..offset),
            };
            native_column(
                file.bytes[prefix]
                    .iter()
                    .chain(std::iter::repeat_n(&b' ', added))
                    .chain(&file.bytes[rest]),
            )
        };
        if span_pairs().any(|&(opener, keyword)| {
            let order = |shifted| column(keyword, shifted).cmp(&column(opener, shifted));
            order(false) != order(true)
        }) {
            return None;
        }
    }
    replacement.extend_from_slice(&file.bytes[cursor..end]);
    let replacement = String::from_utf8(replacement).ok()?;
    if replacement.as_bytes() == &file.bytes[start..end] {
        return None;
    }
    Some(Correction {
        range: start..end,
        replacement,
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/indentation_width_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/IndentationWidth")
            .collect()
    }

    use crate::cops::testing::{assert_correction, correction_offenses};

    #[test]
    fn pinned_body_constructs_and_corrections() {
        for (before, after) in [
            (
                "if a\n   b\n     c\nelsif d\n e\nelse\n    f\nend\n",
                "if a\n  b\n     c\nelsif d\n  e\nelse\n  f\nend\n",
            ),
            (
                "unless a\n b\nelse\n   c\nend\n",
                "unless a\n  b\nelse\n  c\nend\n",
            ),
            ("while a\n b\nend\n", "while a\n  b\nend\n"),
            ("until a\n b\nend\n", "until a\n  b\nend\n"),
            ("for x in xs\n b\nend\n", "for x in xs\n  b\nend\n"),
            (
                "case a\nwhen b\n c\nwhen d\n  e\nelse\n   f\nend\n",
                "case a\nwhen b\n  c\nwhen d\n  e\nelse\n  f\nend\n",
            ),
            (
                "case a\nin b\n c\nin d\n  e\nelse\n   f\nend\n",
                "case a\nin b\n  c\nin d\n  e\nelse\n  f\nend\n",
            ),
            (
                "class A\n    def x\n    end\nend\n",
                "class A\n  def x\n  end\nend\n",
            ),
            (
                "module A\n    def x\n    end\nend\n",
                "module A\n  def x\n  end\nend\n",
            ),
            (
                "class << self\n    def x\n    end\nend\n",
                "class << self\n  def x\n  end\nend\n",
            ),
            ("def self.x\n   f\nend\n", "def self.x\n  f\nend\n"),
            ("def x =\n    foo\n", "def x =\n  foo\n"),
            (
                "public foo def x\n      f\nend\n",
                "public foo def x\n  f\nend\n",
            ),
            ("a = func do\n b\nend\n", "a = func do\n  b\nend\n"),
            ("func {\n   _1&.b\n}\n", "func {\n  _1&.b\n}\n"),
            ("func {\n   it&.b\n}\n", "func {\n  it&.b\n}\n"),
            ("f = -> do\n   x\nend\n", "f = -> do\n  x\nend\n"),
            (
                "foo\n  .bar do |x|\nx\nend\n",
                "foo\n  .bar do |x|\n  x\nend\n",
            ),
            (
                "x = (\nfoo - bar\n).freeze\n",
                "x = (\n  foo - bar\n).freeze\n",
            ),
            (
                "begin\n a\nrescue A\n b\nrescue B\n c\nelse\n   d\nensure\n    e\nend\n",
                "begin\n  a\n rescue A\n  b\n rescue B\n  c\n else\n    d\n ensure\n     e\nend\n",
            ),
            (
                "def x\n  a\nrescue A\n b\nrescue B\n c\nend\n",
                "def x\n  a\nrescue A\n  b\nrescue B\n  c\nend\n",
            ),
            (
                "something = begin\n a\n   b\nend while cond\n",
                "something = begin\n  a\n   b\nend while cond\n",
            ),
            (
                "do_something do\n    foo\nensure\n  handle_error\nend\n",
                "do_something do\n  foo\nensure\nhandle_error\nend\n",
            ),
        ] {
            assert!(!offenses(before).is_empty(), "{before}");
            assert_correction(&IndentationWidth::default(), before, after);
        }
    }

    #[test]
    fn fixed_end_alignment_is_keyword_and_can_report_negative_offsets() {
        for keyword in ["if", "while", "until"] {
            let before = format!("var = {keyword} a\n  b\nend\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].message, "Use 2 (not -4) spaces for indentation.");
            assert_correction(
                &IndentationWidth::default(),
                &before,
                &format!("var = {keyword} a\n        b\nend\n"),
            );
        }
        for source in [
            "var = if a\n        b\n      else\n        c\n      end\n",
            "var =\n  if a\n    b\n  end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn scope_exceptions_and_alignment_are_not_depth_indentation() {
        for source in [
            "class A; foo\n      def x\n      end\n        def y\n        end\nend\n",
            "module A; foo\n      def x\n      end\nend\n",
            "foo {\ndef x\nend }\n",
            "begin\n      foo\nrescue; end\n",
            "while foo(\n  a, b\n)\nx\nend\n",
            "case x\n    when a,\n         b\n      c\n    else\n      d\nend\n",
            "case x\n    in [a,\n        b]\n      c\n    else\n      d\nend\n",
            // Else body is relative to the last when/in, not the else token.
            "case x\n  when a\n    b\nelse\n    c\nend\n",
            "case x\n  in a\n    b\nelse\n    c\nend\n",
            "foo\n  .bar do\n    x\n  end\n",
            "Foo.bar.\n  baz.\n  map do\n  x\nend\n",
            "x = (foo +\n        bar)\n",
            "if a\nthen b\nelse c\nend\n",
            "case x\nwhen 1\nthen y\nelse z\nend\n",
            "begin\nrescue\n  x\nensure\n  y\nend\n",
            "def x\nrescue\n  y\nensure\n  z\nend\n",
            "x = %w(\n      a\n b\n)\n",
            "__END__\n if x\ny\n end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_access_modifiers_check_first_and_all_nonmodifier_members() {
        let before = "class A\n      private\n\n    def x\n    end\n\n      public\n    def y\n    end\nend\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 1), (4, 1), (8, 1)]
        );
        assert_correction(
            &IndentationWidth::default(),
            before,
            "class A\n  private\n\n  def x\n  end\n\n      public\n  def y\n  end\nend\n",
        );
    }

    fn assert_unchanged(source: &str) {
        assert!(offenses(source).is_empty(), "{source}");
        let found = correction_offenses(&IndentationWidth::default(), source);
        assert!(found.is_empty(), "{source}");
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        assert_eq!(applied.bytes, source.as_bytes());
    }

    #[test]
    fn multiline_ternaries_are_continuations_not_bodies() {
        for source in [
            "x = a ? b :\n  c\n",
            "x = a ?\n  b :\n  c\n",
            "x = a ? b :\n        c\n",
            "def x\n  a ? b :\n    c\nend\n",
        ] {
            assert_unchanged(source);
        }
    }

    #[test]
    fn access_modifier_exemptions_require_macro_scope() {
        for source in [
            "module M\n  def x\n  end\nmodule_function\nend\n",
            "if x\nprivate\n  work\nend\n",
            "foo do\nprivate\n  work\nend\n",
            "unless x\nmodule_function\n  work\nend\n",
            "if x\n  work\nelse\nmodule_function\n  work\nend\n",
            "def x\n  Class.new do\nprivate\n    work\n  end\nend\n",
            "def x\n  ::Struct.new(:x) do\nmodule_function\n    work\n  end\nend\n",
        ] {
            assert_unchanged(source);
        }
        for (before, after, indentation) in [
            // Grouping/kwbegin checks the first expression, not the entire
            // statement list, even when that expression is a real modifier.
            (
                "begin\nprivate\n  work\nend\n",
                "begin\n  private\n  work\nend\n",
                0,
            ),
            ("(\nprivate\n  work\n)\n", "(\n  private\n  work\n)\n", 0),
            (
                "def x\nprivate\n  work\nend\n",
                "def x\n  private\n  work\nend\n",
                0,
            ),
            (
                "def x\nmodule_function\n  work\nend\n",
                "def x\n  module_function\n  work\nend\n",
                0,
            ),
            (
                "while x\nprivate\n  work\nend\n",
                "while x\n  private\n  work\nend\n",
                0,
            ),
            (
                "def x\n  foo do\nprivate\n    work\n  end\nend\n",
                "def x\n  foo do\n    private\n    work\n  end\nend\n",
                -2,
            ),
            (
                "if (\nprivate\n  work\n)\n  body\nend\n",
                "if (\n  private\n  work\n)\n  body\nend\n",
                0,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            let expected_line = if before.contains("foo do") { 3 } else { 2 };
            assert_eq!((found[0].line, found[0].col), (expected_line, 1));
            assert_eq!(
                found[0].message,
                format!("Use 2 (not {indentation}) spaces for indentation.")
            );
            assert_correction(&IndentationWidth::default(), before, after);
        }
    }

    #[test]
    fn adjacent_definitions_require_a_receiverless_single_argument_call() {
        for source in [
            "private(def x\n  work\nend)\n",
            "public(foo(def x\n  work\nend))\n",
            "private(\ndef x\n  work\nend)\n",
            "obj.foo private def x\n  work\nend\n",
            "private def self.x\n  work\nend\n",
        ] {
            assert_unchanged(source);
        }
        for (before, after, indentation) in [
            (
                "obj.foo def x\n  work\nend\n",
                "obj.foo def x\n          work\nend\n",
                -6,
            ),
            (
                "obj&.foo def x\n  work\nend\n",
                "obj&.foo def x\n           work\nend\n",
                -7,
            ),
            (
                "foo(def x\n  work\nend, 1)\n",
                "foo(def x\n      work\nend, 1)\n",
                -2,
            ),
            (
                "private(def x\n    work\nend)\n",
                "private(def x\n  work\nend)\n",
                4,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col),
                (2, if indentation > 0 { 1 } else { 3 })
            );
            assert_eq!(
                found[0].message,
                format!("Use 2 (not {indentation}) spaces for indentation.")
            );
            assert_correction(&IndentationWidth::default(), before, after);
        }
    }

    #[test]
    fn utf8_bom_crlf_and_literals_keep_their_bytes() {
        for literal in [
            "\"a\n b\"",
            "'a\n b'",
            "%q(a\n b)",
            "%Q(a\n b)",
            "/a\n b/",
            "%r(a\n b)",
            "%w(a\n b)",
            "%W(a\n b)",
            "%i(a\n b)",
            "%I(a\n b)",
            "%x(a\n b)",
            ":\"a\n b\"",
        ] {
            let before = format!("def x\n   {literal}\nend\n");
            let after = format!("def x\n  {literal}\nend\n");
            assert_correction(&IndentationWidth::default(), &before, &after);
        }
        let before = "\u{feff}module M日本語\r\n    def élève\r\n      \"é\"\r\n    end\r\nend\r\n__END__\r\n  raw\r\n";
        let after = "\u{feff}module M日本語\r\n  def élève\r\n    \"é\"\r\n  end\r\nend\r\n__END__\r\n  raw\r\n";
        assert_correction(&IndentationWidth::default(), before, after);
        assert!(offenses(after).is_empty());
        assert_correction(
            &IndentationWidth::default(),
            "def x\n    1\nend",
            "def x\n  1\nend",
        );
    }

    #[test]
    fn pinned_heredoc_and_block_comment_safety() {
        let before = "module Foo\nmodule Bar\n  SOMETHING = <<GOO\ntext\nmore text\nfoo\nGOO\n  def baz\n    do_something(\"#{x}\")\n  end\nend\nend\n";
        let after = "module Foo\n  module Bar\n    SOMETHING = <<GOO\ntext\nmore text\nfoo\nGOO\n    def baz\n      do_something(\"#{x}\")\n    end\n  end\nend\n";
        assert_correction(&IndentationWidth::default(), before, after);
        let before =
            "module Foo\nclass Bar\n=begin\ncomment\n=end\ndef baz\ndo_something\nend\nend\nend\n";
        assert_eq!(offenses(before).len(), 3);
        assert!(
            correction_offenses(&IndentationWidth::default(), before)
                .iter()
                .all(|o| o.correction.is_none())
        );
        assert_correction(&IndentationWidth::default(), before, before);
    }

    #[test]
    fn directives_filter_highlights_and_entire_shifted_subtrees() {
        for prefix in ["rubocop", "rustocop"] {
            for target in [
                "Layout/IndentationWidth",
                "IndentationWidth",
                "Layout",
                "all",
            ] {
                let before = format!(
                    "# {prefix}:disable {target}\nif a\n x\nend\n# {prefix}:enable {target}\nif b\n y # {prefix}:disable {target}\nend\n# {prefix}:disable-next {target}\nif c\n z\nend\nif d\n w\nend\n"
                );
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col), (14, 1));
                assert_correction(
                    &IndentationWidth::default(),
                    &before,
                    &before.replace("\n w\n", "\n  w\n"),
                );
            }
            let before = format!(
                "module A\nclass B\n  # {prefix}:disable Layout/IndentationWidth\n  def x\n  end\n  # {prefix}:enable Layout/IndentationWidth\nend\nend\n"
            );
            // As in RuboCop, the enabled offense shifts its whole subtree,
            // disabled lines included.
            let found = correction_offenses(&IndentationWidth::default(), &before);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_some(), "{before}");
            assert_correction(
                &IndentationWidth::default(),
                &before,
                &format!(
                    "module A\n  class B\n    # {prefix}:disable Layout/IndentationWidth\n    def x\n    end\n    # {prefix}:enable Layout/IndentationWidth\n  end\nend\n"
                ),
            );
        }
    }

    #[test]
    fn nested_and_wide_correction_work_selects_disjoint_spans() {
        // Stay within the shared AST-depth budget (two nodes per module).
        let nested = format!("{}x\n{}", "module A\n".repeat(30), "end\n".repeat(30));
        let found = correction_offenses(&IndentationWidth::default(), &nested);
        assert_eq!(found.len(), 30);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        let wide = "class A\n    def x\n    end\n    def y\n    end\nend\n".repeat(2000);
        let found = correction_offenses(&IndentationWidth::default(), &wide);
        assert_eq!(found.len(), 4000);
        let mut end = 0;
        for o in found {
            let fix = o.correction.unwrap();
            assert!(fix.range.start >= end);
            end = fix.range.end;
        }
    }

    #[test]
    fn nested_interpolation_and_whitespace_delimiters_are_withheld_safely() {
        // The percent array is protected as a whole, even its code regions.
        for source in [
            "x = %W(a#{if true\n1\nend})\n",
            "x = \"#{if true\n 1\nend}\"\n",
            "x = %Q #{if true\n1\nend} \n",
        ] {
            assert!(!offenses(source).is_empty(), "{source}");
            assert_correction(&IndentationWidth::default(), source, source);
        }
        // A containing shift preserves both literal text and interpolation.
        let before = "class A\nclass B\n  X = \"a\n#{\nif true\n 1\nend\n}tail\"\nend\nend\n";
        let after = "class A\n  class B\n    X = \"a\n#{\nif true\n 1\nend\n}tail\"\n  end\nend\n";
        assert_correction(&IndentationWidth::default(), before, after);
    }

    #[test]
    fn subtree_shifts_do_not_create_a_data_marker() {
        assert_correction(
            &IndentationWidth::default(),
            "class A\n    def x\n  __END__\n    end\nend\n",
            "class A\n  def x\n  __END__\n  end\nend\n",
        );
    }

    /// A partial negative shift (less-indented lines stay) that
    /// moves only one keyword of a pair native Ruby compares (an opener whose
    /// less-indented clause or `end` stays, a less-indented opener whose `end`
    /// moves, or a clause whose opener lies outside the span) would erase or
    /// introduce its mismatched-indentation warning. Every finding stays; the
    /// whole correction is withheld. Each source warns natively.
    #[test]
    fn partial_negative_shifts_keep_findings_and_withhold_the_whole_edit() {
        for (source, expected) in [
            // The exact reproducer.
            (
                "if true\n          if true\n  1\n  end\nend\n",
                &[(2, 1, 10), (3, 3, -8)][..],
            ),
            (
                "def f\n      while x\n  y\n  end\nend\n",
                &[(2, 1, 6), (3, 3, -4)],
            ),
            ("class A\n      def x\n  end\nend\n", &[(2, 1, 6)]),
            ("class A\n      class << self\n  end\nend\n", &[(2, 1, 6)]),
            (
                "unless a\n      case b\n  when c\n  end\nend\n",
                &[(2, 1, 6)],
            ),
            (
                "if a\n      case b\n        when c\n  end\nend\n",
                &[(2, 1, 6)],
            ),
            ("if a\n      case b\n  in c\n      end\nend\n", &[(2, 1, 6)]),
            ("if a\n      for x in y\n  end\nend\n", &[(2, 1, 6)]),
            (
                "module M\n      begin\n  x\n  end\nend\n",
                &[(2, 1, 6), (3, 3, 0)],
            ),
            (
                "if a\n          until b\n    c\n    end\nend\n",
                &[(2, 1, 10), (3, 5, -6)],
            ),
            (
                "if a\n      if b\n        c\n  elsif d\n        e\n      end\nend\n",
                &[(2, 1, 6), (5, 3, 6)],
            ),
            (
                "if a\n      unless b\n        c\n  else\n        e\n      end\nend\n",
                &[(2, 1, 6), (5, 3, 6)],
            ),
            // The less-indented opener stays while its `end` would move.
            ("if a\n      x = [\n  if y\n      end]\nend\n", &[(2, 1, 6)]),
            // `b` stays, and the clause would move while its `def` stays.
            (
                "def x\n      a\n    rescue\n  b\nend\n",
                &[(2, 1, 6), (4, 3, -2)],
            ),
        ] {
            let found = correction_offenses(&IndentationWidth::default(), source);
            let expected: Vec<_> = expected
                .iter()
                .map(|&(line, col, width)| {
                    (
                        line,
                        col,
                        format!("Use 2 (not {width}) spaces for indentation."),
                    )
                })
                .collect();
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{source:?}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
            assert_correction(&IndentationWidth::default(), source, source);
        }
    }

    /// Complete shifts, positive shifts, and partial negative shifts whose
    /// skipped lines pair no keyword with a moved line stay corrected.
    #[test]
    fn warning_neutral_negative_shifts_stay_corrected() {
        for (before, after) in [
            (
                "if true\n          if true\n            1\n          end\nend\n",
                "if true\n  if true\n    1\n  end\nend\n",
            ),
            (
                "class A\n      def x\n      end\nend\n",
                "class A\n  def x\n  end\nend\n",
            ),
            (
                "def f\n      while x\n\n        y(1,\n          2)\n      end\nend\n",
                "def f\n  while x\n\n    y(1,\n      2)\n  end\nend\n",
            ),
            // Heredoc bodies are literal text, not skipped code lines.
            (
                "def f\n      x(<<~T)\nbody\nT\nend\n",
                "def f\n  x(<<~T)\nbody\nT\nend\n",
            ),
            // Only a plain statement stays; `if` and `end` both move.
            (
                "def f\n      if x\n  y\n      end\nend\n",
                "def f\n  if x\n  y\n  end\nend\n",
            ),
            // Ruby compares no block keywords.
            (
                "foo do\n      bar do\n  end\nend\n",
                "foo do\n  bar do\n  end\nend\n",
            ),
            // The skipped clause and its opener both stay.
            (
                "def x\n      a\n  rescue\n      b\nend\n",
                "def x\n  a\n  rescue\n  b\nend\n",
            ),
            (
                "begin\n      x\n  rescue\n      y\nend\n",
                "begin\n  x\n  rescue\n  y\nend\n",
            ),
            // A complete protected-body shift is not partial: like upstream
            // it moves the clause with the body (its warning is a known gap).
            (
                "def x\n      a\n    rescue\n      b\nend\n",
                "def x\n  a\nrescue\n  b\nend\n",
            ),
        ] {
            assert_correction(&IndentationWidth::default(), before, after);
        }
        for source in [
            "if true\n  if true\n    1\n  end\nend\n",
            "class A\n  def x\n  end\nend\n",
        ] {
            assert_unchanged(source);
        }
    }

    /// A negative shift removes trailing prefix bytes on its first
    /// line and leading prefix bytes on the others. With tabs, native Ruby's
    /// 8-column tab stops then move compared keywords by different amounts
    /// (the exact reproducer moves the inner `if` to column 2 while
    /// its `end` stays at column 8). When the native column order of any
    /// compared pair in the span changes, every finding stays and the whole
    /// edit is withheld. Each pair here is equal before and unequal after, or
    /// the reverse.
    #[test]
    fn tab_shifts_that_change_native_keyword_order_are_withheld() {
        for (source, expected) in [
            // The exact reproducer.
            (
                "if true\n  \tif true\n  \t  1\n  \tend\nend\n",
                &[(2, 1, 3)][..],
            ),
            (
                "if true\n  \tif a\n  \t  1\n  \telse\n  \t  2\n  \tend\nend\n",
                &[(2, 1, 3)],
            ),
            // A stabby lambda's `end` is natively compared with its `->`.
            ("if a\n  \t-> do\n  \t  1\n  \tend\nend\n", &[(2, 1, 3)]),
            ("if a\n  \t-> {\n  \t  1\n  \t}\nend\n", &[(2, 1, 3)]),
            // The same pair guards a space-only partial shift.
            ("if a\n      -> do\n  end\nend\n", &[(2, 1, 6)]),
            ("if a\n  \tcase b\n  \twhen c\n  \tend\nend\n", &[(2, 1, 3)]),
            ("if a\n \t\tif b\n \t\tend\nend\n", &[(2, 1, 3)]),
            // The clause's opener is outside the span: `rescue` would move
            // from column 8 to column 0, erasing its `def` warning.
            ("def f\n  \t x\n \trescue\n  \t y\nend\n", &[(2, 1, 4)]),
            // `end` is compared with the `else`, not the `if` (found by an
            // authored native sweep): only that pair's order would change.
            (
                "if true\n \t if x\n1\n\telse\n 2\n end\nend\n",
                &[(2, 1, 3), (3, 1, -3), (5, 2, 0)],
            ),
            // CRLF line ends and a BOM change neither columns nor the hold.
            (
                "if true\r\n  \tif true\r\n  \t  1\r\n  \tend\r\nend\r\n",
                &[(2, 1, 3)],
            ),
            (
                "\u{feff}if true\n  \tif true\n  \t  1\n  \tend\nend\n",
                &[(2, 1, 3)],
            ),
        ] {
            let found = correction_offenses(&IndentationWidth::default(), source);
            let expected: Vec<_> = expected
                .iter()
                .map(|&(line, col, width)| {
                    (
                        line,
                        col,
                        format!("Use 2 (not {width}) spaces for indentation."),
                    )
                })
                .collect();
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{source:?}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
            assert_correction(&IndentationWidth::default(), source, source);
        }
    }

    /// The pairs follow Ruby 3.4.1's native comparisons, confirmed with
    /// authored `-w -c` controls: `elsif`/`else` with the `if`, `end` and a
    /// rescue `ensure` with a preceding `else`, `when` with `case` (plus the
    /// conservative `case` `else`), lambdas but no blocks.
    #[test]
    fn keyword_pairs_follow_native_ruby() {
        fn lines(source: &str) -> Vec<(u32, u32)> {
            struct Pairs(Vec<(usize, usize)>);
            impl<'pr> NodeVisitor<'pr> for Pairs {
                fn enter(&mut self, node: Node<'pr>) {
                    keyword_pairs(&node, &mut self.0);
                }
                fn leave(&mut self) {}
            }
            let result = ruby_prism::parse(source.as_bytes());
            assert_eq!(result.errors().count(), 0, "{source:?}");
            let mut pairs = Pairs(Vec::new());
            walk(&mut pairs, &result.node());
            let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
            let mut lines: Vec<_> = pairs
                .0
                .iter()
                .map(|&(a, b)| (file.line_of(a), file.line_of(b)))
                .collect();
            lines.sort_unstable();
            lines
        }
        for (source, expected) in [
            (
                "if a\n1\nelsif b\n2\nelsif c\n3\nelse\n4\nend\n",
                &[(1, 3), (1, 5), (1, 7), (7, 9)][..],
            ),
            ("if a\n1\nelsif b\n2\nend\n", &[(1, 3), (1, 5)]),
            ("unless a\n1\nelse\n2\nend\n", &[(1, 3), (3, 5)]),
            (
                "def f\n1\nrescue\n2\nrescue\n3\nelse\n4\nensure\n5\nend\n",
                &[(1, 3), (1, 5), (1, 7), (7, 9), (7, 11)],
            ),
            (
                "begin\n1\nrescue\n2\nensure\n3\nend\n",
                &[(1, 3), (1, 5), (1, 7)],
            ),
            (
                "case a\nwhen 1\n2\nelse\n3\nend\n",
                &[(1, 2), (1, 4), (1, 6)],
            ),
            ("p(-> {\n1\n})\n", &[(1, 3)]),
            ("p(-> do\n1\nrescue\n2\nend)\n", &[(1, 3), (1, 5)]),
            ("foo do\n1\nrescue\n2\nend\nx if y\n", &[]),
        ] {
            assert_eq!(lines(source), expected, "{source:?}");
        }
    }

    /// Tab-containing shifts that keep every compared pair's native column
    /// order stay corrected, as do positive shifts (they append spaces after
    /// the whole prefix, so every tab stop is kept).
    #[test]
    fn tab_shifts_that_keep_native_keyword_order_stay_corrected() {
        for (before, after) in [
            (
                "if true\n  \t  if x\n  \t    1\n  \t  end\nend\n",
                "if true\n  if x\n    1\n  end\nend\n",
            ),
            (
                "if true\r\n  \t  if x\r\n  \t    1\r\n  \t  end\r\nend\r\n",
                "if true\r\n  if x\r\n    1\r\n  end\r\nend\r\n",
            ),
            // `rescue` keeps column 8 against its `def` at column 0.
            (
                "def f\n   \tx\n  \trescue\n   \ty\nend\n",
                "def f\n  x\n\trescue\n \ty\nend\n",
            ),
            ("if a\n\tb\nend\n", "if a\n\t b\nend\n"),
            (
                "if a\n\tif b\n\t\tc\n\tend\nend\n",
                "if a\n\t if b\n\t\t c\n\t end\nend\n",
            ),
        ] {
            assert_correction(&IndentationWidth::default(), before, after);
        }
    }

    #[test]
    fn pinned_mixed_tab_character_columns() {
        assert_correction(
            &IndentationWidth::default(),
            "class A\n  def x\n \t\tbar\n  end\nend\n",
            "class A\n  def x\n \t\t bar\n  end\nend\n",
        );
        assert!(offenses("if a\n \tx\nend\n").is_empty());
    }

    #[test]
    fn pinned_first_statement_not_consistency_or_continuation() {
        let found = offenses("if a\n   b\n     c\nelsif d\n e\nelse\n    f\nend\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 1), (5, 1), (7, 1)]
        );
        assert_eq!(found[0].message, "Use 2 (not 3) spaces for indentation.");
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert!(
            found
                .iter()
                .all(|o| o.severity == crate::offense::Severity::Convention)
        );
        for source in [
            "",
            "if a then b else c end",
            "x if y",
            "x ? y : z",
            "foo(\n      bar,\n      baz\n)\n",
            "x = [\n        1,\n        2\n]\n",
            "x = {\n        k: 1\n}\n",
            "def x\n  foo(\n       1,\n       2)\n        another\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
