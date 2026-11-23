//! `Style/Semicolon` — flags semicolons that terminate an expression or
//! separate expressions on one line.
//!
//! Matches RuboCop's semantics with its default
//! `AllowAsExpressionSeparator: false`. RuboCop works on the token stream;
//! rustocop has no lexer, so it rebuilds the tokens of each line that holds
//! a semicolon from the AST (literals, interpolation and block braces) and a
//! light scan of the code in between. A semicolon token is reported when:
//!
//! - it is the last token on its line (`foo;`; but not `foo; # note`), else
//!   the first (`; foo`), else it sits before a closing `}` that ends the
//!   line (`foo { bar; }`), else right after the opening `{`, `->{` or `#{`
//!   that is the line's second token (`foo {; bar }`, `"#{;bar}"`), else
//!   before the `}` that closes an interpolation ending the line
//!   (`"#{foo;}"`); only the first of these checks that matches counts, so
//!   a line yields at most one such semicolon; or
//! - two expressions of one statement sequence end on its line (`a = 1;
//!   b = 2`, `def foo(a) x(1); y(2); end`): then every semicolon token on
//!   that line is reported. The statements directly in a plain `begin…end`
//!   are not a sequence to RuboCop (`begin; a; b; end` passes).
//!
//! So one-line bodies (`def foo; end`, `class Foo < Bar; end`, `if x; y;
//! end`) pass. Semicolons in strings, symbols, regexps, comments, heredoc
//! bodies, `$;` and `__END__` data are not tokens. Offenses are reported at
//! the semicolon.

use std::{collections::BTreeSet, ops::Range};

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        ast::fingerprint_ignoring_grouping,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Style/Semicolon` cop.
pub struct Semicolon;

#[cfg(test)]
thread_local! {
    static CORRECTION_REPARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Cop for Semicolon {
    fn name(&self) -> &'static str {
        "Style/Semicolon"
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
        // Semicolons are tokens, not nodes: one pass over the whole file.
        let Some(program) = node.as_program_node() else {
            return;
        };
        // RuboCop skips a file without an AST (`processed_source.blank?`).
        if program.statements().body().is_empty() {
            return;
        }
        // Only a semicolon outside comments can be a token.
        let outside_comments =
            memchr::memchr_iter(b';', &file.bytes).any(|offset| !in_comment(file, offset));
        if !outside_comments {
            return;
        }
        let mut layout = Layout::default();
        walk(&mut layout, node);
        layout.tokens.sort_by_key(|token| token.range.start);
        let data_start = data_section_start(file, program.location().end_offset(), &layout);
        let semicolons = layout.semicolons(file, data_start);
        if semicolons.is_empty() {
            return;
        }

        let mut offending = Vec::new();
        let mut separators = BTreeSet::new();
        // Expressions sharing a line.
        for sequence in &layout.sequences {
            let mut lines: Vec<u32> = sequence
                .iter()
                .map(|range| file.line_of(range.end.saturating_sub(1).max(range.start)))
                .collect();
            lines.sort_unstable();
            let shared = lines.windows(2).filter(|pair| pair[0] == pair[1]);
            for pair in shared {
                let line = pair[0];
                for &offset in semicolons
                    .iter()
                    .filter(|&&offset| file.line_of(offset) == line)
                {
                    offending.push(offset);
                    separators.insert(offset);
                }
            }
        }
        // Terminators, one per line at most.
        let mut previous_line = 0;
        for &offset in &semicolons {
            let line = file.line_of(offset);
            if line == previous_line {
                continue;
            }
            previous_line = line;
            let tokens = layout.line_tokens(file, line);
            if let Some(index) = terminator_position(&tokens) {
                offending.push(tokens[index].start);
            }
        }

        offending.sort_unstable();
        offending.dedup();
        let first_offense = out.len();
        for offset in offending {
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                correction: cx
                    .corrections_requested()
                    .then(|| layout.correction(file, offset, separators.contains(&offset)))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: "Do not use semicolons to terminate expressions.".to_string(),
                line,
                end_line: line,
                col,
            });
        }
        if cx.corrections_requested() {
            verify_corrections(file, node, &mut out[first_offense..]);
        }
    }
}

/// Check disjoint proposals together, splitting only failing/overlapping batches.
/// The original fingerprint is computed once, not once per semicolon. Removing
/// terminators leaves line numbers alone; separators only add newlines, and the
/// line-value guard excludes any that could shift a later `__LINE__`.
fn verify_corrections(file: &SourceFile, program: &Node<'_>, offenses: &mut [Offense]) {
    let indices: Vec<_> = offenses
        .iter()
        .enumerate()
        .filter_map(|(index, offense)| offense.correction.as_ref().map(|_| index))
        .collect();
    if indices.is_empty() {
        return;
    }
    let expected = fingerprint_ignoring_grouping(program, &file.bytes);
    let mut pending = vec![indices];
    while let Some(group) = pending.pop() {
        let corrected = (|| {
            let mut bytes = Vec::with_capacity(file.bytes.len());
            let mut cursor = 0;
            for &index in &group {
                let edit = offenses[index].correction.as_ref().unwrap();
                if edit.range.start < cursor {
                    return None;
                }
                bytes.extend_from_slice(&file.bytes[cursor..edit.range.start]);
                bytes.extend_from_slice(edit.replacement.as_bytes());
                cursor = edit.range.end;
            }
            bytes.extend_from_slice(&file.bytes[cursor..]);
            Some(bytes)
        })();
        let same = corrected.is_some_and(|bytes| {
            #[cfg(test)]
            CORRECTION_REPARSES.set(CORRECTION_REPARSES.get() + 1);
            let parsed = crate::parser::parse_as(&bytes, file.ruby_version());
            parsed.errors().next().is_none()
                && parsed.depth_error().is_none()
                && fingerprint_ignoring_grouping(&parsed.node(), &bytes) == expected
        });
        if !same {
            if group.len() == 1 {
                offenses[group[0]].correction = None;
            } else {
                let half = group.len() / 2;
                pending.push(group[..half].to_vec());
                pending.push(group[half..].to_vec());
            }
        }
    }
}

/// The kinds of token RuboCop's checks tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `;` (`tSEMI`).
    Semi,
    /// A block's `{` (`tLCURLY`).
    LeftCurly,
    /// A `->` lambda's `{` (`tLAMBEG`).
    LambdaBegin,
    /// `#{` (`tSTRING_DBEG`).
    InterpolationBegin,
    /// A block's, lambda's or hash's `}` (`tRCURLY`).
    RightCurly,
    /// The `}` closing an interpolation (`tSTRING_DEND`).
    InterpolationEnd,
    /// A comment.
    Comment,
    /// The newline token (`tNL`) after an expression that ends a line.
    Newline,
    /// A regexp's closing delimiter and options: two tokens
    /// (`tSTRING_END`, `tREGEXP_OPT`), the second one possibly empty.
    RegexpEnd,
    /// Anything else.
    Other,
}

/// A token found through the AST: its byte range and kind.
#[derive(Debug, Clone)]
struct KnownToken {
    range: Range<usize>,
    kind: Kind,
}

/// A token on a line: its start offset and kind.
#[derive(Debug, Clone, Copy)]
struct Token {
    start: usize,
    kind: Kind,
}

/// RuboCop's `semicolon_position`: the index of the line's terminating
/// semicolon token, if any. The order of the checks matters.
fn terminator_position(tokens: &[Token]) -> Option<usize> {
    let kind = |index: isize| -> Option<Kind> {
        let index = if index < 0 {
            tokens.len().checked_sub(index.unsigned_abs())?
        } else {
            index as usize
        };
        tokens.get(index).map(|token| token.kind)
    };
    let len = tokens.len();
    if kind(-1) == Some(Kind::Semi) {
        Some(len - 1)
    } else if kind(0) == Some(Kind::Semi) {
        Some(0)
    } else if kind(-2) == Some(Kind::RightCurly) && kind(-3) == Some(Kind::Semi) {
        Some(len - 3)
    } else if matches!(
        kind(1),
        Some(Kind::LeftCurly | Kind::LambdaBegin | Kind::InterpolationBegin)
    ) && kind(2) == Some(Kind::Semi)
    {
        Some(2)
    } else if kind(2) == Some(Kind::LambdaBegin) && kind(3) == Some(Kind::Semi) {
        Some(3)
    } else if kind(-3) == Some(Kind::InterpolationEnd) && kind(-4) == Some(Kind::Semi) {
        Some(len - 4)
    } else {
        None
    }
}

/// What the AST tells about the file's tokens.
#[derive(Default)]
struct Layout {
    /// Tokens with a known extent or kind: literals (whose text may hold a
    /// `;` that is not a token), interpolation and brace delimiters.
    tokens: Vec<KnownToken>,
    /// The statements of every sequence of two or more statements that
    /// RuboCop's AST has as a `begin` node.
    sequences: Vec<Vec<Range<usize>>>,
    /// Per open node: whether it is a plain `begin…end` (no `rescue` or
    /// `ensure`), whose statements RuboCop's AST holds directly.
    open: Vec<bool>,
    /// Interpolated heredoc bodies, from their first part to their
    /// terminator. A squiggly heredoc's parts omit the indentation that
    /// the lexer still emits as a content token.
    heredoc_bodies: Vec<Range<usize>>,
    heredoc_openings: Vec<Range<usize>>,
    endless_ranges: Vec<Range<usize>>,
    /// Selector end and argument range for unparenthesized omitted-value calls.
    omitted_arguments: Vec<(usize, Range<usize>)>,
}

impl Layout {
    fn push(&mut self, range: Range<usize>, kind: Kind) {
        // A literal inside the one just recorded (the number in `1i`) is
        // part of its token.
        let nested = self
            .tokens
            .last()
            .is_some_and(|last| last.range.start <= range.start && range.end <= last.range.end);
        if range.start < range.end && !nested {
            self.tokens.push(KnownToken { range, kind });
        }
    }

    fn push_location(&mut self, location: &ruby_prism::Location<'_>, kind: Kind) {
        self.push(location.start_offset()..location.end_offset(), kind);
    }

    fn push_optional(&mut self, location: Option<ruby_prism::Location<'_>>, kind: Kind) {
        if let Some(location) = location {
            self.push_location(&location, kind);
        }
    }

    /// Record the tokens a node owns.
    fn record(&mut self, node: &Node<'_>) {
        self.record_correction_syntax(node);
        match node {
            Node::StringNode { .. } => {
                let string = node.as_string_node().unwrap();
                self.record_string(
                    &node.location(),
                    string.opening_loc(),
                    string.content_loc(),
                    string.closing_loc(),
                );
            }
            Node::XStringNode { .. } => {
                let string = node.as_x_string_node().unwrap();
                self.record_string(
                    &node.location(),
                    Some(string.opening_loc()),
                    string.content_loc(),
                    Some(string.closing_loc()),
                );
            }
            Node::RegularExpressionNode { .. } => {
                let regexp = node.as_regular_expression_node().unwrap();
                self.push_location(&regexp.opening_loc(), Kind::Other);
                self.push_location(&regexp.content_loc(), Kind::Other);
                self.push_location(&regexp.closing_loc(), Kind::RegexpEnd);
            }
            Node::MatchLastLineNode { .. } => {
                let regexp = node.as_match_last_line_node().unwrap();
                self.push_location(&regexp.opening_loc(), Kind::Other);
                self.push_location(&regexp.content_loc(), Kind::Other);
                self.push_location(&regexp.closing_loc(), Kind::RegexpEnd);
            }
            Node::InterpolatedStringNode { .. } => {
                let string = node.as_interpolated_string_node().unwrap();
                self.push_optional(string.opening_loc(), Kind::Other);
                self.push_optional(string.closing_loc(), Kind::Other);
                if let (Some(opening), Some(closing), Some(first)) = (
                    string.opening_loc(),
                    string.closing_loc(),
                    string.parts().iter().next(),
                ) && opening.as_slice().starts_with(b"<<")
                {
                    self.heredoc_bodies
                        .push(first.location().start_offset()..closing.start_offset());
                }
            }
            Node::InterpolatedXStringNode { .. } => {
                let string = node.as_interpolated_x_string_node().unwrap();
                self.push_location(&string.opening_loc(), Kind::Other);
                self.push_location(&string.closing_loc(), Kind::Other);
                if let Some(first) = string.parts().iter().next()
                    && string.opening_loc().as_slice().starts_with(b"<<")
                {
                    self.heredoc_bodies
                        .push(first.location().start_offset()..string.closing_loc().start_offset());
                }
            }
            Node::InterpolatedSymbolNode { .. } => {
                let symbol = node.as_interpolated_symbol_node().unwrap();
                self.push_optional(symbol.opening_loc(), Kind::Other);
                self.push_optional(symbol.closing_loc(), Kind::Other);
            }
            Node::InterpolatedRegularExpressionNode { .. } => {
                let regexp = node.as_interpolated_regular_expression_node().unwrap();
                self.push_location(&regexp.opening_loc(), Kind::Other);
                self.push_location(&regexp.closing_loc(), Kind::RegexpEnd);
            }
            Node::InterpolatedMatchLastLineNode { .. } => {
                let regexp = node.as_interpolated_match_last_line_node().unwrap();
                self.push_location(&regexp.opening_loc(), Kind::Other);
                self.push_location(&regexp.closing_loc(), Kind::RegexpEnd);
            }
            Node::EmbeddedStatementsNode { .. } => {
                let embedded = node.as_embedded_statements_node().unwrap();
                self.push_location(&embedded.opening_loc(), Kind::InterpolationBegin);
                self.push_location(&embedded.closing_loc(), Kind::InterpolationEnd);
            }
            Node::BlockNode { .. } => {
                let block = node.as_block_node().unwrap();
                if block.opening_loc().as_slice() == b"{" {
                    self.push_location(&block.opening_loc(), Kind::LeftCurly);
                    self.push_location(&block.closing_loc(), Kind::RightCurly);
                }
            }
            Node::LambdaNode { .. } => {
                let lambda = node.as_lambda_node().unwrap();
                if lambda.opening_loc().as_slice() == b"{" {
                    self.push_location(&lambda.opening_loc(), Kind::LambdaBegin);
                    self.push_location(&lambda.closing_loc(), Kind::RightCurly);
                }
            }
            Node::ArrayNode { .. } => {
                // A percent literal's delimiters may be `;`.
                let array = node.as_array_node().unwrap();
                if let Some(opening) = array.opening_loc()
                    && opening.as_slice().starts_with(b"%")
                {
                    self.push_location(&opening, Kind::Other);
                    self.push_optional(array.closing_loc(), Kind::Other);
                }
            }
            Node::HashNode { .. } => {
                let hash = node.as_hash_node().unwrap();
                self.push_location(&hash.opening_loc(), Kind::Other);
                self.push_location(&hash.closing_loc(), Kind::RightCurly);
            }
            // Leaves whose text could be misread by the code scan.
            Node::SymbolNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::GlobalVariableReadNode { .. }
            | Node::GlobalVariableTargetNode { .. }
            | Node::BackReferenceReadNode { .. }
            | Node::NumberedReferenceReadNode { .. } => {
                self.push_location(&node.location(), Kind::Other);
            }
            Node::GlobalVariableWriteNode { .. } => {
                let name = node.as_global_variable_write_node().unwrap().name_loc();
                self.push_location(&name, Kind::Other);
            }
            Node::GlobalVariableAndWriteNode { .. } => {
                let name = node.as_global_variable_and_write_node().unwrap().name_loc();
                self.push_location(&name, Kind::Other);
            }
            Node::GlobalVariableOrWriteNode { .. } => {
                let name = node.as_global_variable_or_write_node().unwrap().name_loc();
                self.push_location(&name, Kind::Other);
            }
            Node::GlobalVariableOperatorWriteNode { .. } => {
                let name = node
                    .as_global_variable_operator_write_node()
                    .unwrap()
                    .name_loc();
                self.push_location(&name, Kind::Other);
            }
            _ => {}
        }
    }

    /// Collect syntax that needs wrapping before a terminator can be removed.
    fn record_correction_syntax(&mut self, node: &Node<'_>) {
        if let Some(range) = node.as_range_node()
            && range.right().is_none()
        {
            self.endless_ranges
                .push(node.location().start_offset()..node.location().end_offset());
        }
        let opening = match node {
            Node::StringNode { .. } => node.as_string_node().unwrap().opening_loc(),
            Node::XStringNode { .. } => Some(node.as_x_string_node().unwrap().opening_loc()),
            Node::InterpolatedStringNode { .. } => {
                node.as_interpolated_string_node().unwrap().opening_loc()
            }
            Node::InterpolatedXStringNode { .. } => {
                Some(node.as_interpolated_x_string_node().unwrap().opening_loc())
            }
            _ => None,
        };
        if let Some(opening) = opening
            && opening.as_slice().starts_with(b"<<")
        {
            self.heredoc_openings
                .push(opening.start_offset()..opening.end_offset());
        }
        let (arguments, selector_end) = match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if call.opening_loc().is_some() {
                    return;
                }
                (
                    call.arguments(),
                    call.message_loc().map(|loc| loc.end_offset()),
                )
            }
            Node::YieldNode { .. } => {
                let call = node.as_yield_node().unwrap();
                if call.lparen_loc().is_some() {
                    return;
                }
                (call.arguments(), Some(call.keyword_loc().end_offset()))
            }
            Node::SuperNode { .. } => {
                let call = node.as_super_node().unwrap();
                if call.lparen_loc().is_some() {
                    return;
                }
                (call.arguments(), Some(call.keyword_loc().end_offset()))
            }
            _ => return,
        };
        let (Some(arguments), Some(selector_end)) = (arguments, selector_end) else {
            return;
        };
        let omitted = arguments
            .arguments()
            .iter()
            .last()
            .and_then(|arg| arg.as_keyword_hash_node())
            .and_then(|hash| hash.elements().iter().last())
            .and_then(|pair| pair.as_assoc_node())
            .is_some_and(|pair| pair.value().as_implicit_node().is_some());
        if omitted {
            self.omitted_arguments.push((
                selector_end,
                arguments.location().start_offset()..arguments.location().end_offset(),
            ));
        }
    }

    fn correction(&self, file: &SourceFile, offset: usize, separator: bool) -> Option<Correction> {
        if separator
            && self.heredoc_openings.iter().any(|range| {
                range.end <= offset && file.line_of(range.start) == file.line_of(offset)
            })
        {
            return None;
        }
        let line_end = file.line_start(file.line_of(offset) + 1);
        let newline = if file.bytes[..line_end].ends_with(b"\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let mut edits = vec![Correction {
            range: offset..offset + 1,
            replacement: if separator {
                newline.into()
            } else {
                String::new()
            },
        }];
        let adjacent = |end: usize| {
            end <= offset
                && file.bytes[end..offset]
                    .iter()
                    .all(|b| matches!(b, b' ' | b'\t'))
        };
        for range in self
            .endless_ranges
            .iter()
            .filter(|range| adjacent(range.end))
        {
            edits.push(Correction {
                range: range.start..range.start,
                replacement: "(".into(),
            });
            edits.push(Correction {
                range: range.end..range.end,
                replacement: ")".into(),
            });
        }
        for (selector_end, arguments) in self
            .omitted_arguments
            .iter()
            .filter(|(_, range)| adjacent(range.end))
        {
            // A comment between selector and arguments must not disappear.
            if !file.bytes[*selector_end..arguments.start]
                .iter()
                .all(u8::is_ascii_whitespace)
            {
                return None;
            }
            edits.push(Correction {
                range: *selector_end..arguments.start,
                replacement: "(".into(),
            });
            edits.push(Correction {
                range: arguments.end..arguments.end,
                replacement: ")".into(),
            });
        }
        let correction = super::correction::combine(file, edits)?;
        Some(correction)
    }

    /// A string or command literal: one token, or a heredoc's opening, body
    /// and terminator.
    fn record_string(
        &mut self,
        location: &ruby_prism::Location<'_>,
        opening: Option<ruby_prism::Location<'_>>,
        content: ruby_prism::Location<'_>,
        closing: Option<ruby_prism::Location<'_>>,
    ) {
        let heredoc = opening
            .as_ref()
            .is_some_and(|opening| opening.as_slice().starts_with(b"<<"));
        if heredoc {
            self.push_optional(opening, Kind::Other);
            self.push_location(&content, Kind::Other);
            self.push_optional(closing, Kind::Other);
        } else {
            self.push_location(location, Kind::Other);
        }
    }

    /// The offsets of the file's semicolon tokens before `data_start`.
    fn semicolons(&self, file: &SourceFile, data_start: usize) -> Vec<usize> {
        memchr::memchr_iter(b';', &file.bytes[..data_start])
            .filter(|&offset| self.covering_token(offset).is_none() && !in_comment(file, offset))
            .collect()
    }

    /// The known token whose range holds `offset` (known tokens do not
    /// overlap).
    fn covering_token(&self, offset: usize) -> Option<&KnownToken> {
        let after = self
            .tokens
            .partition_point(|token| token.range.start <= offset);
        after
            .checked_sub(1)
            .map(|index| &self.tokens[index])
            .filter(|token| token.range.contains(&offset))
    }

    /// The tokens that start on a 1-based line, as RuboCop groups them,
    /// followed by the newline token when the line ends an expression.
    fn line_tokens(&self, file: &SourceFile, line: u32) -> Vec<Token> {
        let bytes = &file.bytes;
        let start = file.line_start(line);
        let end = file.line_start(line + 1);
        let text_end = if end > start && bytes[end - 1] == b'\n' {
            end - 1
        } else {
            end
        };
        let comments = file.comments();
        let mut tokens = Vec::new();
        let mut offset = start;
        // A literal that began on an earlier line continues as content.
        if let Some(token) = self.covering_token(start)
            && token.range.start < start
        {
            tokens.push(Token {
                start,
                kind: Kind::Other,
            });
            offset = token.range.end;
        }
        // Indentation the AST dropped from a squiggly heredoc body line.
        if offset == start
            && matches!(bytes.get(start), Some(b' ' | b'\t'))
            && self.covering_token(start).is_none()
            && self
                .heredoc_bodies
                .iter()
                .any(|body| file.line_start(file.line_of(body.start)) <= start && start < body.end)
        {
            tokens.push(Token {
                start,
                kind: Kind::Other,
            });
        }
        let mut continued = false;
        while offset < text_end {
            let byte = bytes[offset];
            // Before skipping whitespace: a heredoc body line's indentation
            // is string content.
            if let Some(token) = self.covering_token(offset) {
                let kind = match token.kind {
                    Kind::RegexpEnd => {
                        tokens.push(Token {
                            start: offset,
                            kind: Kind::Other,
                        });
                        Kind::Other
                    }
                    kind => kind,
                };
                tokens.push(Token {
                    start: offset,
                    kind,
                });
                offset = token.range.end;
                continue;
            }
            if matches!(byte, b' ' | b'\t' | b'\r' | b'\x0c' | b'\x0b') {
                offset += 1;
                continue;
            }
            if byte == b'\\' && matches!(bytes.get(offset + 1), Some(b'\n' | b'\r')) {
                continued = true;
                break;
            }
            if comments
                .binary_search_by_key(&offset, |comment| comment.start)
                .is_ok()
            {
                tokens.push(Token {
                    start: offset,
                    kind: Kind::Comment,
                });
                break;
            }
            let length = code_token_length(&bytes[offset..text_end]);
            tokens.push(Token {
                start: offset,
                kind: if byte == b';' {
                    Kind::Semi
                } else {
                    Kind::Other
                },
            });
            offset += length;
        }
        // The parser emits a newline token after a trailing comment, and
        // after an expression that ends the line unless a leading-dot call
        // or a `\` continues it.
        let newline = !continued
            && match tokens.last().map(|token| token.kind) {
                Some(Kind::Comment) => true,
                Some(Kind::RightCurly | Kind::Other) => {
                    end > text_end && !continues_with_leading_dot(file, line + 1)
                }
                _ => false,
            };
        if newline {
            tokens.push(Token {
                start: text_end,
                kind: Kind::Newline,
            });
        }
        tokens
    }
}

impl<'pr> NodeVisitor<'pr> for Layout {
    fn enter(&mut self, node: Node<'pr>) {
        self.record(&node);
        if let Some(statements) = node.as_statements_node() {
            let body: Vec<Range<usize>> = statements
                .body()
                .iter()
                .map(|statement| {
                    let location = statement.location();
                    location.start_offset()..location.end_offset()
                })
                .collect();
            if body.len() >= 2 && !self.open.last().copied().unwrap_or(false) {
                self.sequences.push(body);
            }
        }
        let plain_begin = node.as_begin_node().is_some_and(|begin| {
            begin.begin_keyword_loc().is_some()
                && begin.rescue_clause().is_none()
                && begin.ensure_clause().is_none()
        });
        self.open.push(plain_begin);
    }

    fn leave(&mut self) {
        self.open.pop();
    }
}

/// Whether `offset` is inside a comment.
fn in_comment(file: &SourceFile, offset: usize) -> bool {
    let comments = file.comments();
    let after = comments.partition_point(|comment| comment.start <= offset);
    after
        .checked_sub(1)
        .is_some_and(|index| comments[index].contains(&offset))
}

/// The length of the code token at the start of `text` (not a literal,
/// brace or comment): an identifier, keyword or label, an operator, or one
/// punctuation byte.
fn code_token_length(text: &[u8]) -> usize {
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80;
    let first = text[0];
    if is_word(first) || first == b'@' || first == b'$' {
        let sigils = text
            .iter()
            .take_while(|&&byte| byte == b'@')
            .count()
            .max(usize::from(first == b'$'));
        let mut length = sigils + text[sigils..].iter().take_while(|&&b| is_word(b)).count();
        if matches!(text.get(length), Some(b'?' | b'!')) && text.get(length + 1) != Some(&b'=') {
            length += 1;
        }
        // A label (`key:`), but not a scope operator (`Foo::Bar`).
        if text.get(length) == Some(&b':') && text.get(length + 1) != Some(&b':') {
            length += 1;
        }
        return length.max(1);
    }
    const OPERATORS: [&[u8]; 37] = [
        b"**=", b"<=>", b"===", b"...", b"&&=", b"||=", b"<<=", b">>=", b"->", b"&.", b"::", b"..",
        b"==", b"!=", b">=", b"<=", b"&&", b"||", b"<<", b">>", b"**", b"=~", b"!~", b"+=", b"-=",
        b"*=", b"/=", b"%=", b"|=", b"&=", b"^=", b"=>", b"+@", b"-@", b"[]=", b"[]", b"!@",
    ];
    OPERATORS
        .iter()
        .find(|operator| text.starts_with(operator))
        .map_or(1, |operator| operator.len())
}

/// Whether the next code line (skipping blank and comment lines) starts
/// with a leading-dot method call (`.foo`, `&.foo`), which keeps the
/// lexer from ending the expression at the newline.
fn continues_with_leading_dot(file: &SourceFile, from_line: u32) -> bool {
    let mut line = from_line;
    while (line as usize) <= file.line_count() {
        let text = file.line_text(line);
        let code = &text[text
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(text.len())..];
        if code.is_empty() || code.starts_with(b"#") {
            line += 1;
            continue;
        }
        return (code.starts_with(b".") && !code.starts_with(b"..")) || code.starts_with(b"&.");
    }
    false
}

/// Where the `__END__` data section starts (the file's end without one): a
/// line reading `__END__` after the program, outside any literal.
fn data_section_start(file: &SourceFile, program_end: usize, layout: &Layout) -> usize {
    let first_line = file.line_of(program_end);
    for line in first_line..=file.line_count() as u32 {
        let start = file.line_start(line);
        if start < program_end && line == first_line {
            continue;
        }
        if file.line_text(line) == b"__END__" && layout.covering_token(start).is_none() {
            return start;
        }
    }
    file.bytes.len()
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/semicolon_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! `AllowAsExpressionSeparator: false`.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/Semicolon")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn detection_does_not_reparse_semicolon_corrections() {
        CORRECTION_REPARSES.set(0);
        let source = "x = 1;\n".repeat(2000);
        let found = offenses(&source);
        assert_eq!(found.len(), 2000);
        assert_eq!(CORRECTION_REPARSES.get(), 0);
    }

    #[test]
    fn verifies_semicolon_corrections_in_one_batch() {
        CORRECTION_REPARSES.set(0);
        crate::cops::testing::assert_correction(
            &Semicolon,
            &"x = 1;\n".repeat(2000),
            &"x = 1\n".repeat(2000),
        );
        assert_eq!(CORRECTION_REPARSES.get(), 1);
    }

    #[test]
    fn a_failed_batch_retains_only_semantically_verified_corrections() {
        let source = "x = 1;\ny = 2;\n";
        let mut found = crate::cops::testing::correction_offenses(&Semicolon, source);
        // Simulate an unforeseen repair that parses but changes the AST.
        found[0].correction.as_mut().unwrap().replacement = " + 1".into();
        let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        CORRECTION_REPARSES.set(0);
        verify_corrections(&file, &parsed.node(), &mut found);
        assert!(found[0].correction.is_none());
        assert!(found[1].correction.is_some());
        assert_eq!(CORRECTION_REPARSES.get(), 3);
        assert_eq!(
            crate::corrections::apply(&file.bytes, &found)
                .unwrap()
                .bytes,
            b"x = 1;\ny = 2\n"
        );
    }

    #[test]
    fn corrects_semicolons_without_changing_statement_boundaries() {
        // RuboCop expect_correction examples (MIT), default separator policy.
        for (before, after) in [
            ("puts \"this is a test\";\n", "puts \"this is a test\"\n"),
            ("x = \"foo;bar\"; y = 2\n", "x = \"foo;bar\"\n y = 2\n"),
            ("x = 1; y = 2\r\n", "x = 1\r\n y = 2\r\n"),
            ("; puts 1\n", " puts 1\n"),
            ("foo {; bar }\n", "foo { bar }\n"),
            ("foo { bar; }\n", "foo { bar }\n"),
            ("foo { bar; baz }\n", "foo { bar\n baz }\n"),
            ("\"#{foo;}\"\n", "\"#{foo}\"\n"),
            ("42..;\n42...;\n", "(42..)\n(42...)\n"),
            ("x = 42..;\nnext_call\n", "x = (42..)\nnext_call\n"),
            (
                "m key1:, key2:;\ndo_something\n",
                "m(key1:, key2:)\ndo_something\n",
            ),
            (
                "obj&.m 1, key:;\ndo_something\n",
                "obj&.m(1, key:)\ndo_something\n",
            ),
            (
                "m\"str\", key:;\ndo_something\n",
                "m(\"str\", key:)\ndo_something\n",
            ),
            (
                "def foo(key)\n  yield key:;\nend\n",
                "def foo(key)\n  yield(key:)\nend\n",
            ),
            (
                "def foo(key)\n  super key:;\nend\n",
                "def foo(key)\n  super(key:)\nend\n",
            ),
            (
                "x = <<~TEXT;\n  text\nTEXT\n",
                "x = <<~TEXT\n  text\nTEXT\n",
            ),
            (
                "x = 1; y = <<~TEXT\n  text\nTEXT\n",
                "x = 1\n y = <<~TEXT\n  text\nTEXT\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&Semicolon, before, after);
        }
    }

    #[test]
    fn splits_separators_even_when_that_changes_a_directive_scope() {
        crate::cops::testing::assert_correction(
            &Semicolon,
            "fail; foo # rubocop:disable Style/SignalException\n",
            "fail\n foo # rubocop:disable Style/SignalException\n",
        );
    }

    #[test]
    fn leaves_semicolons_that_would_change_magic_line_values() {
        let source = "foo; bar\nline = __LINE__\n";
        let found = crate::cops::testing::correction_offenses(&Semicolon, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn leaves_heredoc_separators_uncorrected() {
        for source in [
            "x = <<~TEXT; y = 2\n  text\nTEXT\n",
            "x = <<~ONE; y = <<~TWO\n  one\nONE\n  two\nTWO\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&Semicolon, source);
            assert!(!found.is_empty());
            assert!(found.iter().all(|offense| offense.correction.is_none()));
        }
    }

    #[test]
    fn flags_terminating_semicolon_with_message() {
        let found = offenses("puts \"this is a test\";\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 22));
        assert_eq!(
            found[0].message,
            "Do not use semicolons to terminate expressions."
        );
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, expected) in [
            (
                "puts \"this is a test\"; puts \"So is this\"\n",
                vec![(1, 22)],
            ),
            ("x = \"foo;bar\"; y = 2\n", vec![(1, 14)]),
            ("x = /a;b/; y = 2\n", vec![(1, 10)]),
            ("x = <<~TEXT; y = 2\n  text\nTEXT\n", vec![(1, 12)]),
            (
                "x = <<~ONE; y = <<~TWO\n  one\nONE\n  two\nTWO\n",
                vec![(1, 11)],
            ),
            ("x = 1; y = <<~TEXT\n  text\nTEXT\n", vec![(1, 6)]),
            ("x = <<~TEXT;\n  text\nTEXT\n", vec![(1, 12)]),
            (
                "def foo(a) x(1); y(2); z(3); end\n",
                vec![(1, 16), (1, 22), (1, 28)],
            ),
            ("foo(\n  bar); baz\n", vec![(2, 7)]),
            ("module Foo; end;\n", vec![(1, 16)]),
            ("; puts 1\n", vec![(1, 1)]),
            ("foo {; bar }\n", vec![(1, 6)]),
            ("foo -> {; bar }\n", vec![(1, 9)]),
            ("foo { bar; }\n", vec![(1, 10)]),
            ("foo { bar; baz }\n", vec![(1, 10)]),
            ("\"#{foo;}\"\n", vec![(1, 7)]),
            ("\"#{;foo}\"\n", vec![(1, 4)]),
            ("1..42;\n", vec![(1, 6)]),
            ("1...42;\n", vec![(1, 7)]),
            ("42..;\n", vec![(1, 5)]),
            ("42..;\n42...;\n", vec![(1, 5), (2, 6)]),
            ("def foo\n  42..;\nend\n", vec![(2, 7)]),
            ("m key: value;\ndo_something\n", vec![(1, 13)]),
            ("def c a:;\n  d\nend\n", vec![(1, 9)]),
            ("case h\nin a:;\n  d\nend\n", vec![(2, 6)]),
            ("m key1:, key2:;\ndo_something\n", vec![(1, 15)]),
            ("m(key:);\ndo_something\n", vec![(1, 8)]),
            ("obj&.m key:;\ndo_something\n", vec![(1, 12)]),
            ("m 1, key:;\ndo_something\n", vec![(1, 10)]),
            ("obj.m 1, 2, key1:, key2:;\ndo_something\n", vec![(1, 25)]),
            ("obj&.m 1, key:;\ndo_something\n", vec![(1, 15)]),
            ("m\"str\", key:;\ndo_something\n", vec![(1, 13)]),
            ("def foo(key)\n  yield key:;\nend\n", vec![(2, 13)]),
            ("def foo(key)\n  super key:;\nend\n", vec![(2, 13)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "def foo(a) z(3); end\n",
            "def foo(a); z(3) end\n",
            "def foo1; x(3) end\ndef initialize(*_); end\ndef foo2() x(3); end\ndef foo3; x(3); end\n",
            "# Prefer a single-line format for class ...\nclass Foo < Exception; end\n\nclass Bar; end\n",
            "# One exception to the rule are empty-body methods\ndef no_op; end\n\ndef foo; end\n",
            "module Foo; end\n",
            "string = \";\nmulti-line string\"\n",
            "# ;\nfoo {\n}\n",
            "42..\n",
            "def foo\n  bar = baz if qux rescue quux\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn follows_rubocop_token_rules() {
        for (source, expected) in [
            // Only the first matching terminator check counts per line.
            ("; puts 1;\n", vec![(1, 9)]),
            ("x = 1;y = 2\n", vec![(1, 6)]),
            // Block-local separators are semicolon tokens too.
            ("foo { |a; b| a; b }\n", vec![(1, 9), (1, 15)]),
            ("pid = $$; x\n", vec![(1, 9)]),
            ("a; b; c\n", vec![(1, 2), (1, 5)]),
            ("x = (a; b)\n", vec![(1, 7)]),
            ("\"#{a; b}\"\n", vec![(1, 5)]),
            ("foo;\r\nbar\r\n", vec![(1, 4)]),
            ("foo; \\\nbar\n", vec![(1, 4)]),
            ("-> {; bar }\n", vec![(1, 5)]),
            (
                "begin; a; b; rescue; end\n",
                vec![(1, 6), (1, 9), (1, 12), (1, 20)],
            ),
            ("x = :\"#{a;}\"\n", vec![(1, 10)]),
            ("x = `#{a;}`\n", vec![(1, 9)]),
            ("x = \"a\nb\"; y\n", vec![(2, 3)]),
            ("foo { bar; }\n# note\n\nbaz\n", vec![(1, 10)]),
            ("x = 1i;\n", vec![(1, 7)]),
            ("; 1i\n", vec![(1, 1)]),
            // `;` as a percent-array delimiter is not a semicolon token.
            ("foo %w;a; ; bar\n", vec![(1, 11)]),
            // Leading whitespace in a heredoc body is a content token.
            ("x = <<-A\n  #{;foo}\nA\n", vec![(2, 5)]),
            ("x = <<~A\n  #{;foo}\nA\n", vec![(2, 5)]),
        ] {
            assert_eq!(positions(source), expected, "{source:?}");
        }
    }

    #[test]
    fn accepts_semicolons_rubocop_accepts() {
        for source in [
            "foo; # a comment\n",
            "begin; a; b; end\n",
            "x.foo {; bar }\n",
            "foo { bar; } # c\n",
            "foo { bar; }\n  .baz\n",
            "foo { bar; }\n# c\n  &.baz\n",
            "foo { bar; }",
            "y = $;\n",
            "x = ?;\n",
            "x = :\";\"\n",
            "%w[a ; b]\n",
            "x = <<~TEXT\n  a;\n  ;b\nTEXT\n",
            "x = <<~TEXT\n  #{a}; b;\nTEXT\n",
            "if x; y; end\n",
            "while x; y; end\n",
            "class Foo; def bar; end; end\n",
            "foo\n__END__\na; b;\n",
            "=begin\na;\n=end\n",
            "\"#{a;}x\"\n",
            "foo(\"#{a;}\")\n",
            "x = 1 # a; b;\n",
            // A regexp ends with two tokens: `/` and its (empty) options.
            "x = /#{a;}/\n",
            "x = /#{a;}/i\n",
            "x = %w;a b;\n",
            "x = %i;a b;\n",
            // No newline token after a `\` line continuation.
            "x = foo { bar; } \\\n  + 1\n",
            "x = \"#{a;}\" \\\n  \"b\"\n",
            // A program without statements is not checked.
            ";;\n",
            "# c\n;\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("a; b # rubocop:disable Style/Semicolon\n").is_empty());
        assert!(offenses("# rubocop:disable Style/Semicolon\nfoo;\n").is_empty());
        assert_eq!(positions("a; b\n"), [(1, 2)]);
    }
}
