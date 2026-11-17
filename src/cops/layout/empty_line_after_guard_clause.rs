//! `Layout/EmptyLineAfterGuardClause` — an empty line follows a guard clause.
//!
//! Port of upstream's `on_if` over the parser AST that RuboCop builds from
//! Prism. Every `if`, `unless`, modifier and ternary (never `elsif` or a
//! pattern guard) whose `if_branch` is one single-line `raise`/`fail` without
//! receiver, `return`, `break` or `next` (the right side of `and`/`or`/`&&`/
//! `||` counts) is a guard clause. Upstream's sibling rules are read from
//! Prism's statements: a sole statement's parser parent is its enclosing node
//! (`def`, block, branch, `rescue`, `ensure`, parentheses...), where the
//! guard is always the last child or an `if` branch next to an `else`, so it
//! is accepted; so is a guard used as a value (a receiver, an argument). In a
//! list of statements (parser `begin`, or `kwbegin` for a keyword `begin`
//! without clauses) the last statement is accepted, as is a guard followed by
//! an `if` whose branch is a guard (of any length), or by a statement starting
//! on its line (`begin` only).
//!
//! A modifier guard whose `if_branch` (its `and` left side, its condition when
//! a heredoc is nested there, or the branch's last parser child) carries a
//! heredoc argument is checked after upstream's `heredoc_line` (the guard's
//! last line plus the body's line count plus one) and reported at the heredoc
//! terminator line; any other guard after its last line, at its `end` or at
//! its start. The next line must be blank, or be an `enable` directive
//! (`rubocop:` or `rustocop:`) or a SimpleCov `:nocov:`/`simplecov:` comment
//! followed by a blank line.
//!
//! The correction inserts a line break after the guard's last line (the
//! heredoc terminator line), or after the allowed directive comment on the
//! next line, with that line's own line ending. It is withheld, finding kept,
//! for the listed source observers (the shared position barrier
//! holds a later `__LINE__`), and when the insertion point is inside a
//! literal, ends a line that a heredoc body continues, or follows a
//! backslash.
use ruby_prism::{Location, Node, StatementsNode};

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct EmptyLineAfterGuardClause;

const MSG: &str = "Add empty line after guard clause.";

impl Cop for EmptyLineAfterGuardClause {
    fn name(&self) -> &'static str {
        "Layout/EmptyLineAfterGuardClause"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_statements_node().is_some()
    }

    /// Upstream's sibling rules need the next statement: each statement list
    /// is visited once and its consecutive pairs are checked, so the work
    /// stays linear in long lists. A sole statement has no pair.
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(statements) = node.as_statements_node() else {
            return;
        };
        // A keyword `begin` without clauses is a parser `kwbegin` holding the
        // statements directly: no same-line exemption there.
        let keyword_begin = cx.parent().is_some_and(|parent| {
            parent.as_begin_node().is_some_and(|begin| {
                begin.begin_keyword_loc().is_some()
                    && begin.rescue_clause().is_none()
                    && begin.else_clause().is_none()
                    && begin.ensure_clause().is_none()
            })
        });
        let root = cx.ancestors().last().unwrap_or(node);
        let mut body = statements.body().iter().peekable();
        while let Some(current) = body.next() {
            let Some(sibling) = body.peek() else {
                break;
            };
            if let Some(offense) = self.check(&current, sibling, keyword_begin, root, cx, file) {
                out.push(offense);
            }
        }
    }
}

impl EmptyLineAfterGuardClause {
    /// `on_if` for a statement followed by `sibling` in a parser `begin` or
    /// `kwbegin`.
    fn check(
        &self,
        node: &Node<'_>,
        sibling: &Node<'_>,
        keyword_begin: bool,
        root: &Node<'_>,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Offense> {
        let conditional = Conditional::of(node)?;
        if !if_branch(conditional.statements.as_ref()).is_some_and(|b| guard_clause(file, &b)) {
            return None;
        }
        if Conditional::of(sibling).is_some_and(|s| {
            if_branch(s.statements.as_ref())
                .is_some_and(|b| guard_clause(file, &b) || guard_branch(&b))
        }) {
            return None;
        }
        let start = node.location().start_offset();
        if !keyword_begin && file.line_of(start) == file.line_of(sibling.location().start_offset())
        {
            return None;
        }
        let last_line = file.last_line_of(node.location());
        let heredoc = conditional
            .modifier
            .then(|| heredoc_argument(node))
            .flatten()
            .and_then(|opening| {
                file.heredocs(root)
                    .iter()
                    .find(|h| h.opening.start == opening)
            });
        let (checked, offense, end_line, target) = if let Some(heredoc) = heredoc {
            let body_lines = file.line_of(heredoc.body.end) - file.line_of(heredoc.body.start);
            let closing = file.line_of(heredoc.closing.start);
            (
                last_line + body_lines + 1,
                heredoc.closing.start,
                closing,
                closing,
            )
        } else if let Some(end) = conditional.end_keyword {
            let line = file.line_of(end.start_offset());
            (last_line, end.start_offset(), line, last_line)
        } else {
            (last_line, start, last_line, last_line)
        };
        if next_line_empty_or_allowed(file, checked) {
            return None;
        }
        let (line, col) = file.line_col(offense);
        Some(Offense {
            correction: cx
                .corrections_requested()
                .then(|| insertion_after_line(file, root, target, true))
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line,
            col,
        })
    }
}

/// A parser `if` node: `if`/`unless` in all forms except `elsif`.
struct Conditional<'pr> {
    statements: Option<StatementsNode<'pr>>,
    end_keyword: Option<Location<'pr>>,
    modifier: bool,
}

impl<'pr> Conditional<'pr> {
    fn of(node: &Node<'pr>) -> Option<Self> {
        if let Some(node) = node.as_if_node() {
            let keyword = node.if_keyword_loc();
            if keyword.as_ref().is_some_and(|k| k.as_slice() == b"elsif") {
                return None;
            }
            Some(Self {
                statements: node.statements(),
                modifier: keyword.is_some() && node.end_keyword_loc().is_none(),
                end_keyword: node.end_keyword_loc(),
            })
        } else {
            let node = node.as_unless_node()?;
            Some(Self {
                statements: node.statements(),
                modifier: node.end_keyword_loc().is_none(),
                end_keyword: node.end_keyword_loc(),
            })
        }
    }
}

/// The parser `if_branch` when it is one node (not an empty or a
/// multi-statement `begin` branch).
fn if_branch<'pr>(statements: Option<&StatementsNode<'pr>>) -> Option<Node<'pr>> {
    let mut body = statements?.body().iter();
    match (body.next(), body.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// `guard_clause?`: the right side of `and`/`or`, or the node itself, is a
/// single-line guard branch.
fn guard_clause(file: &SourceFile, node: &Node<'_>) -> bool {
    let single = |node: &Node<'_>| {
        guard_branch(node)
            && file.line_of(node.location().start_offset()) == file.last_line_of(node.location())
    };
    if let Some(and) = node.as_and_node() {
        single(&and.right())
    } else if let Some(or) = node.as_or_node() {
        single(&or.right())
    } else {
        single(node)
    }
}

/// `(send nil? {:raise :fail} ...)`, `return`, `break` or `next`. A call with a
/// literal block is a parser `block` node.
fn guard_branch(node: &Node<'_>) -> bool {
    match node {
        Node::ReturnNode { .. } | Node::BreakNode { .. } | Node::NextNode { .. } => true,
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            call.receiver().is_none()
                && matches!(call.name().as_slice(), b"raise" | b"fail")
                && call
                    .block()
                    .is_none_or(|b| b.as_block_argument_node().is_some())
        }
        _ => false,
    }
}

/// `next_line_empty_or_allowed_directive_comment?(line)`.
pub(super) fn next_line_empty_or_allowed(file: &SourceFile, line: u32) -> bool {
    next_line_empty(file, line)
        || (allowed_comment(file, line + 1).is_some() && next_line_empty(file, line + 1))
}

/// `processed_source[line].blank?`: the line after `line` holds only
/// whitespace (Ruby's `lstrip` set), or is past the code lines.
pub(super) fn next_line_empty(file: &SourceFile, line: u32) -> bool {
    let next = line + 1;
    // Past the last code line (the `__END__` line or the end of the file)
    // upstream reads `nil`, and a line past the end has no text.
    file.data_start()
        .is_some_and(|data| file.line_start(next) >= data)
        || file
            .line_text(next)
            .iter()
            .all(|&b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r' | b'\n' | b'\0'))
}

/// The comment starting on `line` when it is an `enable`/`enable-next`
/// directive or a SimpleCov comment, as its byte range without line break.
fn allowed_comment(file: &SourceFile, line: u32) -> Option<std::ops::Range<usize>> {
    comment_on_line(file, line).filter(|range| {
        let text = &file.bytes[range.clone()];
        crate::directives::is_enable_directive(text) || simplecov(text)
    })
}

/// RuboCop's `enable_directive_comment?(line)` for the declaration-spacing
/// cops: the comment starting on `line` when it is an `enable`/`enable-next`
/// directive in either spelling (no SimpleCov allowance), as its byte range
/// without line break.
pub(super) fn enable_comment(file: &SourceFile, line: u32) -> Option<std::ops::Range<usize>> {
    comment_on_line(file, line)
        .filter(|range| crate::directives::is_enable_directive(&file.bytes[range.clone()]))
}

/// `processed_source.comment_at_line(line)`: the last comment starting on
/// `line`, as its byte range without line break.
fn comment_on_line(file: &SourceFile, line: u32) -> Option<std::ops::Range<usize>> {
    let start = file.line_start(line);
    let end = file.line_start(line + 1);
    let comments = file.comments();
    let first = comments.partition_point(|c| c.start < start);
    let comment = comments[first..]
        .iter()
        .take_while(|c| c.start < end)
        .last()?;
    let mut range = comment.clone();
    while range.end > range.start && matches!(file.bytes[range.end - 1], b'\n' | b'\r') {
        range.end -= 1;
    }
    Some(range)
}

/// `\A#\s*(?::nocov:|simplecov\s*:\s*(?:disable|enable)\b)`.
fn simplecov(text: &[u8]) -> bool {
    let space = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
    let skip = |t: &[u8]| -> usize { t.iter().take_while(|b| space(b)).count() };
    let Some(rest) = text.strip_prefix(b"#") else {
        return false;
    };
    let rest = &rest[skip(rest)..];
    if rest.starts_with(b":nocov:") {
        return true;
    }
    let Some(rest) = rest.strip_prefix(b"simplecov") else {
        return false;
    };
    let rest = &rest[skip(rest)..];
    let Some(rest) = rest.strip_prefix(b":") else {
        return false;
    };
    let rest = &rest[skip(rest)..];
    [b"disable".as_slice(), b"enable"].iter().any(|mode| {
        rest.strip_prefix(*mode).is_some_and(|after| {
            !after
                .first()
                .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80)
        })
    })
}

/// Upstream's `insert_after(range_by_whole_lines(...), "\n")` after `line`,
/// or after the allowed directive comment on the next line when
/// `directive_aware`: one line break with the line's own ending at the end of
/// its content. `None` when held.
pub(super) fn insertion_after_line(
    file: &SourceFile,
    root: &Node<'_>,
    line: u32,
    directive_aware: bool,
) -> Option<Correction> {
    let comment = directive_aware
        .then(|| allowed_comment(file, line + 1))
        .flatten();
    line_break_after(file, root, line, comment)
}

/// `insert_after(range_by_whole_lines(...), "\n")` after `line`, or after
/// `comment` (a comment on the next line) when given: one line break with the
/// line's own ending at the end of its content. `None` when held for a listed
/// source observer, a literal interior, a heredoc body continuing on the next
/// line or a backslash continuation.
pub(super) fn line_break_after(
    file: &SourceFile,
    root: &Node<'_>,
    line: u32,
    comment: Option<std::ops::Range<usize>>,
) -> Option<Correction> {
    let (line, at) = match comment {
        Some(comment) => (line + 1, comment.end),
        None => (line, file.line_start(line) + file.line_text(line).len()),
    };
    let text = file.line_text(line);
    let next = file.line_start(line + 1);
    let heredocs = file.heredocs(root);
    // A heredoc terminator's line break belongs to its closing token: only a
    // heredoc body around the terminator line makes the end of it literal.
    let inside = if heredocs
        .iter()
        .any(|h| h.closing.start <= at && at < h.closing.end)
    {
        heredocs
            .iter()
            .any(|h| h.body.start <= at && at < h.body.end)
    } else {
        file.literal_interior_at(root, at)
    };
    if text.last() == Some(&b'\\')
        || inside
        || heredocs
            .iter()
            .any(|h| h.body.start == next && h.opening.start < at)
    {
        return None;
    }
    let newline = if file.bytes[..next].ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    Some(Correction {
        range: at..at,
        replacement: newline.into(),
    })
}

/// Upstream's `last_heredoc_argument(node)`, as the heredoc's opening offset.
fn heredoc_argument(node: &Node<'_>) -> Option<usize> {
    if node.as_if_node().is_some() || node.as_unless_node().is_some() {
        heredoc_search(&last_heredoc_argument_node(node)?)
    } else {
        heredoc_search(node)
    }
}

/// The rest of `last_heredoc_argument` after `last_heredoc_argument_node`.
fn heredoc_search(n: &Node<'_>) -> Option<usize> {
    if let Some(parentheses) = n.as_parentheses_node() {
        let body = parentheses.body()?;
        return match body.as_statements_node() {
            Some(statements) => heredoc_search(&statements.body().iter().next()?),
            None => heredoc_search(&body),
        };
    }
    if is_heredoc(n) {
        return Some(n.location().start_offset());
    }
    let (arguments, block, receiver) = match n {
        Node::CallNode { .. } => {
            let call = n.as_call_node().unwrap();
            (call.arguments(), call.block(), call.receiver())
        }
        Node::SuperNode { .. } => {
            let call = n.as_super_node().unwrap();
            (call.arguments(), call.block(), None)
        }
        Node::YieldNode { .. } => (n.as_yield_node().unwrap().arguments(), None, None),
        Node::ReturnNode { .. } => (n.as_return_node().unwrap().arguments(), None, None),
        Node::BreakNode { .. } => (n.as_break_node().unwrap().arguments(), None, None),
        Node::NextNode { .. } => (n.as_next_node().unwrap().arguments(), None, None),
        _ => return None,
    };
    // A call with a literal block is a parser `block` node: its arguments are
    // the block parameters and it has no receiver.
    if block.as_ref().is_some_and(|b| b.as_block_node().is_some()) {
        return None;
    }
    arguments
        .iter()
        .flat_map(|a| a.arguments().iter())
        .chain(block)
        .find_map(|argument| heredoc_argument(&argument))
        .or_else(|| receiver.and_then(|r| heredoc_argument(&r)))
}

/// Upstream's `last_heredoc_argument_node(node)` for an `if` node.
fn last_heredoc_argument_node<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let (statements, predicate) = if let Some(n) = node.as_if_node() {
        (n.statements(), n.predicate())
    } else {
        let n = node.as_unless_node()?;
        (n.statements(), n.predicate())
    };
    let branch: Vec<Node<'pr>> = statements?.body().iter().collect();
    if let [only] = branch.as_slice()
        && let Some(and) = only.as_and_node()
    {
        return Some(and.left());
    }
    if has_heredoc_descendant(&predicate) {
        return Some(predicate);
    }
    match branch.as_slice() {
        [only] => last_child(only),
        // A multi-statement branch is a parser `begin`: its last statement.
        [..] => branch.into_iter().last(),
    }
}

/// The parser `children.last` of a guard branch.
fn last_child<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let arguments = match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            if let Some(block) = call.block() {
                return block.as_block_argument_node().map(|b| b.as_node());
            }
            call.arguments()
        }
        Node::ReturnNode { .. } => node.as_return_node().unwrap().arguments(),
        Node::BreakNode { .. } => node.as_break_node().unwrap().arguments(),
        Node::NextNode { .. } => node.as_next_node().unwrap().arguments(),
        Node::OrNode { .. } => return Some(node.as_or_node().unwrap().right()),
        Node::AndNode { .. } => return Some(node.as_and_node().unwrap().right()),
        _ => return None,
    };
    arguments?.arguments().iter().last()
}

/// RuboCop's `heredoc?`: a string or command literal opened with `<<`.
fn is_heredoc(node: &Node<'_>) -> bool {
    let opening = match node {
        Node::StringNode { .. } => node.as_string_node().unwrap().opening_loc(),
        Node::InterpolatedStringNode { .. } => {
            node.as_interpolated_string_node().unwrap().opening_loc()
        }
        Node::XStringNode { .. } => Some(node.as_x_string_node().unwrap().opening_loc()),
        Node::InterpolatedXStringNode { .. } => {
            Some(node.as_interpolated_x_string_node().unwrap().opening_loc())
        }
        _ => None,
    };
    opening.is_some_and(|o| o.as_slice().starts_with(b"<<"))
}

/// `condition.descendants.any?(&:heredoc?)`.
fn has_heredoc_descendant(node: &Node<'_>) -> bool {
    struct Find(bool, usize);
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Find {
        fn enter(&mut self, node: Node<'pr>) {
            self.1 += 1;
            // The root itself is not its own descendant.
            self.0 |= self.1 > 1 && is_heredoc(&node);
        }
    }
    let mut find = Find(false, 0);
    crate::cops::walk::walk(&mut find, node);
    find.0
}

#[cfg(test)]
mod tests;
