//! `Layout/IndentationWidth` — two spaces for bodies, not continuation alignment.

use std::{collections::BTreeMap, ops::Range};

use ruby_prism::{Location, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct IndentationWidth;

impl Cop for IndentationWidth {
    fn name(&self) -> &'static str {
        "Layout/IndentationWidth"
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
        checks(node, cx, file, &mut |check| {
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
            ancestors: Vec<Node<'pr>>,
            checks: Vec<Check>,
            interiors: Vec<Range<usize>>,
        }
        impl<'pr> NodeVisitor<'pr> for Plans<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                let cx = NodeContext {
                    ancestors: &self.ancestors,
                    corrections_requested: true,
                    require_order_reports: &Default::default(),
                };
                checks(&node, &cx, self.file, &mut |c| self.checks.push(c));
                if let Some(range) = literal_interior(&node, self.file) {
                    self.interiors.push(range);
                }
                self.ancestors.push(node);
            }
            fn leave(&mut self) {
                self.ancestors.pop();
            }
        }
        let mut plans = Plans {
            file,
            ancestors: Vec::new(),
            checks: Vec::new(),
            interiors: Vec::new(),
        };
        walk(&mut plans, node);
        plans
            .checks
            .sort_unstable_by_key(|c| (c.expression.start, std::cmp::Reverse(c.expression.end)));
        plans.interiors.sort_unstable_by_key(|r| (r.start, r.end));
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
            if let Some(fix) = correction(file, node, &check, &interiors) {
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

fn bare_access<'a, 'pr: 'a>(
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
fn in_macro_scope<'a, 'pr: 'a>(
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
    emit: &mut impl FnMut(Check),
) {
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
    let mut check = |base: Location<'pr>, body_node: Option<Node<'pr>>| {
        // A multi-statement body starting with an access modifier belongs to
        // the access-modifier/consistency cops, except class/module members.
        let skip = body_node
            .as_ref()
            .is_some_and(|b| starts_with_access(b, node, cx));
        if !skip {
            check_range(base.start_offset(), body_node.as_ref().and_then(body));
        }
    };
    match node {
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            if let Some(keyword) = n.if_keyword_loc()
                && keyword.start_offset() == n.location().start_offset()
            {
                check(keyword, n.statements().map(|s| s.as_node()));
            }
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node().unwrap();
            if n.keyword_loc().start_offset() == n.location().start_offset() {
                check(n.keyword_loc(), n.statements().map(|s| s.as_node()));
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
                check(base, n.statements().map(|s| s.as_node()));
            }
        }
        Node::WhileNode { .. } => {
            let n = node.as_while_node().unwrap();
            if n.keyword_loc().start_offset() == n.location().start_offset()
                && file.line_of(n.predicate().location().start_offset())
                    == file.last_line_of(n.predicate().location())
            {
                check(n.keyword_loc(), n.statements().map(|s| s.as_node()));
            }
        }
        Node::UntilNode { .. } => {
            let n = node.as_until_node().unwrap();
            if n.keyword_loc().start_offset() == n.location().start_offset()
                && file.line_of(n.predicate().location().start_offset())
                    == file.last_line_of(n.predicate().location())
            {
                check(n.keyword_loc(), n.statements().map(|s| s.as_node()));
            }
        }
        Node::ForNode { .. } => {
            let n = node.as_for_node().unwrap();
            check(n.for_keyword_loc(), n.statements().map(|s| s.as_node()));
        }
        Node::WhenNode { .. } => {
            let n = node.as_when_node().unwrap();
            check(n.keyword_loc(), n.statements().map(|s| s.as_node()));
        }
        Node::InNode { .. } => {
            let n = node.as_in_node().unwrap();
            check(n.in_loc(), n.statements().map(|s| s.as_node()));
        }
        Node::RescueNode { .. } => {
            let n = node.as_rescue_node().unwrap();
            check(n.keyword_loc(), n.statements().map(|s| s.as_node()));
        }
        Node::EnsureNode { .. } => {
            let n = node.as_ensure_node().unwrap();
            check(n.ensure_keyword_loc(), n.statements().map(|s| s.as_node()));
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
                check(n.closing_loc(), n.body());
            }
        }
        Node::LambdaNode { .. } => {
            let n = node.as_lambda_node().unwrap();
            if starts_line(file, n.closing_loc().start_offset()) {
                check(n.closing_loc(), n.body());
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

/// Protect whole literal interiors, including interpolation, when shifting a
/// containing subtree. This correction-only conservatism also handles empty
/// indentation at heredoc body boundaries; a zero-width overlap cannot.
fn literal_interior(node: &Node<'_>, file: &SourceFile) -> Option<Range<usize>> {
    let (opening, closing) = match node {
        Node::StringNode { .. } => {
            let n = node.as_string_node()?;
            (n.opening_loc()?, n.closing_loc()?)
        }
        Node::InterpolatedStringNode { .. } => {
            let n = node.as_interpolated_string_node()?;
            (n.opening_loc()?, n.closing_loc()?)
        }
        Node::SymbolNode { .. } => {
            let n = node.as_symbol_node()?;
            (n.opening_loc()?, n.closing_loc()?)
        }
        Node::InterpolatedSymbolNode { .. } => {
            let n = node.as_interpolated_symbol_node()?;
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
        Node::RegularExpressionNode { .. } => {
            let n = node.as_regular_expression_node()?;
            (n.opening_loc(), n.closing_loc())
        }
        Node::InterpolatedRegularExpressionNode { .. } => {
            let n = node.as_interpolated_regular_expression_node()?;
            (n.opening_loc(), n.closing_loc())
        }
        Node::ArrayNode { .. } => {
            let n = node.as_array_node()?;
            let opening = n.opening_loc()?;
            if !opening.as_slice().starts_with(b"%") {
                return None;
            }
            (opening, n.closing_loc()?)
        }
        _ => return None,
    };
    let start = if opening.as_slice().starts_with(b"<<") {
        file.line_start(file.line_of(opening.end_offset()) + 1)
    } else {
        opening.end_offset()
    };
    Some(start..closing.end_offset())
}

fn correction(
    file: &SourceFile,
    root: &Node<'_>,
    check: &Check,
    interiors: &[Range<usize>],
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
            continue;
        }
        replacement.extend_from_slice(&file.bytes[cursor..range.start]);
        if delta > 0 {
            replacement.extend_from_slice(&file.bytes[range.clone()]);
            replacement.extend(std::iter::repeat_n(b' ', delta as usize));
        } else if line == first {
            replacement.extend_from_slice(&file.bytes[range.start..range.end - (-delta) as usize]);
        } else {
            replacement.extend_from_slice(&file.bytes[range.start + (-delta) as usize..range.end]);
        }
        cursor = range.end;
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
            assert_correction(&IndentationWidth, before, after);
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
                &IndentationWidth,
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
            &IndentationWidth,
            before,
            "class A\n  private\n\n  def x\n  end\n\n      public\n  def y\n  end\nend\n",
        );
    }

    fn assert_unchanged(source: &str) {
        assert!(offenses(source).is_empty(), "{source}");
        let found = correction_offenses(&IndentationWidth, source);
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
            assert_correction(&IndentationWidth, before, after);
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
            assert_correction(&IndentationWidth, before, after);
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
            assert_correction(&IndentationWidth, &before, &after);
        }
        let before = "\u{feff}module M日本語\r\n    def élève\r\n      \"é\"\r\n    end\r\nend\r\n__END__\r\n  raw\r\n";
        let after = "\u{feff}module M日本語\r\n  def élève\r\n    \"é\"\r\n  end\r\nend\r\n__END__\r\n  raw\r\n";
        assert_correction(&IndentationWidth, before, after);
        assert!(offenses(after).is_empty());
        assert_correction(&IndentationWidth, "def x\n    1\nend", "def x\n  1\nend");
    }

    #[test]
    fn pinned_heredoc_and_block_comment_safety() {
        let before = "module Foo\nmodule Bar\n  SOMETHING = <<GOO\ntext\nmore text\nfoo\nGOO\n  def baz\n    do_something(\"#{x}\")\n  end\nend\nend\n";
        let after = "module Foo\n  module Bar\n    SOMETHING = <<GOO\ntext\nmore text\nfoo\nGOO\n    def baz\n      do_something(\"#{x}\")\n    end\n  end\nend\n";
        assert_correction(&IndentationWidth, before, after);
        let before =
            "module Foo\nclass Bar\n=begin\ncomment\n=end\ndef baz\ndo_something\nend\nend\nend\n";
        assert_eq!(offenses(before).len(), 3);
        assert!(
            correction_offenses(&IndentationWidth, before)
                .iter()
                .all(|o| o.correction.is_none())
        );
        assert_correction(&IndentationWidth, before, before);
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
                    &IndentationWidth,
                    &before,
                    &before.replace("\n w\n", "\n  w\n"),
                );
            }
            let before = format!(
                "module A\nclass B\n  # {prefix}:disable Layout/IndentationWidth\n  def x\n  end\n  # {prefix}:enable Layout/IndentationWidth\nend\nend\n"
            );
            let found = correction_offenses(&IndentationWidth, &before);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{before}");
            assert_correction(&IndentationWidth, &before, &before);
        }
    }

    #[test]
    fn nested_and_wide_correction_work_selects_disjoint_spans() {
        // Stay within the shared AST-depth budget (two nodes per module).
        let nested = format!("{}x\n{}", "module A\n".repeat(30), "end\n".repeat(30));
        let found = correction_offenses(&IndentationWidth, &nested);
        assert_eq!(found.len(), 30);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
        let wide = "class A\n    def x\n    end\n    def y\n    end\nend\n".repeat(2000);
        let found = correction_offenses(&IndentationWidth, &wide);
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
            assert_correction(&IndentationWidth, source, source);
        }
        // A containing shift preserves both literal text and interpolation.
        let before = "class A\nclass B\n  X = \"a\n#{\nif true\n 1\nend\n}tail\"\nend\nend\n";
        let after = "class A\n  class B\n    X = \"a\n#{\nif true\n 1\nend\n}tail\"\n  end\nend\n";
        assert_correction(&IndentationWidth, before, after);
    }

    #[test]
    fn subtree_shifts_do_not_create_a_data_marker() {
        assert_correction(
            &IndentationWidth,
            "class A\n    def x\n  __END__\n    end\nend\n",
            "class A\n  def x\n  __END__\n  end\nend\n",
        );
    }

    #[test]
    fn pinned_mixed_tab_character_columns() {
        assert_correction(
            &IndentationWidth,
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
