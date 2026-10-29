//! Source bytes that are literal text, not interpolation code. Built lazily
//! from the existing Prism tree for correction safety and separator detection.

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use ruby_prism::{Location, Node, NodeList};

use crate::cops::walk::{NodeVisitor, walk};

use super::SourceFile;

pub(super) fn collect(file: &SourceFile, root: &Node<'_>) -> Vec<Range<usize>> {
    collect_inner(file, root, false).protected
}

/// Literal/token exclusions for separator cops. Unlike correction-only literal
/// protection, percent-array interpolation is code and must be inspected.
pub(super) fn punctuation_ranges(file: &SourceFile, root: &Node<'_>) -> PunctuationRanges {
    collect_inner(file, root, true)
}

pub(super) struct PunctuationRanges {
    pub protected: Vec<Range<usize>>,
    pub interpolation_ends: Vec<usize>,
    /// Openings translated as tLPAREN_ARG, not an ordinary left-paren token.
    pub argument_parens: Vec<usize>,
    /// Ends of simplified tSTRING tokens that started on an earlier line.
    pub multiline_string_ends: Vec<usize>,
    /// tSPACE starts after percent-array words, including at an empty line.
    pub percent_array_separators: Vec<usize>,
    /// Physical definition ends, including trailing heredocs omitted by Prism.
    pub definition_ends: HashMap<usize, usize>,
    /// Exact subtree locations with a heredoc descendant (or root). Locations
    /// shared by wrapper nodes describe the same subtree, not byte containment.
    pub heredoc_subtrees: HashSet<(usize, usize)>,
    /// Removing a surrounding hash delimiter must not rebind a do/end block.
    /// Exact subtree ownership, collected in this same shared token walk.
    pub keyword_block_subtrees: HashSet<(usize, usize)>,
    /// Sorted real opener offsets, including heredocs owned by sibling nodes.
    pub heredoc_openings: Vec<usize>,
    pub heredocs: Vec<super::heredoc_boundaries::Heredoc>,
    pub heredoc_boundaries:
        std::sync::OnceLock<std::sync::Arc<super::heredoc_boundaries::Boundaries>>,
}

fn collect_inner(file: &SourceFile, root: &Node<'_>, punctuation: bool) -> PunctuationRanges {
    let mut collector = Collector {
        file,
        ranges: Vec::new(),
        punctuation,
        interpolation_ends: Vec::new(),
        argument_parens: Vec::new(),
        multiline_string_ends: Vec::new(),
        percent_array_separators: Vec::new(),
        pending: Vec::new(),
        extents: Vec::new(),
        embedded_ends: HashMap::new(),
        definition_ends: HashMap::new(),
        heredoc_subtrees: HashSet::new(),
        keyword_block_subtrees: HashSet::new(),
        heredoc_openings: Vec::new(),
        heredocs: Vec::new(),
    };
    walk(&mut collector, root);
    for literal in std::mem::take(&mut collector.pending) {
        collector.protect_interpolated(literal.opening, literal.closing, literal.parts);
    }
    collector.ranges.sort_unstable_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in collector.ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else if !range.is_empty() {
            merged.push(range);
        }
    }
    collector.interpolation_ends.sort_unstable();
    collector.argument_parens.sort_unstable();
    collector.multiline_string_ends.sort_unstable();
    collector.percent_array_separators.sort_unstable();
    collector.heredoc_openings.sort_unstable();
    PunctuationRanges {
        protected: merged,
        interpolation_ends: collector.interpolation_ends,
        argument_parens: collector.argument_parens,
        multiline_string_ends: collector.multiline_string_ends,
        percent_array_separators: collector.percent_array_separators,
        definition_ends: collector.definition_ends,
        heredoc_subtrees: collector.heredoc_subtrees,
        keyword_block_subtrees: collector.keyword_block_subtrees,
        heredoc_openings: collector.heredoc_openings,
        heredocs: collector.heredocs,
        heredoc_boundaries: std::sync::OnceLock::new(),
    }
}

struct PendingLiteral<'pr> {
    opening: Option<Location<'pr>>,
    closing: Option<Location<'pr>>,
    parts: NodeList<'pr>,
}

struct Collector<'a, 'pr> {
    file: &'a SourceFile,
    ranges: Vec<Range<usize>>,
    punctuation: bool,
    interpolation_ends: Vec<usize>,
    argument_parens: Vec<usize>,
    multiline_string_ends: Vec<usize>,
    percent_array_separators: Vec<usize>,
    pending: Vec<PendingLiteral<'pr>>,
    // Physical subtree ends include heredoc bodies/terminators, which Prism's
    // ordinary node locations omit. Compute them once in the same AST walk.
    extents: Vec<SubtreeExtent>,
    embedded_ends: HashMap<usize, usize>,
    definition_ends: HashMap<usize, usize>,
    heredoc_subtrees: HashSet<(usize, usize)>,
    keyword_block_subtrees: HashSet<(usize, usize)>,
    heredoc_openings: Vec<usize>,
    heredocs: Vec<super::heredoc_boundaries::Heredoc>,
}

struct SubtreeExtent {
    location: (usize, usize),
    end: usize,
    embedded_start: Option<usize>,
    definition_start: Option<usize>,
    has_heredoc: bool,
    has_keyword_block: bool,
}

impl<'pr> Collector<'_, 'pr> {
    fn argument_paren(&mut self, prefix: Location<'_>, value: Option<Node<'_>>) {
        let Some(paren) = value.and_then(|n| n.as_parentheses_node()) else {
            return;
        };
        let start = paren.opening_loc().start_offset();
        // Unlike f(x), f (x) is parsed as a parenthesized command argument.
        // Keep only actual whitespace/continuation gaps, not intervening code.
        if start > prefix.end_offset()
            && self.file.bytes[prefix.end_offset()..start]
                .iter()
                .all(|b| b.is_ascii_whitespace() || *b == b'\\')
        {
            self.argument_parens.push(start);
        }
    }

    fn multiline_string(&mut self, string: &ruby_prism::StringNode<'_>) {
        let (Some(opening), Some(closing)) = (string.opening_loc(), string.closing_loc()) else {
            return;
        };
        let content = string.content_loc();
        let bytes = content.as_slice();
        // The pinned Prism-to-parser translator simplifies a static double-
        // quoted string to one tSTRING only when every newline is escaped.
        // Other literals retain a separate closing token on the last line.
        if opening.as_slice() == b"\""
            && bytes.contains(&b'\n')
            && memchr::memchr_iter(b'\n', bytes).all(|newline| {
                let end = newline - usize::from(newline > 0 && bytes[newline - 1] == b'\r');
                bytes[..end]
                    .iter()
                    .rev()
                    .take_while(|&&b| b == b'\\')
                    .count()
                    % 2
                    == 1
            })
        {
            self.multiline_string_ends.push(closing.end_offset());
        }
    }

    fn protect(&mut self, loc: &Location<'_>) {
        if let Some(extent) = self.extents.last_mut() {
            extent.end = extent.end.max(loc.end_offset());
        }
        self.ranges.push(loc.start_offset()..loc.end_offset());
    }

    fn record_heredoc(
        &mut self,
        opening: Option<&Location<'_>>,
        closing: Option<&Location<'_>>,
        body_start: Option<usize>,
    ) {
        if self.punctuation
            && let Some(opening) = opening
            && opening.as_slice().starts_with(b"<<")
        {
            self.extents.last_mut().unwrap().has_heredoc = true;
            self.heredoc_openings.push(opening.start_offset());
            if let Some(closing) = closing {
                let raw = opening.as_slice();
                let indented = matches!(raw.get(2), Some(b'-' | b'~'));
                let mut label =
                    opening.start_offset() + 2 + usize::from(indented)..opening.end_offset();
                if matches!(self.file.bytes.get(label.start), Some(b'\'' | b'"' | b'`')) {
                    label.start += 1;
                    label.end = label.end.saturating_sub(1).max(label.start);
                }
                let end_line = self.file.line_of(closing.start_offset());
                let closing_start = self.file.line_start(end_line);
                let body_start = body_start.map_or(closing_start, |offset| {
                    self.file.line_start(self.file.line_of(offset))
                });
                self.heredocs.push(super::heredoc_boundaries::Heredoc {
                    opening: opening.start_offset()..opening.end_offset(),
                    body: body_start..closing_start,
                    closing: closing_start..closing.end_offset(),
                    label,
                    indented,
                });
            }
        }
    }

    fn literal(
        &mut self,
        opening: Option<Location<'_>>,
        content: Option<Location<'_>>,
        closing: Option<Location<'_>>,
    ) {
        self.record_heredoc(
            opening.as_ref(),
            closing.as_ref(),
            content.as_ref().map(Location::start_offset),
        );
        // Whitespace can itself delimit a percent literal. Dropping either
        // delimiter may still parse but produce a different runtime value.
        for loc in [opening, content, closing].into_iter().flatten() {
            self.protect(&loc);
        }
    }

    fn interpolated(
        &mut self,
        opening: Option<Location<'pr>>,
        closing: Option<Location<'pr>>,
        parts: NodeList<'pr>,
    ) {
        self.record_heredoc(
            opening.as_ref(),
            closing.as_ref(),
            parts
                .iter()
                .next()
                .map(|part| part.location().start_offset()),
        );
        for loc in [opening.as_ref(), closing.as_ref()].into_iter().flatten() {
            self.protect(loc);
        }
        self.pending.push(PendingLiteral {
            opening,
            closing,
            parts,
        });
    }

    fn protect_interpolated(
        &mut self,
        opening: Option<Location<'pr>>,
        closing: Option<Location<'pr>>,
        parts: NodeList<'pr>,
    ) {
        let (Some(opening), Some(closing)) = (opening, closing) else {
            // Implicit string concatenation: the children carry delimiters.
            return;
        };
        let mut cursor = if opening.as_slice().starts_with(b"<<") {
            // Heredocs may share an opener line. The first part, not the
            // opener's next line, locates this particular heredoc's body.
            parts.iter().next().map_or(closing.start_offset(), |part| {
                self.file
                    .line_start(self.file.line_of(part.location().start_offset()))
            })
        } else {
            opening.end_offset()
        };
        for part in parts.iter() {
            let loc = part.location();
            // Protect gaps (including dedented indentation), and literal
            // parts. Interpolation nodes themselves contain ordinary code;
            // nested literal children are independently collected by the walk.
            let end = if matches!(
                part,
                Node::EmbeddedStatementsNode { .. } | Node::EmbeddedVariableNode { .. }
            ) {
                loc.start_offset()
            } else {
                loc.end_offset()
            };
            if cursor < end {
                self.ranges.push(cursor..end);
            }
            // An embedded heredoc's body is not an outer literal gap. Its
            // own children protect literal text and expose interpolation code.
            // Later parts can still be on its opener line: never move back.
            cursor = cursor.max(
                self.embedded_ends
                    .get(&loc.start_offset())
                    .copied()
                    .unwrap_or(loc.end_offset()),
            );
        }
        if cursor < closing.start_offset() {
            self.ranges.push(cursor..closing.start_offset());
        }
    }
}

impl<'pr> NodeVisitor<'pr> for Collector<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let location = node.location();
        self.extents.push(SubtreeExtent {
            location: (location.start_offset(), location.end_offset()),
            end: location.end_offset(),
            embedded_start: node
                .as_embedded_statements_node()
                .map(|_| location.start_offset()),
            definition_start: (self.punctuation
                && matches!(
                    node,
                    Node::DefNode { .. } | Node::ClassNode { .. } | Node::ModuleNode { .. }
                ))
            .then(|| location.start_offset()),
            has_heredoc: false,
            has_keyword_block: node
                .as_block_node()
                .is_some_and(|b| b.opening_loc().as_slice() == b"do"),
        });
        match node {
            Node::StringNode { .. } => {
                let n = node.as_string_node().unwrap();
                if self.punctuation {
                    self.multiline_string(&n);
                }
                self.literal(n.opening_loc(), Some(n.content_loc()), n.closing_loc());
            }
            Node::SymbolNode { .. } => {
                let n = node.as_symbol_node().unwrap();
                self.literal(n.opening_loc(), n.value_loc(), n.closing_loc());
            }
            Node::RegularExpressionNode { .. } => {
                let n = node.as_regular_expression_node().unwrap();
                self.literal(
                    Some(n.opening_loc()),
                    Some(n.content_loc()),
                    Some(n.closing_loc()),
                );
            }
            Node::MatchLastLineNode { .. } => {
                let n = node.as_match_last_line_node().unwrap();
                self.literal(
                    Some(n.opening_loc()),
                    Some(n.content_loc()),
                    Some(n.closing_loc()),
                );
            }
            Node::XStringNode { .. } => {
                let n = node.as_x_string_node().unwrap();
                self.literal(
                    Some(n.opening_loc()),
                    Some(n.content_loc()),
                    Some(n.closing_loc()),
                );
            }
            Node::InterpolatedStringNode { .. } => {
                let n = node.as_interpolated_string_node().unwrap();
                self.interpolated(n.opening_loc(), n.closing_loc(), n.parts());
            }
            Node::InterpolatedSymbolNode { .. } => {
                let n = node.as_interpolated_symbol_node().unwrap();
                self.interpolated(n.opening_loc(), n.closing_loc(), n.parts());
            }
            Node::InterpolatedRegularExpressionNode { .. } => {
                let n = node.as_interpolated_regular_expression_node().unwrap();
                self.interpolated(Some(n.opening_loc()), Some(n.closing_loc()), n.parts());
            }
            Node::InterpolatedMatchLastLineNode { .. } => {
                let n = node.as_interpolated_match_last_line_node().unwrap();
                self.interpolated(Some(n.opening_loc()), Some(n.closing_loc()), n.parts());
            }
            Node::InterpolatedXStringNode { .. } => {
                let n = node.as_interpolated_x_string_node().unwrap();
                self.interpolated(Some(n.opening_loc()), Some(n.closing_loc()), n.parts());
            }
            Node::ArrayNode { .. } => {
                let n = node.as_array_node().unwrap();
                if n.opening_loc()
                    .is_some_and(|loc| loc.as_slice().starts_with(b"%"))
                {
                    // For punctuation detection, child strings protect word
                    // content while embedded statements remain inspectable.
                    // The correction-only whitespace guard still conservatively
                    // protects entire percent arrays, including interpolation.
                    if self.punctuation {
                        self.literal(n.opening_loc(), None, n.closing_loc());
                        self.percent_array_separators.extend(
                            n.elements()
                                .iter()
                                .map(|element| element.location().end_offset()),
                        );
                    } else {
                        self.protect(&n.location());
                    }
                }
            }
            _ => {}
        }
        if self.punctuation {
            match node {
                Node::CallNode { .. } => {
                    let n = node.as_call_node().unwrap();
                    if n.opening_loc().is_none()
                        && let Some(message) = n.message_loc()
                    {
                        let value = if message.as_slice() == b"not" {
                            n.receiver()
                        } else {
                            n.arguments().and_then(|a| a.arguments().iter().next())
                        };
                        self.argument_paren(message, value);
                    }
                }
                Node::YieldNode { .. } => {
                    let n = node.as_yield_node().unwrap();
                    if n.lparen_loc().is_none() {
                        self.argument_paren(
                            n.keyword_loc(),
                            n.arguments().and_then(|a| a.arguments().iter().next()),
                        );
                    }
                }
                Node::SuperNode { .. } => {
                    let n = node.as_super_node().unwrap();
                    if n.lparen_loc().is_none() {
                        self.argument_paren(
                            n.keyword_loc(),
                            n.arguments().and_then(|a| a.arguments().iter().next()),
                        );
                    }
                }
                Node::DefinedNode { .. } => {
                    let n = node.as_defined_node().unwrap();
                    if n.lparen_loc().is_none() {
                        self.argument_paren(n.keyword_loc(), Some(n.value()));
                    }
                }
                _ => {}
            }
            // Method selectors (`obj.[]`, `def []=`) are names, not square
            // delimiters. Global-variable punctuation likewise belongs to its
            // token, not a statement/argument separator (e.g. `$;` and `$,`).
            let name = match node {
                Node::CallNode { .. } => {
                    let n = node.as_call_node().unwrap();
                    n.message_loc().filter(|loc| {
                        n.call_operator_loc().is_some() && matches!(loc.as_slice(), b"[]" | b"[]=")
                    })
                }
                Node::DefNode { .. } => Some(node.as_def_node().unwrap().name_loc())
                    .filter(|loc| matches!(loc.as_slice(), b"[]" | b"[]=")),
                Node::GlobalVariableReadNode { .. }
                | Node::GlobalVariableTargetNode { .. }
                | Node::BackReferenceReadNode { .. } => Some(node.location()),
                Node::GlobalVariableWriteNode { .. } => {
                    Some(node.as_global_variable_write_node().unwrap().name_loc())
                }
                Node::GlobalVariableAndWriteNode { .. } => {
                    Some(node.as_global_variable_and_write_node().unwrap().name_loc())
                }
                Node::GlobalVariableOrWriteNode { .. } => {
                    Some(node.as_global_variable_or_write_node().unwrap().name_loc())
                }
                Node::GlobalVariableOperatorWriteNode { .. } => Some(
                    node.as_global_variable_operator_write_node()
                        .unwrap()
                        .name_loc(),
                ),
                _ => None,
            };
            if let Some(name) = name {
                self.protect(&name);
            }
            if let Some(embedded) = node.as_embedded_statements_node() {
                self.interpolation_ends
                    .push(embedded.closing_loc().start_offset());
            }
        }
    }

    fn leave(&mut self) {
        let extent = self.extents.pop().unwrap();
        if let Some(start) = extent.definition_start {
            self.definition_ends.insert(start, extent.end);
        }
        if let Some(start) = extent.embedded_start {
            self.embedded_ends.insert(start, extent.end);
        }
        if extent.has_heredoc {
            self.heredoc_subtrees.insert(extent.location);
        }
        if extent.has_keyword_block {
            self.keyword_block_subtrees.insert(extent.location);
        }
        if let Some(parent) = self.extents.last_mut() {
            parent.end = parent.end.max(extent.end);
            parent.has_heredoc |= extent.has_heredoc;
            parent.has_keyword_block |= extent.has_keyword_block;
        }
    }
}
