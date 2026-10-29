//! Inline directive comments.
//!
//! Both spellings are honored:
//!
//! - `# rustocop:disable CopName -- optional reason`
//! - `# rubocop:disable CopName` (legacy — teams adopting rustocop on an
//!   existing codebase get their existing suppression comments working for
//!   free)
//!
//! Targets can be full cop names (`Lint/Debugger`), department-less cop names
//! (`Debugger`), department names (`Lint`), or `all`. Scoping follows
//! RuboCop's `CommentConfig`:
//!
//! - A `disable` on a comment-only line applies from its line until the end
//!   of the file or a matching `enable` (whose line is still disabled).
//! - A `disable` with code before it on the same line (`foo # rubocop:disable
//!   X`), or written after other comment text (`# note # rubocop:disable X`),
//!   applies to its own line only. An `enable` in either position does
//!   nothing.
//! - A `disable-next` on a comment-only line applies to the whole next
//!   statement, including heredocs; blank lines break attachment.
//! - `enable-next` suspends an open disable for the attached statement only.
//! - `next -Cop +Other` combines statement disables and temporary enables.
//! - `push` saves all open states, applies signed arguments, and `pop` restores
//!   them from its own line. A `+` closes a disable after the push line.
//!
//! `todo` / `todo-next` alias `disable` / `disable-next`. See the shared
//! [`Directives`] analyzer for range and opt-in semantics.

pub(crate) mod comments;
mod scopes;
pub(crate) mod syntax;
pub use scopes::Directives;
pub(crate) use scopes::{redundant_enables, source_opts_in};
#[cfg(test)]
mod scope_tests;

use ruby_prism::{BeginNode, IfNode, Node, RescueNode, Visit};

use crate::source::SourceFile;

/// What a directive comment asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Disable,
    Enable,
    DisableNext,
    EnableNext,
    Push,
    Next,
    Pop,
}

/// RuboCop's directive modes, longest first so `disable-next` is not read as
/// `disable` (`-` ends a word).
const MODES: [(&[u8], Mode); 9] = [
    (b"disable-next", Mode::DisableNext),
    (b"enable-next", Mode::EnableNext),
    (b"todo-next", Mode::DisableNext),
    (b"disable", Mode::Disable),
    (b"enable", Mode::Enable),
    (b"todo", Mode::Disable),
    (b"push", Mode::Push),
    (b"next", Mode::Next),
    (b"pop", Mode::Pop),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Targets {
    All,
    List(Vec<String>),
}

/// Resolve exact candidate code lines in one AST walk, sharing the existing
/// next-statement semantics (including deferred heredoc terminators).
pub(crate) fn statement_ends(
    root: &Node<'_>,
    file: &SourceFile,
    mut lines: Vec<u32>,
) -> Vec<(u32, u32)> {
    lines.sort_unstable();
    lines.dedup();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut finder = StatementFinder {
        file,
        best: vec![None; lines.len()],
        lines,
        heredocs: Vec::new(),
    };
    finder.visit(root);
    finder.heredocs.sort_unstable_by_key(|(start, _)| *start);
    finder
        .lines
        .iter()
        .filter_map(|&line| {
            // A comment/blank-only line is not a statement. In particular, don't
            // attach through stacked comments for the tighter-scope diagnostic.
            let text = file.line_text(line);
            let indent = text.iter().position(|&b| !is_space(b))?;
            let offset = file.line_start(line) + indent;
            let comments = file.comments();
            let i = comments.partition_point(|c| c.start <= offset);
            if i > 0 && comments[i - 1].end > offset {
                return None;
            }
            Some((line, finder.statement_end_line(line)))
        })
        .collect()
}

/// Resolve every comment-only line's next-statement attachment in one reverse
/// source pass. In particular, stacked directives must not rescan their tails.
pub(crate) fn comment_attachments(file: &SourceFile) -> std::collections::HashMap<u32, u32> {
    let mut attached = std::collections::HashMap::new();
    let mut next_code = None;
    let comments = file.comments();
    for line in (1..=u32::try_from(file.line_count()).unwrap_or(u32::MAX)).rev() {
        let text = file.line_text(line);
        let bom = if line == 1 && text.starts_with(b"\xEF\xBB\xBF") {
            3
        } else {
            0
        };
        let Some(indent) = text[bom..].iter().position(|&b| !is_space(b)) else {
            next_code = None;
            continue;
        };
        let offset = file.line_start(line) + bom + indent;
        let i = comments.partition_point(|c| c.start <= offset);
        if i > 0 && comments[i - 1].end > offset {
            if let Some(code) = next_code {
                attached.insert(line, code);
            }
        } else {
            next_code = Some(line);
        }
    }
    attached
}

/// A statement candidate: its byte span and last line.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    start: usize,
    end: usize,
    last_line: u32,
}

/// Finds, for each target line, the statement starting on it, and every
/// heredoc's terminator line.
///
/// Prism nodes are mapped onto the parser gem's (whose AST RuboCop scopes
/// against): sequences and wrappers the parser gem has no statement-like
/// node for are skipped (see [`is_statement`]), a `rescue` clause ends
/// before the clauses after it, an `elsif` before the shared `end`, and a
/// body with `rescue`/`ensure` gets the wrapper node the parser gem builds
/// from its first statement.
struct StatementFinder<'a> {
    file: &'a SourceFile,
    /// Target lines, sorted.
    lines: Vec<u32>,
    /// The best candidate per target line, parallel to `lines`.
    best: Vec<Option<Candidate>>,
    /// (opening offset, terminator line) of every heredoc.
    heredocs: Vec<(usize, u32)>,
}

impl StatementFinder<'_> {
    fn node(&mut self, node: &Node<'_>) {
        if let Some(line) = heredoc_end_line(node, self.file) {
            self.heredocs.push((node.location().start_offset(), line));
        }
        if is_statement(node) {
            let start = node.location().start_offset();
            let end = match node {
                Node::RescueNode { .. } => node.as_rescue_node().map(|r| rescue_clause_end(&r)),
                Node::IfNode { .. } => node.as_if_node().and_then(|i| elsif_end(&i)),
                _ => None,
            };
            self.consider(start, end.unwrap_or(node.location().end_offset()));
        }
        if let Some((start, end)) = node.as_begin_node().and_then(|b| body_wrapper(&b)) {
            self.consider(start, end);
        }
    }

    /// Keep `start..end` if it starts on a target line and ends later than
    /// that line's candidate so far (earlier, outer nodes win ties).
    fn consider(&mut self, start: usize, end: usize) {
        let Ok(index) = self.lines.binary_search(&self.file.line_of(start)) else {
            return;
        };
        let last_line = self.file.line_of(end.saturating_sub(1).max(start));
        if self.best[index].is_none_or(|best| last_line > best.last_line) {
            self.best[index] = Some(Candidate {
                start,
                end,
                last_line,
            });
        }
    }

    /// The last line of the statement starting on `line` (a target line).
    fn statement_end_line(&self, line: u32) -> u32 {
        let Some(Some(statement)) = self
            .lines
            .binary_search(&line)
            .ok()
            .map(|index| self.best[index])
        else {
            return line;
        };
        let first = self
            .heredocs
            .partition_point(|(start, _)| *start < statement.start);
        self.heredocs[first..]
            .iter()
            .take_while(|(start, _)| *start < statement.end)
            .map(|(_, end_line)| *end_line)
            .fold(statement.last_line, u32::max)
    }
}

impl<'pr> Visit<'pr> for StatementFinder<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.node(&node);
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.node(&node);
    }

    // ruby-prism visits `rescue` clauses as typed children, skipping the
    // enter hooks. The other kinds it visits that way are either skipped by
    // `is_statement` or start where their parent does.
    fn visit_rescue_node(&mut self, node: &RescueNode<'pr>) {
        self.node(&node.as_node());
        ruby_prism::visit_rescue_node(self, node);
    }
}

/// Whether the node can be a statement scope: the parser gem builds no node
/// with the same start for sequences (`begin`), call arguments and method
/// parameters, `else`/`ensure` clauses, block bodies, implicit block
/// parameters (`_1`, `it`; Prism spans them over the whole block) and
/// interpolations.
fn is_statement(node: &Node<'_>) -> bool {
    match node {
        Node::ProgramNode { .. }
        | Node::StatementsNode { .. }
        | Node::ParenthesesNode { .. }
        | Node::ArgumentsNode { .. }
        | Node::ParametersNode { .. }
        | Node::ElseNode { .. }
        | Node::EnsureNode { .. }
        | Node::BlockNode { .. }
        | Node::NumberedParametersNode { .. }
        | Node::ItParametersNode { .. }
        | Node::EmbeddedStatementsNode { .. } => false,
        // An implicit `begin` (a body with `rescue`/`ensure`) is handled by
        // `body_wrapper`.
        Node::BeginNode { .. } => node
            .as_begin_node()
            .is_some_and(|begin| begin.begin_keyword_loc().is_some()),
        _ => true,
    }
}

/// The terminator line of a heredoc node.
fn heredoc_end_line(node: &Node<'_>, file: &SourceFile) -> Option<u32> {
    let (opening, closing) = match node {
        Node::StringNode { .. } => {
            let string = node.as_string_node()?;
            (string.opening_loc()?, string.closing_loc()?)
        }
        Node::InterpolatedStringNode { .. } => {
            let string = node.as_interpolated_string_node()?;
            (string.opening_loc()?, string.closing_loc()?)
        }
        Node::XStringNode { .. } => {
            let string = node.as_x_string_node()?;
            (string.opening_loc(), string.closing_loc())
        }
        Node::InterpolatedXStringNode { .. } => {
            let string = node.as_interpolated_x_string_node()?;
            (string.opening_loc(), string.closing_loc())
        }
        _ => return None,
    };
    if !opening.as_slice().starts_with(b"<<") {
        return None;
    }
    // The terminator's location includes its newline.
    let end = closing
        .end_offset()
        .saturating_sub(1)
        .max(closing.start_offset());
    Some(file.line_of(end))
}

/// The end of a `rescue` clause without the clauses after it (the parser
/// gem's `resbody`).
fn rescue_clause_end(rescue: &RescueNode<'_>) -> usize {
    let mut end = rescue.keyword_loc().end_offset();
    for exception in &rescue.exceptions() {
        end = end.max(exception.location().end_offset());
    }
    let parts = [
        rescue.reference().map(|node| node.location().end_offset()),
        rescue.then_keyword_loc().map(|loc| loc.end_offset()),
        rescue.statements().map(|node| node.location().end_offset()),
    ];
    parts.into_iter().flatten().fold(end, usize::max)
}

/// The end of an `elsif` branch without the `end` it shares with its `if`;
/// `None` for any other `if`.
fn elsif_end(node: &IfNode<'_>) -> Option<usize> {
    if node.if_keyword_loc()?.as_slice() != b"elsif" {
        return None;
    }
    let mut end = node.predicate().location().end_offset();
    if let Some(then) = node.then_keyword_loc() {
        end = end.max(then.end_offset());
    }
    if let Some(statements) = node.statements() {
        end = end.max(statements.location().end_offset());
    }
    let subsequent = node.subsequent().and_then(|subsequent| {
        if let Some(elsif) = subsequent.as_if_node() {
            return elsif_end(&elsif);
        }
        let else_clause = subsequent.as_else_node()?;
        let keyword = else_clause.else_keyword_loc().end_offset();
        let body = else_clause.statements().map(|s| s.location().end_offset());
        Some(body.map_or(keyword, |body| body.max(keyword)))
    });
    Some(subsequent.map_or(end, |subsequent| subsequent.max(end)))
}

/// The span of the node the parser gem wraps a body with `rescue`/`ensure`
/// in: from its first statement (or, with an empty body, its first `rescue`
/// or `ensure` keyword) to the end of the `ensure` body, the `else` body or
/// the last `rescue` clause.
fn body_wrapper(begin: &BeginNode<'_>) -> Option<(usize, usize)> {
    let start = match begin.statements() {
        Some(statements) => statements.location().start_offset(),
        None => match (begin.rescue_clause(), begin.ensure_clause()) {
            (Some(rescue), _) => rescue.keyword_loc().start_offset(),
            (None, Some(ensure)) => ensure.ensure_keyword_loc().start_offset(),
            (None, None) => return None,
        },
    };
    let end = if let Some(ensure) = begin.ensure_clause() {
        let keyword = ensure.ensure_keyword_loc().end_offset();
        let body = ensure.statements().map(|s| s.location().end_offset());
        body.map_or(keyword, |body| body.max(keyword))
    } else if let Some(else_clause) = begin.else_clause() {
        let keyword = else_clause.else_keyword_loc().end_offset();
        let body = else_clause.statements().map(|s| s.location().end_offset());
        body.map_or(keyword, |body| body.max(keyword))
    } else {
        let mut rescue = begin.rescue_clause()?;
        while let Some(subsequent) = rescue.subsequent() {
            rescue = subsequent;
        }
        rescue_clause_end(&rescue)
    };
    Some((start, end))
}

/// Whether only whitespace (or a leading byte-order mark) precedes `offset`
/// on its line, i.e. a comment there sits on a comment-only line.
pub(crate) fn starts_line(file: &SourceFile, offset: usize) -> bool {
    let line_start = file.line_start(file.line_of(offset));
    let prefix = &file.bytes[line_start..offset];
    let prefix = match line_start {
        0 => prefix.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(prefix),
        _ => prefix,
    };
    prefix.iter().all(|&byte| is_space(byte))
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\x0B' | b'\x0C')
}

fn starts_unicode_word(text: &[u8]) -> bool {
    static WORD: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {
        regex::bytes::Regex::new(r"\A[\p{L}\p{M}\p{N}\p{Pc}]").expect("Ruby word boundary")
    });
    WORD.is_match(text)
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn skip_space(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|&b| !(b.is_ascii_whitespace() || b == b'\x0B'));
    &text[start.unwrap_or(text.len())..]
}

/// Where the first directive marker in `text` starts: the offset of its
/// `#`. Matches what RuboCop's `DirectiveComment::DIRECTIVE_COMMENT_REGEXP`
/// matches, in either spelling: `# rubocop:` or `# rustocop:` and any mode
/// RuboCop knows, with or without a cop list. Works on any text, so it
/// also finds the directive in a whole source line (RuboCop's
/// `DirectiveComment.before_comment`).
pub fn marker_start(text: &[u8]) -> Option<usize> {
    find_marker(text).map(|(hash, _, _)| hash)
}

/// Whether a comment's text holds a directive in either spelling, whatever
/// its mode and whether or not it names cops, unless it is commented out:
/// what RuboCop's `DirectiveComment#match_captures` accepts.
pub fn is_directive_comment(text: &[u8]) -> bool {
    find_marker(text).is_some_and(|(hash, _, _)| !is_commented_out(&text[..hash]))
}

/// The first directive marker in `text`: the offset of its `#`, its mode,
/// and the text after the mode.
fn find_marker(text: &[u8]) -> Option<(usize, Mode, &[u8])> {
    // Cheap reject for ordinary comments.
    memchr::memmem::find(text, b"ocop")?;
    memchr::memchr_iter(b'#', text).find_map(|hash| {
        let (mode, rest) = parse_marker(&text[hash + 1..])?;
        Some((hash, mode, rest))
    })
}

/// Whether the comment text before a directive marker is just `#` and
/// blanks: the directive itself is commented out.
fn is_commented_out(before: &[u8]) -> bool {
    before.first() == Some(&b'#')
        && before[1..]
            .iter()
            .all(|&b| b.is_ascii_whitespace() || b == b'\x0B')
}

/// Parse `\s*(rubocop|rustocop)\s*:\s*MODE\b`, returning the mode and the
/// text after it.
fn parse_marker(text: &[u8]) -> Option<(Mode, &[u8])> {
    let text = skip_space(text);
    let text = text
        .strip_prefix(b"rubocop")
        .or_else(|| text.strip_prefix(b"rustocop"))?;
    let text = skip_space(skip_space(text).strip_prefix(b":")?);
    MODES.iter().find_map(|&(spelling, mode)| {
        let rest = text.strip_prefix(spelling)?;
        match rest.first() {
            Some(&byte) if is_word(byte) || (byte >= 0x80 && starts_unicode_word(rest)) => None,
            _ => Some((mode, rest)),
        }
    })
}

/// Parse `\s+(all|NAME(\s*,\s*NAME)*)` at the start of `text`.
fn parse_targets(text: &[u8]) -> Option<Targets> {
    let rest = skip_space(text);
    if rest.len() == text.len() {
        return None; // no targets
    }
    // Like RuboCop's `(all|NAMES)`: no word boundary after `all`, so
    // `allowed_cop` means `all`.
    if rest.starts_with(b"all") {
        return Some(Targets::All);
    }
    let mut names = Vec::new();
    let mut rest = rest;
    loop {
        let len = cop_name_len(rest)?;
        names.push(String::from_utf8_lossy(&rest[..len]).into_owned());
        rest = &rest[len..];
        let Some(after_comma) = skip_space(rest).strip_prefix(b",") else {
            break;
        };
        let after_comma = skip_space(after_comma);
        if cop_name_len(after_comma).is_none() {
            break;
        }
        rest = after_comma;
    }
    Some(Targets::List(names))
}

/// The length of the cop name `([A-Za-z]\w+/)*[A-Za-z]\w+` at the start of
/// `text`.
fn cop_name_len(text: &[u8]) -> Option<usize> {
    let segment_len = |text: &[u8]| -> Option<usize> {
        if !text.first()?.is_ascii_alphabetic() {
            return None;
        }
        let len = 1 + text[1..].iter().take_while(|&&byte| is_word(byte)).count();
        (len >= 2).then_some(len)
    };
    let mut end = 0;
    let mut pos = 0;
    while let Some(len) = segment_len(&text[pos..]) {
        pos += len;
        end = pos;
        if text.get(pos) != Some(&b'/') {
            break;
        }
        pos += 1;
    }
    (end > 0).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directives_for(source: &[u8]) -> Directives {
        let file = SourceFile::new(std::path::PathBuf::from("test.rb"), source.to_vec());
        let result = ruby_prism::parse(&file.bytes);
        Directives::parse(&result, &file)
    }

    #[test]
    fn trailing_disable_on_last_line_suppresses_multiline_offense() {
        for prefix in ["rubocop", "rustocop"] {
            let source = format!("binding.pry(\n  value\n) # {prefix}:disable Lint/Debugger\n");
            assert!(debugger_lines(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn overlapping_disable_does_not_hide_a_later_independent_offense() {
        for prefix in ["rubocop", "rustocop"] {
            for source in [
                format!("binding.pry(\n  value\n) # {prefix}:disable Lint/Debugger\nbinding.pry\n"),
                format!(
                    "binding.pry(\n  # {prefix}:disable-next Lint/Debugger\n  value)\nbinding.pry\n"
                ),
            ] {
                assert_eq!(debugger_lines(&source), [4], "{source}");
            }
        }
    }

    // Ported from RuboCop's spec/rubocop/comment_config_spec.rb
    // (`#cop_enabled_at_lines?`). MIT, Copyright (c) 2012-26 Bozhidar Batsov.
    #[test]
    fn rubocop_span_overlap_examples() {
        let directives = directives_for(
            b"def foo(a,\n        b) # rubocop:disable Metrics/ParameterLists\nend\n",
        );
        assert!(!directives.is_enabled_for_lines("Metrics/ParameterLists", 1, 2));
        assert!(directives.is_enabled_for_lines("Metrics/ParameterLists", 3, 3));
        assert!(directives.is_enabled_for_lines("Style/ClassVars", 1, 3));
    }

    #[test]
    fn span_checks_match_every_line_for_all_directive_scopes() {
        let directives = directives_for(
            b"before\n# rubocop:disable Lint\nwork\n# rubocop:enable Debugger\n\
              after\nx # rustocop:todo all\nafter\n# rubocop:disable-next Semicolon\n\
              foo(\n  bar\n)\nafter\n",
        );
        for cop in [
            "Lint/Debugger",
            "Lint/Void",
            "Style/Semicolon",
            "Unknown/Cop",
        ] {
            for first in 1..=14 {
                for last in first..=14 {
                    assert_eq!(
                        directives.is_enabled_for_lines(cop, first, last),
                        (first..=last).all(|line| directives.is_enabled(cop, line)),
                        "{cop} {first}..={last}"
                    );
                }
            }
        }
        assert!(Directives::default().is_enabled_for_lines("Lint/Debugger", 1, u32::MAX));
    }

    #[test]
    fn interior_disable_suppresses_even_when_both_ends_are_enabled() {
        for prefix in ["rubocop", "rustocop"] {
            for target in ["Lint/Debugger", "Debugger", "Lint", "all"] {
                let source =
                    format!("binding.pry(\n  value, # {prefix}:disable {target}\n  other\n)\n");
                assert!(debugger_lines(&source).is_empty(), "{source}");
            }
            let source = format!(
                "binding.pry(\n  # {prefix}:disable Lint/Debugger\n  value\n  # {prefix}:enable Lint/Debugger\n)\n"
            );
            assert!(debugger_lines(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn disables_outside_the_highlight_or_for_other_cops_do_not_suppress() {
        for source in [
            "binding.pry(\n  value\n) # rubocop:disable Lint/Void\n",
            "binding.pry(\n  value\n)\n# rubocop:disable Lint/Debugger\n",
            "binding.pry do\n  value # rubocop:disable Lint/Debugger\nend\n",
            "binding.pry do\n  value\nend # rubocop:disable Lint/Debugger\n",
            "before # rubocop:disable Lint/Debugger\nbinding.pry(\n  value\n)\n",
        ] {
            assert_eq!(debugger_lines(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn honors_rustocop_disable_by_cop() {
        let directives = directives_for(b"# rustocop:disable Lint/Debugger\nbinding.pry\n");
        assert!(!directives.is_enabled("Lint/Debugger", 1));
        assert!(!directives.is_enabled("Lint/Debugger", 2));
        assert!(directives.is_enabled("Lint/Void", 2));
    }

    #[test]
    fn finds_directive_markers_in_lines() {
        assert_eq!(marker_start(b"foo # rubocop:disable Foo/Bar"), Some(4));
        assert_eq!(marker_start(b"# rustocop : enable all"), Some(0));
        assert_eq!(marker_start(b"x # a # rubocop:todo Foo"), Some(6));
        // Any mode RuboCop knows; the cop list is optional.
        assert_eq!(marker_start(b"x # rubocop:push"), Some(2));
        assert_eq!(marker_start(b"x # rubocop:disable"), Some(2));
        assert_eq!(marker_start(b"x # rubocop:disabled Foo"), None);
        assert_eq!(marker_start(b"x # rubocop: Foo"), None);
        assert_eq!(marker_start(b"x # robocop:disable Foo"), None);
    }

    #[test]
    fn tells_directive_comments_from_commented_out_ones() {
        assert!(is_directive_comment(b"# rubocop:disable Foo/Bar"));
        assert!(is_directive_comment(b"# rustocop:enable Foo -- why"));
        assert!(is_directive_comment(b"# note # rubocop:disable Foo"));
        assert!(is_directive_comment(b"# rubocop:todo Foo"));
        assert!(!is_directive_comment(b"# # rubocop:disable Foo"));
        assert!(!is_directive_comment(b"# just a comment"));
    }

    #[test]
    fn department_less_names_disable_and_enable_ranges() {
        for prefix in ["rubocop", "rustocop"] {
            let source = format!(
                "if a = foo; end\n# {prefix}:disable AssignmentInCondition\n\
                 if b = foo; end\n# {prefix}:enable AssignmentInCondition\n\
                 if c = foo; end\n"
            );
            let directives = directives_for(source.as_bytes());
            assert_eq!(
                disabled_lines(&directives, "Lint/AssignmentInCondition", 5),
                [2, 3, 4]
            );
            assert!(directives.is_enabled("Lint/Debugger", 3));
            let positions: Vec<_> = crate::pipeline::lint_str(&source)
                .into_iter()
                .filter(|offense| offense.cop == "Lint/AssignmentInCondition")
                .map(|offense| (offense.line, offense.col))
                .collect();
            assert_eq!(positions, [(1, 6), (5, 6)]);
        }
    }

    #[test]
    fn department_less_trailing_directives_are_single_line() {
        for prefix in ["rubocop", "rustocop"] {
            let source = format!(
                "binding.pry # {prefix}:disable Debugger -- temporary\n\
                 binding.pry\n# {prefix}:disable Debugger\n\
                 binding.pry # {prefix}:enable Debugger\nbinding.pry\n\
                 # {prefix}:enable Debugger\nbinding.pry\n"
            );
            assert_eq!(
                disabled_lines(&directives_for(source.as_bytes()), "Lint/Debugger", 7),
                [1, 3, 4, 5, 6]
            );
            assert_eq!(debugger_lines(&source), [2, 7]);
        }
    }

    #[test]
    fn department_less_and_qualified_names_can_be_mixed() {
        for prefix in ["rubocop", "rustocop"] {
            for mode in ["disable", "todo"] {
                for (disable, enable) in [
                    ("Debugger, Lint/Void", "Lint/Debugger, Void"),
                    ("Lint/Debugger, Void", "Debugger, Lint/Void"),
                ] {
                    let source = format!(
                        "# {prefix}:{mode} {disable} -- legacy\nbinding.pry\n\
                         # {prefix}:enable {enable}\nbinding.pry\n"
                    );
                    let directives = directives_for(source.as_bytes());
                    for cop in ["Lint/Debugger", "Lint/Void"] {
                        assert_eq!(disabled_lines(&directives, cop, 4), [1, 2, 3]);
                    }
                    assert!(directives.is_enabled("Style/Semicolon", 2));
                    assert_eq!(debugger_lines(&source), [4]);
                }
            }
        }
    }

    #[test]
    fn department_less_disable_next_covers_the_whole_statement() {
        for prefix in ["rubocop", "rustocop"] {
            let source = format!(
                "# {prefix}:disable-next Debugger\n\
                 if foo\n  binding.pry\nend\nbinding.pry\n"
            );
            assert_eq!(
                disabled_lines(&directives_for(source.as_bytes()), "Lint/Debugger", 5),
                [2, 3, 4]
            );
            assert_eq!(debugger_lines(&source), [5]);
        }
    }

    #[test]
    fn department_less_targets_match_exact_names_only() {
        for target in [
            "Debug",
            "DebuggerExtra",
            "debugger",
            "Other/Debugger",
            "Lint/Other/Debugger",
            "UnknownCop",
        ] {
            let source = format!("# rubocop:disable {target}\nbinding.pry\n");
            assert!(directives_for(source.as_bytes()).is_enabled("Lint/Debugger", 2));
            assert_eq!(debugger_lines(&source), [2], "{target}");
        }
    }

    #[test]
    fn honors_legacy_rubocop_prefix() {
        let directives = directives_for(b"# rubocop:disable Lint/Debugger\nbinding.pry\n");
        assert!(!directives.is_enabled("Lint/Debugger", 2));
    }

    #[test]
    fn todo_opens_a_range_until_enable_or_eof() {
        for prefix in ["rubocop", "rustocop"] {
            let source = format!(
                "# {prefix}:todo Lint/Debugger\nbinding.pry\n\
                 # {prefix}:enable Lint/Debugger\nbinding.pry\n"
            );
            let directives = directives_for(source.as_bytes());
            assert_eq!(disabled_lines(&directives, "Lint/Debugger", 4), [1, 2, 3]);
            assert!(directives.is_enabled("Lint/Void", 2));
            assert_eq!(debugger_lines(&source), [4]);

            let source = format!("# {prefix}:todo Lint/Debugger\nbinding.pry\n");
            assert_eq!(
                disabled_lines(&directives_for(source.as_bytes()), "Lint/Debugger", 3),
                [1, 2, 3]
            );
            assert!(debugger_lines(&source).is_empty());
        }
    }

    #[test]
    fn todo_after_code_or_comment_text_only_disables_its_own_line() {
        for prefix in ["rubocop", "rustocop"] {
            for before in ["binding.pry ", "# note "] {
                let source =
                    format!("{before}# {prefix}:todo Lint/Debugger -- later\nbinding.pry\n");
                assert_eq!(
                    disabled_lines(&directives_for(source.as_bytes()), "Lint/Debugger", 3),
                    [1]
                );
                assert_eq!(debugger_lines(&source), [2]);
            }
        }
    }

    #[test]
    fn todo_accepts_departments_all_and_cop_lists_with_reasons() {
        for prefix in ["rubocop", "rustocop"] {
            for target in ["Lint", "all", "Lint/Debugger, Lint/Void"] {
                let source = format!("# {prefix}:todo {target} -- legacy\nbinding.pry\n");
                let directives = directives_for(source.as_bytes());
                assert!(!directives.is_enabled("Lint/Debugger", 2));
                assert!(!directives.is_enabled("Lint/Void", 2));
                assert_eq!(directives.is_enabled("Style/Semicolon", 2), target != "all");
                assert!(debugger_lines(&source).is_empty());
            }
        }
    }

    #[test]
    fn todo_lookalikes_and_non_comments_do_not_suppress() {
        for prefix in ["rubocop", "rustocop"] {
            for comment in [
                format!("# {prefix}:todo"),
                format!("# {prefix}:todos Lint/Debugger"),
                format!("# {prefix}:todo_next Lint/Debugger"),
                format!("# # {prefix}:todo Lint/Debugger"),
                format!("## {prefix}:todo Lint/Debugger"),
                format!("text = '# {prefix}:todo Lint/Debugger'"),
            ] {
                let source = format!("{comment}\nbinding.pry\n");
                assert!(directives_for(source.as_bytes()).is_empty(), "{source}");
                assert_eq!(debugger_lines(&source), [2], "{source}");
            }
        }
    }

    #[test]
    fn honors_department_and_all() {
        let directives = directives_for(b"# rustocop:disable Lint\n# rustocop:disable all\n");
        assert!(!directives.is_enabled("Lint/Debugger", 1));
        assert!(!directives.is_enabled("Style/Semicolon", 2));
    }

    #[test]
    fn enable_re_enables() {
        let directives =
            directives_for(b"# rustocop:disable Lint/Debugger\n# rustocop:enable Lint/Debugger\n");
        assert!(!directives.is_enabled("Lint/Debugger", 1));
        // Like RuboCop, the `enable` line still belongs to the range.
        assert!(!directives.is_enabled("Lint/Debugger", 2));
        assert!(directives.is_enabled("Lint/Debugger", 3));
    }

    #[test]
    fn accepts_reasons_and_multiple_cops() {
        let directives = directives_for(
            b"# rustocop:disable Lint/Debugger, Lint/Void -- still debugging this spec\n",
        );
        assert!(!directives.is_enabled("Lint/Debugger", 1));
        assert!(!directives.is_enabled("Lint/Void", 1));
    }

    #[test]
    fn ignores_plain_comments() {
        let directives = directives_for(b"# we disabled Lint/Debugger because reasons\n");
        assert!(directives.is_enabled("Lint/Debugger", 1));
        assert!(directives.is_empty());
    }

    #[test]
    fn ignores_bare_disable_without_targets() {
        let directives = directives_for(b"# rustocop:disable\n");
        assert!(directives.is_empty());
    }

    /// The lines in `1..=max` on which `cop` is disabled.
    fn disabled_lines(directives: &Directives, cop: &str, max: u32) -> Vec<u32> {
        (1..=max)
            .filter(|&line| !directives.is_enabled(cop, line))
            .collect()
    }

    fn debugger_lines(source: &str) -> Vec<u32> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|offense| offense.cop == "Lint/Debugger")
            .map(|offense| offense.line)
            .collect()
    }

    #[test]
    fn trailing_disable_applies_to_its_own_line_only() {
        let directives =
            directives_for(b"binding.pry # rubocop:disable Lint/Debugger\nbinding.pry\n");
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 5), [1]);
        let directives = directives_for(b"def foo\n  x # rustocop:disable Lint/Void\nend\n");
        assert_eq!(disabled_lines(&directives, "Lint/Void", 5), [2]);
    }

    #[test]
    fn trailing_enable_is_a_no_op() {
        let directives = directives_for(
            b"# rubocop:disable Lint/Debugger\nfoo # rubocop:enable Lint/Debugger\nbar\n",
        );
        assert_eq!(
            disabled_lines(&directives, "Lint/Debugger", 4),
            [1, 2, 3, 4]
        );
        let directives = directives_for(b"foo # rubocop:enable Lint/Debugger\n");
        assert!(disabled_lines(&directives, "Lint/Debugger", 2).is_empty());
    }

    #[test]
    fn indented_comment_only_directive_opens_a_range() {
        let directives = directives_for(b"class A\n  # rubocop:disable Lint/Void\n  x\nend\ny\n");
        assert_eq!(disabled_lines(&directives, "Lint/Void", 6), [2, 3, 4, 5, 6]);
    }

    #[test]
    fn enable_line_itself_is_still_disabled() {
        // RuboCop closes the range on the `enable` line (inclusive).
        let directives =
            directives_for(b"# rubocop:disable Lint/Void\nx\n# rubocop:enable Lint/Void\ny\n");
        assert_eq!(disabled_lines(&directives, "Lint/Void", 5), [1, 2, 3]);
    }

    #[test]
    fn directive_after_other_comment_text_is_single_line() {
        let directives = directives_for(
            b"#=SomeDslDirective # rubocop:disable Lint/Void\nx\n# note # rubocop:enable Lint/Debugger\n",
        );
        assert_eq!(disabled_lines(&directives, "Lint/Void", 3), [1]);
        assert!(disabled_lines(&directives, "Lint/Debugger", 3).is_empty());
    }

    #[test]
    fn commented_out_directives_are_ignored() {
        for source in [
            b"# # rubocop:disable Lint/Debugger\nbinding.pry\n".as_slice(),
            b"## rubocop:disable Lint/Debugger\nbinding.pry\n",
        ] {
            assert!(directives_for(source).is_empty());
        }
    }

    #[test]
    fn cop_list_ends_where_names_end() {
        let directives = directives_for(
            b"# rubocop:disable Lint/Void, Lint/Debugger some comment why\n\
              # rubocop:enable Lint/Void but why?\n",
        );
        assert_eq!(disabled_lines(&directives, "Lint/Void", 3), [1, 2]);
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 3), [1, 2, 3]);
        let directives = directives_for(b"# rubocop:disable Lint/Void,Lint/Debugger-- why\n");
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 1), [1]);
    }

    #[test]
    fn marker_tolerates_whitespace() {
        let directives = directives_for(b"#rubocop : disable Lint/Debugger\n");
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 2), [1, 2]);
    }

    #[test]
    fn detached_next_and_invalid_modes_are_ignored() {
        for source in [
            b"# rubocop:enable-next Lint/Debugger\n".as_slice(),
            b"# rubocop:disablefoo Lint/Debugger\n",
            b"# rubocop:disable_next Lint/Debugger\n",
        ] {
            assert!(directives_for(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn all_need_not_be_a_whole_word() {
        // RuboCop 1.92's `(all|NAMES)` has no word boundary after `all`.
        for source in [
            &b"# rubocop:disable allowed_cop\n"[..],
            b"# rubocop:disable all_cops\n",
        ] {
            assert!(!directives_for(source).is_enabled("Lint/Debugger", 2));
        }
        let directives = directives_for(b"# rubocop:disable all -- legacy file\n");
        assert!(!directives.is_enabled("Lint/Debugger", 2));
    }

    #[test]
    fn directive_text_inside_a_string_is_not_a_comment() {
        let directives =
            directives_for(b"x = <<~END\n  # rubocop:disable Lint/Debugger\nEND\nbinding.pry\n");
        assert!(directives.is_empty());
    }

    #[test]
    fn comment_only_first_line_after_a_bom_opens_a_range() {
        let directives = directives_for(b"\xEF\xBB\xBF# rubocop:disable Lint/Debugger\nx\n");
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 2), [1, 2]);
    }

    #[test]
    fn task_repro_matches_rubocop() {
        let source = "binding.pry # rubocop:disable Lint/Debugger\nbinding.pry\n\
                      # rubocop:disable-next Lint/Debugger\nbinding.pry\nbinding.pry\n";
        assert_eq!(debugger_lines(source), [2, 5]);
    }

    #[test]
    fn disable_next_covers_the_next_statement_only() {
        let source = "# rustocop:disable-next Lint/Debugger\n\
                      foo(1,\n    binding.pry)\nbinding.pry\n";
        assert_eq!(debugger_lines(source), [4]);
        let directives = directives_for(source.as_bytes());
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 5), [2, 3]);
        assert!(directives.is_enabled("Lint/Void", 2));
    }

    #[test]
    fn disable_next_covers_a_multi_line_heredoc() {
        // RuboCop's own spec/rubocop/comment_config_spec.rb does this.
        let source = "let(:source) do\n\
                      \x20 # rubocop:disable-next Lint/EmptyInterpolation -- what it is\n\
                      \x20 <<~RUBY\n    \"result is #{}\"\n  RUBY\nend\n\"#{}\"\n";
        let found: Vec<u32> = crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|offense| offense.cop == "Lint/EmptyInterpolation")
            .map(|offense| offense.line)
            .collect();
        assert_eq!(found, [7]);
    }

    #[test]
    fn disable_next_on_a_department_and_all() {
        let directives = directives_for(b"# rubocop:disable-next Lint\nx\ny\n");
        assert_eq!(disabled_lines(&directives, "Lint/Void", 3), [2]);
        assert!(directives.is_enabled("Style/Semicolon", 2));
        let directives = directives_for(b"# rubocop:disable-next all\nx\ny\n");
        assert_eq!(disabled_lines(&directives, "Style/Semicolon", 3), [2]);
    }

    #[test]
    fn disable_next_after_other_comment_text_still_scopes_to_the_statement() {
        let directives = directives_for(
            b"# note # rubocop:disable-next Lint/Debugger\nfoo(<<~A, <<~B)\n  a\nA\n  b\nB\nz\n",
        );
        assert_eq!(
            disabled_lines(&directives, "Lint/Debugger", 7),
            [2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn disable_next_scopes_like_rubocop_around_clauses() {
        // Every expectation is RuboCop 1.92's `cop_disabled_line_ranges`.
        let source = b"begin\n\
                       \x20 # rubocop:disable-next Lint/Debugger\n\
                       \x20 foo\n\
                       rescue A\n  bar\nelse\n  baz\nensure\n  qux\nend\n\
                       case x\n\
                       # rubocop:disable-next Lint/Void\n\
                       when 1 then\n  a\n\
                       # rubocop:disable-next Lint/Debugger\n\
                       else\n  b\nend\n\
                       if a\n  b\n\
                       # rubocop:disable-next Lint/Void\n\
                       elsif c\n  d\nend\n\
                       begin\n  x\n\
                       # rubocop:disable-next Lint/EmptyBlock\n\
                       rescue A => e\n  x\n  y\nrescue B\n  z\nend\n";
        let directives = directives_for(source);
        let debugger = [span(3..=9), vec![16]].concat();
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 35), debugger);
        let void = [span(13..=14), span(22..=23)].concat();
        assert_eq!(disabled_lines(&directives, "Lint/Void", 35), void);
        assert_eq!(
            disabled_lines(&directives, "Lint/EmptyBlock", 35),
            span(28..=30)
        );
    }

    #[test]
    fn disable_next_scopes_a_rescue_wrapped_method_body() {
        // RuboCop's parser wraps a body with `rescue` in a node that starts
        // at its first statement.
        let directives = directives_for(
            b"def foo\n  # rubocop:disable-next Lint/Debugger\n  bar\nrescue A => e\n  baz\nend\n",
        );
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 6), [3, 4, 5]);
    }

    #[test]
    fn disable_next_before_an_implicit_parameter_block_continuation() {
        // RuboCop 1.92: the parser gem's `numblock`/`itblock` start at the
        // receiver, so the scope is the continuation line alone.
        for block in [
            "do\n    binding.pry\n    _1\n  end",
            "{\n    binding.pry\n    it\n  }",
        ] {
            let source = format!("foo\n  # rubocop:disable-next Lint/Debugger\n  .bar {block}\n");
            let directives = directives_for(source.as_bytes());
            assert_eq!(
                disabled_lines(&directives, "Lint/Debugger", 6),
                [3],
                "{block}"
            );
            assert_eq!(debugger_lines(&source), [4], "{block}");
        }
    }

    #[test]
    fn disable_next_before_rescue_or_ensure_of_an_empty_body() {
        // RuboCop 1.92: the wrapper starts at the first `rescue`/`ensure`.
        let directives = directives_for(
            b"def foo\n  # rubocop:disable-next Lint/Debugger\nrescue A\n  x\nrescue B\n  y\nend\n",
        );
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 7), span(3..=6));
        let directives =
            directives_for(b"begin\n  # rubocop:disable-next Lint/Debugger\nensure\n  x\nend\n");
        assert_eq!(disabled_lines(&directives, "Lint/Debugger", 5), [3, 4]);
    }

    #[test]
    fn disable_next_skips_unrelated_lookalikes() {
        // Not a directive mode, or a `disable` (a range, not a statement).
        let directives = directives_for(b"# rubocop:disable-nextLint/Debugger\nx\n");
        assert!(directives.is_empty());
        let directives = directives_for(b"# rubocop:disable next\nx\ny\n");
        assert!(directives.is_enabled("Lint/Debugger", 3));
        assert!(!directives.is_enabled("next", 3));
    }

    fn span(range: std::ops::RangeInclusive<u32>) -> Vec<u32> {
        range.collect()
    }

    // Ported from RuboCop's spec/rubocop/comment_config_spec.rb
    // (`disable-next directives`).
    // RuboCop is MIT licensed: Copyright (c) 2012-26 Bozhidar Batsov.
    mod rubocop_disable_next_spec {
        use super::*;

        fn lines(source: &str, cop: &str) -> Vec<u32> {
            disabled_lines(&directives_for(source.as_bytes()), cop, 20)
        }

        #[test]
        fn disables_the_cop_for_the_whole_following_statement() {
            let source = "# rubocop:disable-next Metrics/MethodLength\n\
                          def foo(a,\n        b)\n  puts a\nend\nputs 1\n";
            assert_eq!(lines(source, "Metrics/MethodLength"), span(2..=5));
        }

        #[test]
        fn stacked_directives_share_the_statement() {
            let source = "# rubocop:disable-next Metrics/MethodLength\n\
                          # rubocop:disable-next Metrics/AbcSize -- another reason\n\
                          def foo\n  puts 1\nend\n";
            assert_eq!(lines(source, "Metrics/MethodLength"), span(3..=5));
            assert_eq!(lines(source, "Metrics/AbcSize"), span(3..=5));
        }

        #[test]
        fn a_blank_line_detaches_the_directive() {
            let source = "# rubocop:disable-next Metrics/MethodLength\n\n\
                          def foo\n  puts 1\nend\n";
            assert!(lines(source, "Metrics/MethodLength").is_empty());
        }

        #[test]
        fn nothing_following_detaches_the_directive() {
            let source = "puts 1\n# rubocop:disable-next Metrics/MethodLength\n";
            assert!(lines(source, "Metrics/MethodLength").is_empty());
        }

        #[test]
        fn a_directive_at_the_end_of_a_code_line_is_not_honored() {
            let source = "puts 1 # rubocop:disable-next Metrics/MethodLength\nputs 2\n";
            assert!(lines(source, "Metrics/MethodLength").is_empty());
        }

        #[test]
        fn a_heredoc_extends_the_scope_to_its_end() {
            let source = "# rubocop:disable-next Layout/LineLength\n\
                          foo(<<~TEXT)\n  some text\nTEXT\nputs 1\n";
            assert_eq!(lines(source, "Layout/LineLength"), span(2..=4));
        }

        #[test]
        fn several_statements_on_the_target_line() {
            let source = "# rubocop:disable-next Style/Semicolon\na = 1; b = 2\nc = 3\n";
            assert_eq!(lines(source, "Style/Semicolon"), [2]);
        }

        #[test]
        fn a_when_clause_scopes_to_the_clause() {
            let source = "case foo\n# rubocop:disable-next Lint/EmptyWhen\n\
                          when :a\n  nil\nwhen :b\n  puts 1\nend\n";
            assert_eq!(lines(source, "Lint/EmptyWhen"), span(3..=4));
        }

        #[test]
        fn a_line_starting_no_statement_scopes_to_itself() {
            let source =
                "def foo\n  puts 1\n# rubocop:disable-next Layout/DefEndAlignment\n  end\n";
            assert_eq!(lines(source, "Layout/DefEndAlignment"), [4]);
        }

        #[test]
        fn chains_past_other_comment_lines() {
            let source = "# rubocop:disable-next Style/ClassVars\n# rubocop:push\n\
                          @@a = 1\n# rubocop:pop\n@@b = 2\n";
            assert_eq!(lines(source, "Style/ClassVars"), [3]);
        }

        #[test]
        fn a_department_covers_its_cops() {
            // RuboCop's example spells this `todo-next`.
            let source = "# rubocop:disable-next Style\n@@foo = 1\n@@bar = 2\n";
            assert_eq!(lines(source, "Style/ClassVars"), [2]);
            assert_eq!(lines(source, "Style/FrozenStringLiteralComment"), [2]);
        }
    }

    // Ported from RuboCop's spec/rubocop/comment_config_spec.rb
    // (`#cop_enabled_at_line?`, `#cop_disabled_line_ranges`).
    // RuboCop is MIT licensed: Copyright (c) 2012-26 Bozhidar Batsov.
    mod rubocop_spec {
        use super::*;

        const SOURCE: &str = r#"# rubocop:disable Metrics/MethodLength with a comment why
def some_method
  puts 'foo'                                                        # 03
end
# rubocop:enable Metrics/MethodLength

# rubocop:disable all
some_method                                                         # 08
# rubocop:enable all

code = 'This is evil.'
eval(code) # rubocop:disable Security/Eval
puts 'This is not evil.'                                            # 12

def some_method
  puts 'Disabling indented single line' # rubocop:disable Layout/LineLength
end
                                                                    # 18
string = <<~END
This is a string not a real comment # rubocop:disable Style/Loop
END

foo # rubocop:disable Style/MethodCallWithoutArgsParentheses        # 23

# rubocop:enable Lint/Void

# rubocop:disable Style/For, Style/Not,Layout/IndentationStyle
foo                                                                 # 28

class One
  # rubocop:disable Style/ClassVars
  @@class_var = 1
end                                                                 # 33

class Two
  # rubocop:disable Style/ClassVars
  @@class_var = 2
end                                                                 # 38
# rubocop:enable Style/Not,Layout/IndentationStyle
# rubocop:disable Style/Send, Lint/RandOne some comment why
# rubocop:disable Layout/BlockAlignment some comment why
# rubocop:enable Style/Send, Layout/BlockAlignment but why?
# rubocop:enable Lint/RandOne foo bar!                              # 43
# rubocop:disable Lint/EmptyInterpolation
"result is #{}"
# rubocop:enable Lint/EmptyInterpolation
# rubocop:disable RSpec/Example
# rubocop:disable Custom2/Number9                                   # 48

#=SomeDslDirective # rubocop:disable Layout/LeadingCommentSpace
# rubocop:disable RSpec/Rails/HttpStatus
it { is_expected.to have_http_status 200 }                          # 52
# rubocop:enable RSpec/Rails/HttpStatus
"#;

        /// Past the last line, so open ranges show as running to the end.
        const MAX: u32 = 60;

        fn lines(cop: &str) -> Vec<u32> {
            disabled_lines(&directives_for(SOURCE.as_bytes()), cop, MAX)
        }

        fn concat(parts: &[Vec<u32>]) -> Vec<u32> {
            parts.concat()
        }

        #[test]
        fn matches_rubocop_disabled_ranges() {
            // Every expectation is RuboCop 1.92's `cop_disabled_line_ranges`
            // for this source.
            let all = span(7..=9);
            let cases: [(&str, Vec<u32>); 17] = [
                ("Metrics/MethodLength", concat(&[span(1..=5), all.clone()])),
                ("Security/Eval", concat(&[all.clone(), vec![12]])),
                ("Layout/LineLength", concat(&[all.clone(), vec![16]])),
                (
                    "Style/MethodCallWithoutArgsParentheses",
                    concat(&[all.clone(), vec![23]]),
                ),
                ("Lint/Void", all.clone()),
                ("Style/For", concat(&[all.clone(), span(27..=MAX)])),
                ("Style/Not", concat(&[all.clone(), span(27..=39)])),
                (
                    "Layout/IndentationStyle",
                    concat(&[all.clone(), span(27..=39)]),
                ),
                ("Style/ClassVars", concat(&[all.clone(), span(31..=MAX)])),
                ("Style/Send", concat(&[all.clone(), span(40..=42)])),
                ("Lint/RandOne", concat(&[all.clone(), span(40..=43)])),
                (
                    "Layout/BlockAlignment",
                    concat(&[all.clone(), span(41..=42)]),
                ),
                (
                    "Lint/EmptyInterpolation",
                    concat(&[all.clone(), span(44..=46)]),
                ),
                (
                    "Layout/LeadingCommentSpace",
                    concat(&[all.clone(), vec![50]]),
                ),
                (
                    "RSpec/Rails/HttpStatus",
                    concat(&[all.clone(), span(51..=53)]),
                ),
                ("Style/Loop", all.clone()),
                ("Lint/Debugger", all),
            ];
            for (cop, expected) in cases {
                assert_eq!(lines(cop), expected, "{cop}");
            }
        }

        #[test]
        fn multi_level_and_numbered_names() {
            assert!(lines("RSpec/Example").contains(&47));
            assert!(lines("Custom2/Number9").contains(&48));
        }

        #[test]
        fn cop_disabled_line_ranges_example() {
            let directives = directives_for(
                b"# rubocop:disable Metrics/MethodLength with a comment why\n\
                  def some_method\n  puts 'foo'\nend\n\
                  # rubocop:enable Metrics/MethodLength\n\n\
                  code = 'This is evil.'\n\
                  eval(code) # rubocop:disable Security/Eval\n\
                  puts 'This is not evil.'\n",
            );
            assert_eq!(
                disabled_lines(&directives, "Metrics/MethodLength", 10),
                span(1..=5)
            );
            assert_eq!(disabled_lines(&directives, "Security/Eval", 10), [8]);
        }
    }
}
