//! `Layout/HeredocIndentation` — heredoc bodies are indented one step (two
//! spaces) past their opening line, using `<<~`. Fixed defaults:
//! `ActiveSupportExtensionsEnabled: false` (no `squish` handling) and
//! `Layout/LineLength` `AllowHeredoc: true` (no long-line exemption).
use std::ops::Range;

use ruby_prism::Node;

use super::heredoc::indent_level;
use crate::{
    cops::{
        Cop, NodeContext,
        naming::heredoc::{Heredoc, strip},
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct HeredocIndentation;

/// `Layout/IndentationWidth` `Width`, fixed.
const WIDTH: usize = 2;

impl Cop for HeredocIndentation {
    fn name(&self) -> &'static str {
        "Layout/HeredocIndentation"
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
        let Some(heredoc) = Heredoc::of(node, file) else {
            return;
        };
        let Some(finding) = Finding::of(&heredoc, file) else {
            return;
        };
        // `within_offending_heredoc?`: earlier offending bodies are those of
        // enclosing heredocs (an opener inside a body is in its interpolation),
        // evaluated outermost first like the stateful upstream callback.
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let mut offending: Vec<Range<usize>> = Vec::new();
        let contains = |bodies: &[Range<usize>], h: &Heredoc<'_>| {
            let opening = h.opening.start_offset()..h.opening.end_offset();
            bodies
                .iter()
                .any(|b| b.start <= opening.start && opening.end <= b.end)
        };
        for ancestor in ancestors.iter().rev() {
            if let Some(outer) = Heredoc::of(ancestor, file)
                && let Some(found) = Finding::of(&outer, file)
                && !contains(&offending, &outer)
            {
                offending.push(found.body);
            }
        }
        if contains(&offending, &heredoc) {
            return;
        }
        let message = match finding.kind {
            Kind::Squiggly => format!("Use {WIDTH} spaces for indentation in a heredoc."),
            Kind::Dash => format!(
                "Use {WIDTH} spaces for indentation in a heredoc by using `<<~` instead of `<<-`."
            ),
            Kind::Plain => format!(
                "Use {WIDTH} spaces for indentation in a heredoc by using `<<~` instead of `<<`."
            ),
        };
        let correction = if cx.corrections_requested() {
            let root = ancestors.last().copied().unwrap_or(node);
            correction(node, &heredoc, &finding, file, root)
        } else {
            None
        };
        let (line, col) = file.line_col(finding.body.start);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            end_line: file.line_of(finding.body.end.saturating_sub(1)).max(line),
            col,
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Plain,
    Dash,
    Squiggly,
}

/// The upstream `on_heredoc` predicate, before `within_offending_heredoc?`.
struct Finding {
    /// The translation's `heredoc_body` (composed parts).
    body: Range<usize>,
    kind: Kind,
    /// `indent_level(body)`.
    level: usize,
    /// `base_indent_level`: the opening line's indentation.
    base: usize,
}

impl Finding {
    fn of(heredoc: &Heredoc<'_>, file: &SourceFile) -> Option<Self> {
        let body = &file.bytes[heredoc.body.clone()];
        if strip(body).is_empty() {
            return None;
        }
        let level = indent_level(body);
        let kind = match heredoc.opening.as_slice().get(2) {
            Some(b'~') => Kind::Squiggly,
            Some(b'-') => Kind::Dash,
            _ => Kind::Plain,
        };
        let opening_line = file.line_of(heredoc.opening.start_offset());
        let base = indent_level(file.line_text(opening_line));
        let offending = match kind {
            Kind::Squiggly => base + WIDTH != level,
            // `ActiveSupportExtensionsEnabled: false`: never `heredoc_squish?`.
            Kind::Dash | Kind::Plain => level == 0,
        };
        offending.then(|| Self {
            body: heredoc.body.clone(),
            kind,
            level,
            base,
        })
    }
}

/// Subtree facts that decide whether a re-indentation keeps literal values.
#[derive(Default)]
struct Subtree {
    /// Heredoc openings below the checked one, and whether each is `<<~`.
    nested: Vec<bool>,
    /// An interpolation whose code spans lines: its lines are not string lines.
    multiline_interpolation: bool,
    depth: usize,
}

impl<'pr> NodeVisitor<'pr> for Subtree {
    fn enter(&mut self, node: Node<'pr>) {
        self.depth += 1;
        if self.depth > 1
            && let Some(opening) = heredoc_opening(&node)
        {
            self.nested.push(opening.get(2) == Some(&b'~'));
        }
        if let Some(embedded) = node.as_embedded_statements_node() {
            let (open, close) = (embedded.opening_loc(), embedded.closing_loc());
            self.multiline_interpolation |= open.as_slice() == b"#{"
                && close.start_offset() > open.start_offset()
                && memchr::memchr(
                    b'\n',
                    &embedded.location().as_slice()[..close.start_offset() - open.start_offset()],
                )
                .is_some();
        }
    }
    fn leave(&mut self) {
        self.depth -= 1;
    }
}

fn heredoc_opening<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    let opening = match node {
        Node::StringNode { .. } => node.as_string_node()?.opening_loc()?,
        Node::InterpolatedStringNode { .. } => node.as_interpolated_string_node()?.opening_loc()?,
        Node::XStringNode { .. } => node.as_x_string_node()?.opening_loc(),
        Node::InterpolatedXStringNode { .. } => node.as_interpolated_x_string_node()?.opening_loc(),
        _ => return None,
    };
    let bytes = opening.as_slice();
    bytes.starts_with(b"<<").then_some(bytes)
}

/// Physical lines from `start` (a line start) up to `end` (a line start).
fn lines(file: &SourceFile, range: Range<usize>) -> impl Iterator<Item = (usize, &[u8])> {
    let mut offset = range.start;
    file.bytes[range]
        .split_inclusive(|&b| b == b'\n')
        .map(move |line| {
            let start = offset;
            offset += line.len();
            (start, line)
        })
}

/// A line's text without its `\n`/`\r\n` terminator.
fn content(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Leading blanks Ruby's squiggly dedent and the upstream rewrite inspect.
fn prefix(line: &[u8]) -> &[u8] {
    let n = line
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c'))
        .count();
    &line[..n]
}

/// A backslash line continuation joins the next physical line to this one.
fn continues(line: &[u8]) -> bool {
    content(line)
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

/// The upstream edit (`adjust_squiggly` or `adjust_minus`), withheld when it
/// could change the string's value or when a listed observer reads source.
fn correction<'pr>(
    node: &Node<'pr>,
    heredoc: &Heredoc<'pr>,
    finding: &Finding,
    file: &SourceFile,
    root: &Node<'pr>,
) -> Option<Correction> {
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
    {
        return None;
    }
    let mut subtree = Subtree::default();
    walk(&mut subtree, node);
    if subtree.multiline_interpolation {
        return None;
    }
    let closing_line = file.line_of(heredoc.closing.start_offset());
    let closing_start = file.line_start(closing_line);
    let region = heredoc.body.start..closing_start;
    if lines(file, region.clone()).any(|(_, line)| continues(line)) {
        return None;
    }
    let opening = heredoc.opening.start_offset();
    if finding.kind != Kind::Squiggly {
        // With no nested heredoc, interpolated code line or continuation,
        // every physical line is a string line: a zero upstream level is a
        // zero dedent, so `<<~` keeps the value. Terminators are proved by
        // the shared heredoc boundary index.
        if !subtree.nested.is_empty() {
            return None;
        }
        let prefix = if finding.kind == Kind::Dash { 3 } else { 2 };
        return Some(Correction {
            range: opening..opening + prefix,
            replacement: "<<~".into(),
        });
    }
    // A uniform shift of every line at least `level` deep keeps each `<<~`
    // value (its dedent shifts by the same amount). Tabs (dedented by
    // columns), `<<-` bodies and short blank lines would not shift uniformly.
    let (level, target) = (finding.level, finding.base + WIDTH);
    let opening_line = file.line_text(file.line_of(opening));
    let closing_text = file.line_text(closing_line);
    if subtree.nested.iter().any(|squiggly| !squiggly)
        || prefix(opening_line).iter().any(|&b| b != b' ')
        || prefix(closing_text).iter().any(|&b| b != b' ')
    {
        return None;
    }
    let mut replacement = Vec::with_capacity(region.len() + closing_text.len());
    for (start, line) in lines(file, region) {
        let blanks = prefix(line);
        if blanks.iter().any(|&b| b != b' ') {
            return None;
        }
        let blank_line = content(line).len() == blanks.len();
        if blank_line && blanks.len() < level && blanks.len() > target && !blanks.is_empty() {
            return None;
        }
        if start < finding.body.end && blanks.len() >= level {
            replacement.extend(std::iter::repeat_n(b' ', target));
            replacement.extend_from_slice(&line[level..]);
        } else {
            replacement.extend_from_slice(line);
        }
    }
    let end_level = prefix(closing_text).len();
    replacement.extend(std::iter::repeat_n(b' ', end_level.max(finding.base)));
    Some(Correction {
        range: heredoc.body.start..closing_start + end_level,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests;
