//! A narrow port of RuboCop's `MultilineLiteralBraceLayout` mixin and its
//! `MultilineLiteralBraceCorrector` (97f30c4, MIT) at the fixed
//! `EnforcedStyle: symmetrical`, shared by the array and hash brace layout
//! cops.
//!
//! `check_brace_layout`: implicit (no opening brace), empty and single-line
//! literals are ignored, as are literals whose last child holds a heredoc
//! ending on or after that child's last line (`last_line_heredoc?`) and
//! literals with a heredoc opened on the closing brace's line. When the
//! opening brace shares a line with the first child, the closing brace must
//! share one with the last child, and when it does not, the closing brace
//! must not either. The closing brace is reported.
//!
//! The correction is the corrector's canonical edit, expressed as one
//! contiguous replacement: a `"\n"` inserted before a closing brace on the
//! last child's line, or the closing brace moved to the end of the last
//! child (after its trailing comma) when it sits on a later line, together
//! with the rest of its line when the last child's line ends in a comment.
//! Upstream's holds are kept (`new_line_needed_before_closing_brace?`,
//! `nested_closing_bracket_moved_before_comment?`). The replacement starts at
//! the last child's last character, so a nested literal's brace edit (which
//! ends there) always overlaps it: one `--fix` pass applies one of the two
//! and the next run the other, instead of interleaving them, which is what
//! upstream's `autocorrect_incompatible_with` pass ordering ensures. The edit
//! is also withheld for the listed source observers, when anything
//! but blanks and real comments lies between the last child and the brace,
//! when a comment between them would end up before code, and when the moved
//! rest of the line holds a comment or a heredoc opener or ends inside a
//! literal. Line-number observers and heredoc boundaries are held by the
//! shared `run_cops` guards.
//!
//! Reuses the projection for method call arguments and method
//! definition parameters (`List::Call`, `List::Parameters`). Their
//! `each_descendant` subtrees are explicit (`Literal::scan`: a `send` without
//! the literal block that wraps it, a definition's parameters without its
//! body); parameters are never `chained?` or `argument?`; and a call wrapped
//! by a literal block is the child of that `block`, never of another call.
//! For these lists the gap between the last child and the closing
//! parenthesis may also hold the bodies of heredocs opened in the list, as
//! upstream's edit allows, and a call's edit includes upstream's
//! `correct_heredoc_argument_method_chain` move. Whenever the gap is not
//! blank, what stays behind on the parenthesis's line must be blank, so
//! nothing is glued to a heredoc terminator or a comment.
use std::ops::Range;

use ruby_prism::Node;

use super::heredoc::{CallType, Role, call_parent, call_type, has_literal_block};
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The two symmetrical-style messages of a brace layout cop.
pub(crate) struct Messages {
    /// `SAME_LINE_MESSAGE`: the opening brace shares the first child's line.
    pub same_line: &'static str,
    /// `NEW_LINE_MESSAGE`: the opening brace is on its own line.
    pub new_line: &'static str,
}

/// A braced literal as the mixin sees it.
pub(crate) struct Literal<'a, 'pr> {
    pub node: &'a Node<'pr>,
    /// `node.loc.begin`'s start; `None` for an implicit literal.
    pub opening: Option<usize>,
    /// `node.loc.end`.
    pub closing: Option<Range<usize>>,
    /// `children(node)`, in source order.
    pub children: &'a [Node<'pr>],
    /// The closing location of a node of the same kind as `node`
    /// (`same_kind_as_node?`), `None` for any other node.
    pub same_kind_closing: fn(&Node<'pr>) -> Option<Range<usize>>,
    /// The subtrees `heredoc_opened_on_closing_brace_line?` searches (the
    /// parser node's descendants).
    pub scan: &'a [Node<'pr>],
    pub list: List,
}

/// What the braces delimit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum List {
    /// An array or hash literal.
    Literal,
    /// A `send`/`csend`'s parenthesized arguments; `Literal::node` is the
    /// call.
    Call,
    /// A definition's parenthesized parameters (`args`), which are never
    /// `chained?` or `argument?`.
    Parameters,
}

/// `check_brace_layout` at the fixed symmetrical style.
pub(crate) fn check_brace_layout<'pr>(
    cop: &dyn Cop,
    messages: &Messages,
    literal: &Literal<'_, 'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    // `implicit_literal?` and `empty_literal?`.
    let (Some(opening), Some(closing), Some(first), Some(last)) = (
        literal.opening,
        literal.closing.clone(),
        literal.children.first(),
        literal.children.last(),
    ) else {
        return;
    };
    let Some(root) = cx.ancestors().next_back() else {
        return;
    };
    let location = literal.node.location();
    // `node.single_line?`.
    if file.line_of(location.start_offset()) == file.last_line_of(location) {
        return;
    }
    let closing_line = file.line_of(closing.start);
    let last_line = file.last_line_of(last.location());
    if last_line_heredoc(file, root, last, last_line)
        || literal
            .scan
            .iter()
            .any(|node| heredoc_opened_on_line(file, root, node, closing_line))
    {
        return;
    }
    let opening_same = file.line_of(opening) == file.line_of(first.location().start_offset());
    let closing_same = closing_line == last_line;
    let message = match (opening_same, closing_same) {
        (true, false) => messages.same_line,
        (false, true) => messages.new_line,
        _ => return,
    };
    let correction = cx
        .corrections_requested()
        .then(|| correction(literal, cx, file, root, closing_same))
        .flatten();
    let (line, col) = file.line_col(closing.start);
    out.push(Offense {
        correction,
        cop: cop.name(),
        severity: cop.severity(),
        message: message.to_owned(),
        line,
        end_line: closing_line,
        col,
    });
}

/// The heredoc strings of a subtree (parser `str`/`dstr`/`xstr` nodes with a
/// heredoc map): `(opening start, terminator start, terminator end)` of each.
#[derive(Default)]
struct Heredocs(Vec<(usize, usize, usize)>);

impl<'pr> NodeVisitor<'pr> for Heredocs {
    fn enter(&mut self, node: Node<'pr>) {
        let (opening, closing) = match &node {
            Node::StringNode { .. } => {
                let n = node.as_string_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            Node::InterpolatedStringNode { .. } => {
                let n = node.as_interpolated_string_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            Node::XStringNode { .. } => {
                let n = node.as_x_string_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            Node::InterpolatedXStringNode { .. } => {
                let n = node.as_interpolated_x_string_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            _ => return,
        };
        if let (Some(opening), Some(closing)) = (opening, closing)
            && opening.as_slice().starts_with(b"<<")
        {
            self.0.push((
                opening.start_offset(),
                closing.start_offset(),
                closing.end_offset(),
            ));
        }
    }
}

/// Walk a subtree for its heredocs, only when the shared exact main-parse
/// index says it has one.
fn heredocs(file: &SourceFile, root: &Node<'_>, node: &Node<'_>) -> Vec<(usize, usize, usize)> {
    if !file.contains_heredoc(root, node) {
        return Vec::new();
    }
    let mut found = Heredocs::default();
    walk(&mut found, node);
    found.0
}

/// `last_line_heredoc?`: a heredoc in the last child whose terminator is on
/// or after the child's last line.
fn last_line_heredoc(file: &SourceFile, root: &Node<'_>, last: &Node<'_>, last_line: u32) -> bool {
    heredocs(file, root, last)
        .iter()
        .any(|&(_, closing, _)| file.line_of(closing) >= last_line)
}

/// `heredoc_opened_on_closing_brace_line?`: a heredoc in the literal opened
/// on the closing brace's line.
fn heredoc_opened_on_line(file: &SourceFile, root: &Node<'_>, node: &Node<'_>, line: u32) -> bool {
    heredocs(file, root, node)
        .iter()
        .any(|&(opening, _, _)| file.line_of(opening) == line)
}

/// `chained? || argument?`: the node is a `send`/`csend` receiver or a
/// `send` argument in the parser AST. `ancestors` are innermost first. A
/// call with a literal block hangs from its `block` node instead.
fn chained_or_argument<'pr>(node: &Node<'pr>, ancestors: &[&Node<'pr>]) -> bool {
    !has_literal_block(node)
        && call_parent(node, ancestors).is_some_and(|(_, kind, role)| {
            role == Role::Receiver || kind == CallType::Send && role == Role::Argument
        })
}

/// Whether a comment starts on `line` (`processed_source.comment_at_line`).
fn comment_at_line(file: &SourceFile, line: u32) -> bool {
    line > 0 && file.has_comment_in_lines(line..line + 1)
}

/// The corrector's canonical edit, or `None` when it is withheld.
fn correction<'pr>(
    literal: &Literal<'_, 'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
    root: &Node<'pr>,
    closing_same: bool,
) -> Option<Correction> {
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
    {
        return None;
    }
    let brace = literal.closing.clone()?;
    // `correct_same_line_brace`.
    if closing_same {
        return Some(Correction {
            range: brace.start..brace.start,
            replacement: "\n".to_owned(),
        });
    }
    let last = literal.children.last()?;
    let bytes = &file.bytes;
    let element_end = last.location().end_offset();
    // `last_element_range_with_trailing_comma(node).end`.
    let end = trailing_comma_end(bytes, element_end).unwrap_or(element_end);
    // `new_line_needed_before_closing_brace?`.
    let ancestors: Vec<_> = cx.ancestors().take(2).collect();
    let chainable = literal.list != List::Parameters;
    if comment_at_line(file, file.line_of(end - 1))
        && chainable
        && chained_or_argument(literal.node, &ancestors)
    {
        return None;
    }
    if nested_closing_bracket_moved_before_comment(literal, file) {
        return None;
    }
    // `range_with_surrounding_space(node.loc.end, side: :left).begin`.
    let gap_end = blank_start(bytes, brace.start);
    if gap_end < end {
        return None;
    }
    let bodies = if literal.list == List::Literal {
        Vec::new()
    } else {
        heredoc_bodies(file, root, literal.children)
    };
    gap_contents(file, end..gap_end, &bodies)?;
    let line_end = {
        let line = file.line_of(brace.start);
        file.line_start(line) + file.line_text(line).len()
    };
    // `content_if_comment_present`: the rest of the brace's line moves too.
    let moved_end = if comment_at_line(file, file.last_line_of(last.location())) {
        let tail = brace.end..line_end;
        if comment_in(file, &tail)
            || file.heredoc_opening_in(root, tail)
            || file.overlaps_literal(root, &(line_end..line_end + 1))
        {
            return None;
        }
        line_end
    } else {
        brace.end
    };
    // `correct_heredoc_argument_method_chain`: the parent call's dot and
    // selector follow the parenthesis.
    let chain = match literal.list {
        List::Call => heredoc_argument_method_chain(literal, cx, file, root, brace.end)?,
        _ => None,
    };
    let edit_end = chain.as_ref().map_or(moved_end, |chain| chain.end);
    // The rest of the parenthesis's line would follow the gap's last comment
    // or heredoc terminator.
    if gap_end > end && bytes[edit_end..line_end].iter().any(|b| !is_blank(*b)) {
        return None;
    }
    // Start at the last child's last character, so that a nested literal's
    // edit ending there conflicts with this one instead of composing.
    let mut start = element_end - 1;
    while start > last.location().start_offset() && bytes[start] & 0xc0 == 0x80 {
        start -= 1;
    }
    let mut replacement = Vec::with_capacity(edit_end - start);
    replacement.extend_from_slice(&bytes[start..end]);
    replacement.extend_from_slice(&bytes[brace.start..moved_end]);
    if let Some(chain) = chain {
        replacement.extend_from_slice(&bytes[chain]);
    }
    replacement.extend_from_slice(&bytes[end..gap_end]);
    Some(Correction {
        range: start..edit_end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

/// `correct_heredoc_argument_method_chain`: when the call's first argument
/// is a heredoc parser `str` and the call is the receiver of another
/// `send`/`csend`, that call's dot and selector (`parent.loc.dot` to the end
/// of the parent `send`) move behind the parenthesis. `Some(None)` when
/// upstream would not move anything; `None` (edit withheld) where upstream's
/// move raises (no dot, or the call is an argument) or would not be a
/// same-line, adjacent, plain chain (a literal block, a comment, a heredoc
/// opener or a line break in it).
fn heredoc_argument_method_chain<'pr>(
    literal: &Literal<'_, 'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
    root: &Node<'pr>,
    closing_end: usize,
) -> Option<Option<Range<usize>>> {
    let Some(parent) = cx.parent() else {
        return Some(None);
    };
    if call_type(parent).is_none()
        || !literal
            .children
            .first()
            .is_some_and(|first| str_heredoc(file, first))
    {
        return Some(None);
    }
    let parent_call = parent.as_call_node()?;
    let receiver = parent_call.receiver()?;
    let location = literal.node.location();
    if receiver.location().start_offset() != location.start_offset()
        || receiver.location().end_offset() != location.end_offset()
        || parent_call
            .block()
            .is_some_and(|b| b.as_block_node().is_some())
    {
        return None;
    }
    let dot = parent_call.call_operator_loc()?.start_offset();
    let end = parent.location().end_offset();
    if dot != closing_end
        || file.line_of(end) != file.line_of(dot)
        || comment_in(file, &(dot..end))
        || file.heredoc_opening_in(root, dot..end)
    {
        return None;
    }
    Some(Some(dot..end))
}

/// Whether a node is a heredoc that the parser translation emits as a `str`:
/// a heredoc `StringNode` whose body is exactly one line (an empty or
/// multi-line body is a `dstr`).
fn str_heredoc(file: &SourceFile, node: &Node<'_>) -> bool {
    let Some(string) = node.as_string_node() else {
        return false;
    };
    let (Some(opening), Some(closing)) = (string.opening_loc(), string.closing_loc()) else {
        return false;
    };
    opening.as_slice().starts_with(b"<<")
        && file.line_of(closing.start_offset()) == file.line_of(opening.start_offset()) + 2
}

/// The bodies (through their terminators) of the heredocs opened in
/// `children`, as byte ranges starting at the line after each opener.
fn heredoc_bodies(file: &SourceFile, root: &Node<'_>, children: &[Node<'_>]) -> Vec<Range<usize>> {
    children
        .iter()
        .flat_map(|child| heredocs(file, root, child))
        .map(|(opening, _, closing_end)| file.line_start(file.line_of(opening) + 1)..closing_end)
        .collect()
}

/// `nested_closing_bracket_moved_before_comment?`: a comment starts on the
/// line before the last child's last line, and the last child (or the first
/// node below it, in pre-order) of the literal's kind closes with `}`, `]` or
/// `)` first on that last line; the edit is withheld when that comment is on
/// its own line or the nested node is not a receiver or a `send` argument.
fn nested_closing_bracket_moved_before_comment(
    literal: &Literal<'_, '_>,
    file: &SourceFile,
) -> bool {
    let Some(last) = literal.children.last() else {
        return false;
    };
    let last_line = file.last_line_of(last.location());
    if !comment_at_line(file, last_line - 1) {
        return false;
    }
    let mut finder = NestedFinder {
        file,
        line: last_line,
        same_kind_closing: literal.same_kind_closing,
        chainable: literal.list != List::Parameters,
        last_is_argument: literal.list == List::Call
            && call_type(literal.node) == Some(CallType::Send),
        stack: Vec::new(),
        found: None,
    };
    walk(&mut finder, last);
    let Some(chained_or_argument) = finder.found else {
        return false;
    };
    // `own_line_comment?`: the comment's line starts with it (`lstrip`).
    let own_line = file
        .line_text(last_line - 1)
        .iter()
        .find(|b| !is_ruby_space(**b) && **b != 0)
        == Some(&b'#');
    own_line || !chained_or_argument
}

/// Pre-order search for `nested_literal_with_closing_bracket_on_own_line`;
/// records whether the found node is `chained? || argument?`.
struct NestedFinder<'f, 'pr> {
    file: &'f SourceFile,
    line: u32,
    same_kind_closing: fn(&Node<'pr>) -> Option<Range<usize>>,
    /// Parameter lists (`args`) are never `chained?` or `argument?`.
    chainable: bool,
    /// Whether the last child itself is `argument?`: the last argument of a
    /// `send` (never of a `csend`, an array or a hash).
    last_is_argument: bool,
    stack: Vec<Node<'pr>>,
    found: Option<bool>,
}

impl<'pr> NodeVisitor<'pr> for NestedFinder<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if self.found.is_none()
            && let Some(closing) = (self.same_kind_closing)(&node)
            && matches!(&self.file.bytes[closing.clone()], b"}" | b"]" | b")")
            && self.file.line_of(closing.start) == self.line
        {
            let start = self.file.line_start(self.line);
            let prefix = &self.file.bytes[start..closing.start];
            if prefix.iter().all(|b| is_ruby_space(*b) || *b == 0) {
                let ancestors: Vec<_> = self.stack.iter().rev().collect();
                self.found = Some(if ancestors.is_empty() {
                    // The last child itself hangs from the literal.
                    self.last_is_argument
                } else {
                    self.chainable && chained_or_argument(&node, &ancestors)
                });
            }
        }
        self.stack.push(node);
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

/// Ruby's `/\s/`: space, `\t`, `\n`, `\v`, `\f`, `\r`.
fn is_ruby_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

/// Space or tab.
fn is_blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

/// The end of a trailing comma after `end`: the byte after spaces and tabs,
/// then line breaks (`range_with_surrounding_space(side: :right)`), when it
/// is `,`.
fn trailing_comma_end(bytes: &[u8], end: usize) -> Option<usize> {
    let mut pos = end;
    while matches!(bytes.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    loop {
        match (bytes.get(pos), bytes.get(pos + 1)) {
            (Some(b'\n'), _) => pos += 1,
            (Some(b'\r'), Some(b'\n')) => pos += 2,
            _ => break,
        }
    }
    (bytes.get(pos) == Some(&b',')).then_some(pos + 1)
}

/// The start of the spaces and tabs, then line breaks, before `start`
/// (`range_with_surrounding_space(side: :left)`); `\r\n` is one break, as in
/// RuboCop's normalized buffer.
fn blank_start(bytes: &[u8], start: usize) -> usize {
    let mut pos = start;
    while pos > 0 && matches!(bytes[pos - 1], b' ' | b'\t') {
        pos -= 1;
    }
    while pos > 0 && bytes[pos - 1] == b'\n' {
        pos -= 1;
        if pos > 0 && bytes[pos - 1] == b'\r' {
            pos -= 1;
        }
    }
    pos
}

/// Whether `gap` holds only whitespace, real comments and the given heredoc
/// `bodies`; `None` when it holds anything else (a line continuation, a
/// separator, another heredoc's body).
fn gap_contents(file: &SourceFile, gap: Range<usize>, bodies: &[Range<usize>]) -> Option<()> {
    let comments = file.comments();
    let mut pos = gap.start;
    while pos < gap.end {
        if is_ruby_space(file.bytes[pos]) {
            pos += 1;
            continue;
        }
        if let Some(body) = bodies.iter().find(|b| b.contains(&pos)) {
            pos = body.end;
            continue;
        }
        let index = comments.partition_point(|c| c.start < pos);
        let comment = comments.get(index).filter(|c| c.start == pos)?;
        pos = comment.end.max(pos + 1);
    }
    Some(())
}

/// Whether a comment starts in `range`.
fn comment_in(file: &SourceFile, range: &Range<usize>) -> bool {
    let comments = file.comments();
    let index = comments.partition_point(|c| c.start < range.start);
    comments.get(index).is_some_and(|c| c.start < range.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_scans_match_range_with_surrounding_space() {
        assert_eq!(trailing_comma_end(b"b ,", 1), Some(3));
        assert_eq!(trailing_comma_end(b"b\n\n,", 1), Some(4));
        assert_eq!(trailing_comma_end(b"b\r\n,", 1), Some(4));
        // Spaces after a line break are not skipped.
        assert_eq!(trailing_comma_end(b"b\n ,", 1), None);
        assert_eq!(trailing_comma_end(b"b # ,", 1), None);
        assert_eq!(trailing_comma_end(b"b", 1), None);
        assert_eq!(blank_start(b"b\n  ]", 4), 1);
        assert_eq!(blank_start(b"b \n\n]", 4), 2);
        assert_eq!(blank_start(b"b\r\n\r\n]", 5), 1);
        assert_eq!(blank_start(b"b\n \n]", 4), 3);
        assert_eq!(blank_start(b"]", 0), 0);
    }
}
