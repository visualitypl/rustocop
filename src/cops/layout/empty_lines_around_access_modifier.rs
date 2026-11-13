//! `Layout/EmptyLinesAroundAccessModifier` — a bare access modifier is
//! surrounded by blank lines.
//!
//! Port of upstream's callbacks at the fixed `around` style over the parser
//! AST that RuboCop builds from Prism. A bare `private`/`protected`/`public`/
//! `module_function` (no receiver, arguments or block, in a macro scope: the
//! shared `layout::indentation_width::bare_access`) is checked unless the
//! next statement starts on its line. It needs a blank previous line (comment
//! lines skipped; the first line of a file reads upward from the file's last
//! line, as upstream's `processed_source[0..-1]`) and a blank next line.
//!
//! Upstream keeps whole-investigation state: the class/module/`class << self`
//! and the block visited last before the modifier in parser order, not the
//! enclosing ones. A modifier on the line after that block's first line or
//! after that class's first line (its superclass's for `class A <\n B`, the
//! expression's for `class <<\n self`) needs no blank line before it and gets
//! ``Keep a blank line after `private`.``; one on the line before that class's
//! last line needs none after it. Otherwise the message is ``Keep a blank
//! line before and after `private`.``. The state is computed once per file in
//! one walk ([`Scopes`]), with a block's event at its call as in the parser
//! AST, where the block node encloses its call.
//!
//! The correction inserts the missing line breaks before and after the
//! modifier's line, with that line's own ending, as one edit. With
//! `Layout/EmptyLinesAroundBlockBody` at its fixed `no_empty_lines`, nothing
//! is inserted before the first or after the last statement of a `do`/brace
//! block (not a numbered or `it` block), so the sole statement of a block
//! keeps its finding without an edit. The correction is withheld, finding
//! kept, for the listed source observers (the shared position
//! barrier holds a later `__LINE__`), when an insertion point is inside a
//! literal, ends a line that a heredoc body continues, or meets a backslash
//! continuation, and for a line break before line 1 (which
//! `Layout/LeadingEmptyLines` would remove again).
use std::collections::HashMap;

use ruby_prism::Node;

use super::{empty_line_after_guard_clause::line_break_after, indentation_width::bare_access};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct EmptyLinesAroundAccessModifier;

impl Cop for EmptyLinesAroundAccessModifier {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundAccessModifier"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_statements_node().is_some()
    }

    /// Each statement list is visited once, with every statement's parser
    /// right sibling, keeping long lists linear.
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
        let list = statements.body();
        let mut nexts = list.iter().skip(1);
        let count = list.len();
        for (index, statement) in list.iter().enumerate() {
            let next = nexts.next();
            if !bare_access(&statement, std::iter::once(node).chain(cx.ancestors())) {
                continue;
            }
            let line = file.line_of(statement.location().start_offset());
            if next
                .as_ref()
                .is_some_and(|n| file.line_of(n.location().start_offset()) == line)
            {
                continue;
            }
            let position = Position {
                first: index == 0,
                last: index + 1 == count,
            };
            if let Some(offense) = self.check(&statement, position, cx, file) {
                out.push(offense);
            }
        }
    }
}

/// Where the modifier sits in its statement list.
#[derive(Clone, Copy)]
struct Position {
    first: bool,
    last: bool,
}

impl EmptyLinesAroundAccessModifier {
    fn check(
        &self,
        node: &Node<'_>,
        position: Position,
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Offense> {
        let root = cx.ancestors().last()?;
        let start = node.location().start_offset();
        let first_line = file.line_of(start);
        let last_line = file.last_line_of(node.location());
        let state = file
            .access_modifier_scopes(root)
            .at
            .get(&start)
            .copied()
            .unwrap_or_default();
        let before_empty = previous_line_empty(file, first_line, state);
        let after_empty = next_line_empty(file, last_line, state);
        if before_empty && after_empty {
            return None;
        }
        let name = String::from_utf8_lossy(node.location().as_slice());
        let message = if state.block_start(first_line) || state.class_def(first_line) {
            format!("Keep a blank line after `{name}`.")
        } else {
            format!("Keep a blank line before and after `{name}`.")
        };
        // `inside_block?` with EmptyLinesAroundBlockBody's fixed
        // `no_empty_lines`: the parser parent is a `block` (sole statement)
        // or a `begin` in one.
        let block = cx.parent().is_some_and(plain_block);
        let insert_before = !before_empty && !(block && position.first && !position.last);
        let insert_after = !after_empty && !(block && position.last);
        let (line, col) = file.line_col(start);
        Some(Offense {
            correction: cx
                .corrections_requested()
                .then(|| {
                    correction(
                        file,
                        root,
                        first_line,
                        last_line,
                        insert_before,
                        insert_after,
                    )
                })
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            end_line: last_line,
            col,
        })
    }
}

/// A parser `block` (not `numblock`/`itblock`): a block or lambda whose
/// parameters are not numbered or `it`.
fn plain_block(node: &Node<'_>) -> bool {
    let parameters = if let Some(block) = node.as_block_node() {
        block.parameters()
    } else if let Some(lambda) = node.as_lambda_node() {
        lambda.parameters()
    } else {
        return false;
    };
    !parameters.is_some_and(|p| {
        p.as_numbered_parameters_node().is_some() || p.as_it_parameters_node().is_some()
    })
}

/// One edit around the modifier's lines: the missing line breaks before and
/// after them, each with the line's own ending. `None` when nothing is to be
/// inserted or an insertion is held.
fn correction(
    file: &SourceFile,
    root: &Node<'_>,
    first_line: u32,
    last_line: u32,
    before: bool,
    after: bool,
) -> Option<Correction> {
    if !before && !after {
        return None;
    }
    // The after-insertion's guards (observers, literal interiors, heredoc
    // bodies, backslash lines) apply to the edit as a whole.
    let tail = line_break_after(file, root, last_line, None)?;
    let start = file.line_start(first_line);
    // A line break before line 1 (upstream's wrap-around read of the file's
    // last line, only without a final line break) would be removed again by
    // Layout/LeadingEmptyLines: held.
    if before
        && (first_line == 1
            || file.literal_interior_at(root, start)
            || file.line_text(first_line - 1).last() == Some(&b'\\'))
    {
        return None;
    }
    let end = tail.range.start;
    let newline = tail.replacement;
    let mut replacement = String::new();
    if before {
        replacement.push_str(&newline);
    }
    replacement.push_str(std::str::from_utf8(&file.bytes[start..end]).ok()?);
    if after {
        replacement.push_str(&newline);
    }
    Some(Correction {
        range: start..end,
        replacement,
    })
}

/// `processed_source.lines.size`: the lines before `__END__`, plus the empty
/// line after a final line break when there is no data section.
fn processed_line_count(file: &SourceFile) -> u32 {
    let count = file.code_line_count() as u32;
    if file.data_start().is_none() && file.bytes.last() == Some(&b'\n') {
        count + 1
    } else {
        count
    }
}

/// RuboCop's `String#blank?` on a line (a line past the code reads as blank).
fn blank(text: &[u8]) -> bool {
    text.iter()
        .all(|&b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r' | b'\n' | b'\0'))
}

/// `comment_line?`: `/^\s*#/`.
fn comment_line(text: &[u8]) -> bool {
    text.iter()
        .find(|&&b| !matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r' | b'\n'))
        == Some(&b'#')
}

/// `previous_line_empty?(send_line)`.
fn previous_line_empty(file: &SourceFile, line: u32, state: State) -> bool {
    // `processed_source[0..(send_line - 2)].reverse`: for line 1 that range
    // is `0..-1`, the whole file read upward from its last line.
    let top = if line >= 2 {
        line - 1
    } else {
        processed_line_count(file)
    };
    let Some(previous) = (1..=top)
        .rev()
        .map(|l| file.line_text(l))
        .find(|text| !comment_line(text))
    else {
        return true;
    };
    state.block_start(line) || state.class_def(line) || blank(previous)
}

/// `next_line_empty?(last_send_line)`.
fn next_line_empty(file: &SourceFile, line: u32, state: State) -> bool {
    state.body_end(line) || line >= processed_line_count(file) || blank(file.line_text(line + 1))
}

/// Upstream's instance variables when it reaches a modifier.
#[derive(Clone, Copy, Default)]
pub(crate) struct State {
    /// `@block_line`: the first line of the block visited last.
    block_line: Option<u32>,
    /// `@class_or_module_def_first_line` and `..._last_line`.
    class: Option<(u32, u32)>,
}

impl State {
    fn block_start(self, line: u32) -> bool {
        self.block_line.is_some_and(|b| line == b + 1)
    }

    fn class_def(self, line: u32) -> bool {
        self.class.is_some_and(|(first, _)| line == first + 1)
    }

    fn body_end(self, line: u32) -> bool {
        self.class.is_some_and(|(_, last)| line + 1 == last)
    }
}

/// The upstream state at each candidate modifier (a receiverless call
/// without arguments or block, keyed by its start offset), from one walk of
/// the file in parser order. Built once per file.
#[derive(Default)]
pub(crate) struct Scopes {
    at: HashMap<usize, State>,
}

impl Scopes {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        struct Walk<'f> {
            file: &'f SourceFile,
            state: State,
            at: HashMap<usize, State>,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Walk<'_> {
            fn enter(&mut self, node: Node<'pr>) {
                let file = self.file;
                let line = |offset: usize| file.line_of(offset);
                match &node {
                    Node::ClassNode { .. } => {
                        let class = node.as_class_node().unwrap();
                        let first = class.superclass().map_or_else(
                            || line(class.location().start_offset()),
                            |s| line(s.location().start_offset()),
                        );
                        self.state.class = Some((first, file.last_line_of(class.location())));
                    }
                    Node::ModuleNode { .. } => {
                        let location = node.location();
                        self.state.class =
                            Some((line(location.start_offset()), file.last_line_of(location)));
                    }
                    Node::SingletonClassNode { .. } => {
                        let sclass = node.as_singleton_class_node().unwrap();
                        self.state.class = Some((
                            line(sclass.expression().location().start_offset()),
                            file.last_line_of(sclass.location()),
                        ));
                    }
                    // The parser's `block` node encloses its call: its event
                    // comes before the call's receiver and arguments.
                    Node::CallNode { .. } => {
                        let call = node.as_call_node().unwrap();
                        if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                            self.state.block_line = Some(line(node.location().start_offset()));
                        } else if call.receiver().is_none()
                            && call.arguments().is_none()
                            && call.block().is_none()
                            && matches!(
                                call.name().as_slice(),
                                b"public" | b"protected" | b"private" | b"module_function"
                            )
                        {
                            self.at.insert(node.location().start_offset(), self.state);
                        }
                    }
                    Node::SuperNode { .. } | Node::ForwardingSuperNode { .. } => {
                        let block = match &node {
                            Node::SuperNode { .. } => node.as_super_node().unwrap().block(),
                            _ => node
                                .as_forwarding_super_node()
                                .unwrap()
                                .block()
                                .map(|b| b.as_node()),
                        };
                        if block.is_some_and(|b| b.as_block_node().is_some()) {
                            self.state.block_line = Some(line(node.location().start_offset()));
                        }
                    }
                    Node::LambdaNode { .. } => {
                        self.state.block_line = Some(line(node.location().start_offset()));
                    }
                    _ => {}
                }
            }
        }
        let mut walk = Walk {
            file,
            state: State::default(),
            at: HashMap::new(),
        };
        crate::cops::walk::walk(&mut walk, root);
        Scopes { at: walk.at }
    }
}

#[cfg(test)]
mod tests;
