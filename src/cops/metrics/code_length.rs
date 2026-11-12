//! RuboCop's `CodeLength` mixin and `Metrics::Utils::CodeLengthCalculator`
//! at the fixed defaults (`CountComments: false`, `CountAsOne: []`), shared
//! by `Metrics/MethodLength`, `Metrics/BlockLength`, `Metrics/ClassLength`
//! and `Metrics/ModuleLength`.
//!
//! Upstream measures RuboCop's (parser gem) AST; this module maps the Prism
//! nodes onto that view:
//!
//! * A definition's *body* is RuboCop's `node.body`: the sole statement, the
//!   statement list, or, with `rescue`/`ensure` clauses, the handler node
//!   spanning from the first statement (or keyword) to the end of the last
//!   clause. Its source lines are counted unless blank or a comment line.
//! * A body with a heredoc below it is counted by whole lines up to the last
//!   line of its deepest RuboCop descendant, heredoc terminators included.
//! * A `class`/`module` is counted by line numbers between its first and last
//!   line, minus the lines of every nested `class`/`module`, reading
//!   `processed_source[line]`: upstream indexes the zero-based line array with
//!   one-based numbers, so it reads the line *after* each number. A
//!   class/module whose body is a sole class/module counts zero.
//!
//! Every count is skipped (no finding) when the measured node spans at most
//! `max` lines, as upstream's shortcut does. Findings are never corrected.

use ruby_prism::{BeginNode, Location, Node};

use crate::{
    cops::{
        Cop,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};

/// Upstream `check_code_length`: report `label` when the measured node
/// (`span`) is longer than `max` lines and its `length` exceeds `max`.
/// The finding covers `reported`.
#[allow(clippy::too_many_arguments)]
pub(super) fn check(
    cop: &dyn Cop,
    label: &str,
    max: usize,
    span: &Location<'_>,
    reported: &Location<'_>,
    length: impl FnOnce() -> usize,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let first = file.line_of(span.start_offset());
    let last = last_line(span, file);
    if (last - first + 1) as usize <= max {
        return;
    }
    let length = length();
    if length <= max {
        return;
    }
    let (line, col) = file.line_col(reported.start_offset());
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        line,
        col,
        end_line: last_line(reported, file),
        message: format!("{label} has too many lines. [{length}/{max}]"),
        correction: None,
    });
}

/// The inclusive last line of a location ([`SourceFile::last_line_of`]).
fn last_line(location: &Location<'_>, file: &SourceFile) -> u32 {
    file.line_of(
        location
            .end_offset()
            .saturating_sub(1)
            .max(location.start_offset()),
    )
}

/// The code length of a `def`, block, lambda or `class << self` with this
/// Prism body (upstream `code_length` for non-classlike nodes).
pub(super) fn body_length(body: Option<Node<'_>>, file: &SourceFile) -> usize {
    let Some(body) = Body::of(body) else {
        return 0;
    };
    let (start, end) = body.range();
    let source = &file.bytes[start..end];
    let mut lines = DescendantLines::new(file);
    // Every heredoc below the body opens with `<<` inside its range.
    if memchr::memmem::find(source, b"<<").is_some() {
        body.visit_descendants(&mut lines);
    }
    if !lines.heredoc {
        return source
            .split_inclusive(|&b| b == b'\n')
            .filter(|line| relevant(line))
            .count();
    }
    let last = lines.last.max(body.root_additions(file));
    (file.line_of(start)..=last)
        .filter(|&line| relevant(file.line_text(line)))
        .count()
}

/// The code length of a `class` or `module` node with this Prism body
/// (upstream `classlike_code_length`).
pub(super) fn classlike_length(
    node: &Node<'_>,
    body: Option<Node<'_>>,
    file: &SourceFile,
) -> usize {
    let namespace = body
        .and_then(|body| body.as_statements_node())
        .is_some_and(|statements| {
            let body = statements.body();
            body.len() == 1
                && body
                    .iter()
                    .all(|s| s.as_class_node().is_some() || s.as_module_node().is_some())
        });
    if namespace {
        return 0;
    }
    let first = file.line_of(node.location().start_offset());
    let last = last_line(&node.location(), file);
    if last < first + 2 {
        return 0;
    }
    // Body line numbers `first + 1 ..= last - 1`, minus nested class/module lines.
    let mut excluded = vec![false; (last - first + 1) as usize];
    let mut inner = InnerClasslike {
        file,
        first,
        last,
        excluded: &mut excluded,
        depth: 0,
    };
    walk(&mut inner, node);
    (first + 1..last)
        .filter(|&number| !excluded[(number - first) as usize])
        // `processed_source[number]`: the zero-based array read with a
        // one-based number, i.e. the following line.
        .filter(|&number| relevant(file.line_text(number + 1)))
        .count()
}

/// Upstream `irrelevant_line` negated at `CountComments: false`: neither
/// blank (`lstrip.empty?`) nor a comment line (`/^\s*#/`).
fn relevant(line: &[u8]) -> bool {
    let blank = line
        .iter()
        .all(|&b| matches!(b, 0 | b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'));
    let comment = line
        .iter()
        .find(|&&b| !matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
        == Some(&b'#');
    !blank && !comment
}

/// RuboCop's `node.body` for a Prism body.
enum Body<'pr> {
    /// A sole statement.
    Single(Node<'pr>),
    /// Several statements (RuboCop's `begin`).
    Many(Vec<Node<'pr>>),
    /// Statements with `rescue`/`ensure` clauses (RuboCop's `rescue` or
    /// `ensure` node); Prism's implicit `BeginNode`.
    Handler(BeginNode<'pr>),
}

impl<'pr> Body<'pr> {
    fn of(body: Option<Node<'pr>>) -> Option<Self> {
        let body = body?;
        if let Some(statements) = body.as_statements_node() {
            let mut list: Vec<_> = statements.body().iter().collect();
            return match list.len() {
                0 => None,
                1 => list.pop().map(Body::Single),
                _ => Some(Body::Many(list)),
            };
        }
        match body.as_begin_node() {
            Some(begin) if begin.begin_keyword_loc().is_none() => Some(Body::Handler(begin)),
            _ => Some(Body::Single(body)),
        }
    }

    /// The body's source range as RuboCop's AST has it.
    fn range(&self) -> (usize, usize) {
        match self {
            Body::Single(node) => (node.location().start_offset(), node.location().end_offset()),
            Body::Many(list) => (
                list[0].location().start_offset(),
                list[list.len() - 1].location().end_offset(),
            ),
            Body::Handler(begin) => handler_range(begin),
        }
    }

    /// Walk every RuboCop descendant of the body, the body itself excluded.
    fn visit_descendants(&self, lines: &mut DescendantLines<'_>) {
        match self {
            Body::Single(node) => walk(lines, node),
            Body::Many(list) => {
                lines.include_root = true;
                for node in list {
                    walk(lines, node);
                }
            }
            Body::Handler(begin) => walk(lines, &begin.as_node()),
        }
    }

    /// The last line of RuboCop descendants that have no Prism node of
    /// their own: the send of a block (Prism folds it into the call), and
    /// the `rescue`/`ensure` nodes of a `begin` that end at a bare keyword.
    fn root_additions(&self, file: &SourceFile) -> u32 {
        let node = match self {
            Body::Single(node) => node,
            Body::Many(_) => return 0,
            Body::Handler(begin) => {
                // The handler is an `ensure` wrapping a `rescue`-with-`else`
                // only when both clauses exist.
                return match (begin.else_clause(), begin.ensure_clause()) {
                    (Some(else_clause), Some(_)) => {
                        file.line_of(else_clause.else_keyword_loc().start_offset())
                    }
                    _ => 0,
                };
            }
        };
        if let Some(call) = node.as_call_node() {
            if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                let end = crate::cops::ast::source_end(node);
                return file.line_of(end.saturating_sub(1).max(node.location().start_offset()));
            }
        } else if let Some(sup) = node.as_super_node() {
            if sup.block().is_some_and(|b| b.as_block_node().is_some()) {
                let mut end = sup.keyword_loc().end_offset();
                if let Some(arguments) = sup.arguments() {
                    end = end.max(arguments.location().end_offset());
                }
                if let Some(rparen) = sup.rparen_loc() {
                    end = end.max(rparen.end_offset());
                }
                return file.line_of(end - 1);
            }
        } else if let Some(begin) = node.as_begin_node() {
            // An explicit `begin`: its `rescue` node extends to a bare
            // `else`, its `ensure` node to a bare `ensure`.
            let mut last = 0;
            if let Some(else_clause) = begin.else_clause() {
                last = file.line_of(else_clause.else_keyword_loc().start_offset());
            }
            if let Some(ensure) = begin.ensure_clause() {
                last = last.max(file.line_of(ensure.ensure_keyword_loc().start_offset()));
            }
            return last;
        }
        0
    }
}

/// The range of RuboCop's `rescue`/`ensure` body node (parser's
/// `eh_keyword_map`): from the first statement, or the first clause keyword,
/// to the end of the last clause.
fn handler_range(begin: &BeginNode<'_>) -> (usize, usize) {
    let statements = begin.statements().filter(|s| !s.body().is_empty());
    let rescue = begin.rescue_clause();
    let start = statements
        .as_ref()
        .map(|s| s.location().start_offset())
        .or_else(|| rescue.as_ref().map(|r| r.keyword_loc().start_offset()))
        .or_else(|| {
            begin
                .ensure_clause()
                .map(|e| e.ensure_keyword_loc().start_offset())
        })
        .unwrap_or_else(|| begin.location().start_offset());
    let end = if let Some(ensure) = begin.ensure_clause() {
        ensure
            .statements()
            .map_or(ensure.ensure_keyword_loc().end_offset(), |s| {
                s.location().end_offset()
            })
    } else if let Some(else_clause) = begin.else_clause() {
        else_clause
            .statements()
            .map_or(else_clause.else_keyword_loc().end_offset(), |s| {
                s.location().end_offset()
            })
    } else if let Some(mut clause) = rescue {
        while let Some(next) = clause.subsequent() {
            clause = next;
        }
        crate::cops::ast::source_end(&clause.as_node())
    } else {
        statements.map_or(start, |s| s.location().end_offset())
    };
    (start, end.max(start))
}

/// Collects, over the RuboCop descendants of a body, whether one is a
/// `str`/`dstr` heredoc and the last line any of them reaches (a heredoc's
/// terminator line). Prism nodes with no RuboCop counterpart, or whose
/// range runs past it to a closing keyword, contribute only their children.
struct DescendantLines<'f> {
    file: &'f SourceFile,
    /// Nodes entered and not left; the walk root is at depth 0.
    depth: usize,
    /// Whether walk roots are descendants too (the statements of a list).
    include_root: bool,
    heredoc: bool,
    last: u32,
}

impl<'f> DescendantLines<'f> {
    fn new(file: &'f SourceFile) -> Self {
        Self {
            file,
            depth: 0,
            include_root: false,
            heredoc: false,
            last: 0,
        }
    }

    fn record(&mut self, node: &Node<'_>) {
        let (string, closing) = match node {
            Node::StringNode { .. } => {
                let n = node.as_string_node().unwrap();
                (true, heredoc_closing(n.opening_loc(), n.closing_loc()))
            }
            Node::InterpolatedStringNode { .. } => {
                let n = node.as_interpolated_string_node().unwrap();
                (true, heredoc_closing(n.opening_loc(), n.closing_loc()))
            }
            Node::XStringNode { .. } => {
                let n = node.as_x_string_node().unwrap();
                (
                    false,
                    heredoc_closing(Some(n.opening_loc()), Some(n.closing_loc())),
                )
            }
            Node::InterpolatedXStringNode { .. } => {
                let n = node.as_interpolated_x_string_node().unwrap();
                (
                    false,
                    heredoc_closing(Some(n.opening_loc()), Some(n.closing_loc())),
                )
            }
            Node::StatementsNode { .. }
            | Node::ArgumentsNode { .. }
            | Node::BlockNode { .. }
            | Node::ElseNode { .. }
            | Node::EnsureNode { .. }
            | Node::NumberedParametersNode { .. }
            | Node::ItParametersNode { .. } => return,
            Node::BeginNode { .. }
                if node.as_begin_node().unwrap().begin_keyword_loc().is_none() =>
            {
                return;
            }
            Node::IfNode { .. }
                if node
                    .as_if_node()
                    .unwrap()
                    .if_keyword_loc()
                    .is_some_and(|k| k.as_slice() == b"elsif") =>
            {
                // Parser's `elsif` node has no `end`: it ends at its else
                // body, or at a bare `else` keyword (`condition_map`).
                let bare_else = node
                    .as_if_node()
                    .unwrap()
                    .subsequent()
                    .and_then(|s| s.as_else_node())
                    .filter(|e| e.statements().is_none());
                if let Some(else_clause) = bare_else {
                    let line = self
                        .file
                        .line_of(else_clause.else_keyword_loc().start_offset());
                    self.last = self.last.max(line);
                }
                return;
            }
            _ => (false, None),
        };
        if let Some(closing) = closing {
            self.heredoc |= string;
            self.last = self.last.max(self.file.line_of(closing.start_offset()));
        } else {
            self.last = self.last.max(last_line(&node.location(), self.file));
        }
    }
}

impl<'pr> NodeVisitor<'pr> for DescendantLines<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        if self.depth > 0 || self.include_root {
            self.record(&node);
        }
        self.depth += 1;
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }
}

/// A heredoc's terminator location; `None` for other string literals.
fn heredoc_closing<'pr>(
    opening: Option<Location<'pr>>,
    closing: Option<Location<'pr>>,
) -> Option<Location<'pr>> {
    opening
        .filter(|o| o.as_slice().starts_with(b"<<"))
        .and(closing)
}

/// Marks the line numbers of every `class`/`module` below a classlike node.
struct InnerClasslike<'a> {
    file: &'a SourceFile,
    first: u32,
    last: u32,
    excluded: &'a mut [bool],
    depth: usize,
}

impl<'pr> NodeVisitor<'pr> for InnerClasslike<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        if self.depth > 0 && (node.as_class_node().is_some() || node.as_module_node().is_some()) {
            let from = self
                .file
                .line_of(node.location().start_offset())
                .max(self.first);
            let to = last_line(&node.location(), self.file).min(self.last);
            for number in from..=to {
                self.excluded[(number - self.first) as usize] = true;
            }
        }
        self.depth += 1;
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }
}
