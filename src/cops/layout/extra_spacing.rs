//! `Layout/ExtraSpacing` — more than one space between two tokens on a line.
//!
//! A port of RuboCop's cop (97f30c4, MIT) at the fixed defaults
//! `AllowForAlignment: true`, `AllowBeforeTrailingComments: false` and
//! `ForceEqualSignAlignment: false`. It walks consecutive pairs of RuboCop's
//! own token stream (Prism's lexer tokens in the pinned translator's
//! projection, `SourceFile::parser_tokens`): a gap of two or more characters
//! between tokens on the same line is reported, from the end of the first
//! token to one character before the second, unless the second token is
//!
//! - a comment aligned with an adjacent comment (`aligned_comments`), or
//! - aligned with something on the nearest relevant line
//!   (`PrecedingFollowingAlignment`) and either an assignment/comparison
//!   operator or a token whose spacing varies within its alignment group, or
//! - inside a multi-line hash (or hash pattern) between a key and its value,
//!   which `Layout/HashAlignment` owns.
//!
//! The correction removes the reported range, leaving one blank. It is a
//! canonical edit of blank bytes between two tokens on one line: no token,
//! literal, comment, line or encoding byte changes, and Ruby's lexer only
//! distinguishes the presence of a blank, which stays. It is withheld when
//! the range holds anything but spaces, tabs, vertical tabs or form feeds,
//! and for the listed source observers.
//!
//! Upstream's `autocorrect_incompatible_with` (`Layout/RedundantLineBreak`,
//! `Style/SelfAssignment`) only orders RuboCop's iterative correction rounds.
//! It is not declared here: rustocop withholds a cop's edits whenever a
//! declared companion is selected, and the default-on SelfAssignment would
//! disable every ExtraSpacing edit, while the overlap selection of
//! `corrections::apply` already keeps one edit of any overlapping pair (as
//! `Layout/FirstArgumentIndentation` decided for its upstream entry).
use std::collections::HashMap;

use ruby_prism::Node;

use super::preceding_following::Lines;
use crate::{
    cops::{
        Cop,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::{
        SourceFile,
        tokens::{Token, assignment_or_comparison},
    },
};

pub struct ExtraSpacing;

const MESSAGE: &str = "Unnecessary spacing detected.";

impl Cop for ExtraSpacing {
    fn name(&self) -> &'static str {
        "Layout/ExtraSpacing"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let root = result.node();
        // `processed_source.blank?`: the parser gem has no AST for a file of
        // only comments and blanks.
        if root
            .as_program_node()
            .is_none_or(|program| program.statements().body().is_empty())
        {
            return;
        }
        let tokens = file.parser_tokens();
        let mut investigation = Investigation::new(file, &root, tokens);
        for pair in tokens.windows(2) {
            let (first, second) = (&pair[0], &pair[1]);
            if second.is_newline() || first.line != second.line {
                continue;
            }
            let (start, end) = (first.end, second.start.saturating_sub(1));
            if end <= start || investigation.aligned(first, second) {
                continue;
            }
            if investigation.ignored(start) {
                continue;
            }
            let (line, col) = file.line_col(start);
            let blank = file.bytes[start..end]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c'));
            let correction = blank.then(|| Correction {
                range: start..end,
                replacement: String::new(),
            });
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: MESSAGE.into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

/// Per-file state of one investigation: RuboCop's memoized instance
/// variables, with the same first-come cache identities.
struct Investigation<'a> {
    file: &'a SourceFile,
    tokens: &'a [Token],
    lines: &'a Lines,
    /// `@aligned_comments`: lines of consecutive comments in one column.
    aligned_comments: Vec<bool>,
    /// `definition_boundary_lines`, sorted, plus a per-line flag.
    boundaries: Vec<u32>,
    boundary: Vec<bool>,
    /// Key-to-value gaps of pairs in multi-line hashes, sorted.
    ignored: Vec<std::ops::Range<usize>>,
    /// `@tokens_by_line`: token indices per line, in stream order.
    by_line: Vec<Vec<usize>>,
    groups: Vec<Vec<u32>>,
    group_of_line: Vec<Option<usize>>,
    profiles: HashMap<usize, Profile>,
    verdicts: HashMap<(usize, u32, &'static str), bool>,
    /// Per column, the lines whose first token at that column follows
    /// another token, with that spacing (for `spacing_before_token_at`).
    spacing_index: Option<HashMap<u32, Vec<(u32, i64)>>>,
}

/// Lines and spacings of the tokens of one column and type.
type Typed = (Vec<u32>, Vec<i64>);

#[derive(Default)]
struct Profile {
    by_column: HashMap<u32, Vec<(u32, i64)>>,
    typed: HashMap<(u32, &'static str), Typed>,
}

/// Definition boundaries and multi-line pair gaps from the main AST.
#[derive(Default)]
struct AstFacts {
    boundaries: Vec<u32>,
    ignored: Vec<std::ops::Range<usize>>,
}

struct AstCollector<'a> {
    file: &'a SourceFile,
    facts: AstFacts,
}

impl AstCollector<'_> {
    fn pairs<'pr>(&mut self, parent: &Node<'pr>, elements: impl Iterator<Item = Node<'pr>>) {
        let location = parent.location();
        if self.file.line_of(location.start_offset()) == self.file.last_line_of(location) {
            return;
        }
        for element in elements {
            if let Some(pair) = element.as_assoc_node() {
                let (key, value) = (pair.key().location(), pair.value().location());
                if key.end_offset() < value.start_offset() {
                    self.facts
                        .ignored
                        .push(key.end_offset()..value.start_offset());
                }
            }
        }
    }
}

impl<'pr> NodeVisitor<'pr> for AstCollector<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        match &node {
            Node::DefNode { .. } => {
                let location = node.location();
                self.facts
                    .boundaries
                    .push(self.file.line_of(location.start_offset()));
                self.facts.boundaries.push(self.file.last_line_of(location));
            }
            Node::HashNode { .. } => {
                let elements = node.as_hash_node().unwrap().elements();
                self.pairs(&node, elements.iter());
            }
            Node::KeywordHashNode { .. } => {
                let elements = node.as_keyword_hash_node().unwrap().elements();
                self.pairs(&node, elements.iter());
            }
            Node::HashPatternNode { .. } => {
                let elements = node.as_hash_pattern_node().unwrap().elements();
                self.pairs(&node, elements.iter());
            }
            _ => {}
        }
    }
}

impl<'a> Investigation<'a> {
    fn new(file: &'a SourceFile, root: &Node<'_>, tokens: &'a [Token]) -> Self {
        let lines = file.preceding_following();
        let count = lines.count();
        let mut aligned_comments = vec![false; count + 1];
        let positions = file.line_cols(file.comments().iter().map(|c| c.start));
        for pair in positions.windows(2) {
            if pair[0].1 == pair[1].1 {
                for (line, _) in pair {
                    if let Some(flag) = aligned_comments.get_mut(*line as usize) {
                        *flag = true;
                    }
                }
            }
        }
        let mut collector = AstCollector {
            file,
            facts: AstFacts::default(),
        };
        walk(&mut collector, root);
        let AstFacts {
            mut boundaries,
            mut ignored,
        } = collector.facts;
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut boundary = vec![false; count + 2];
        for &line in &boundaries {
            if let Some(flag) = boundary.get_mut(line as usize) {
                *flag = true;
            }
        }
        ignored.sort_unstable_by_key(|r| (r.start, r.end));
        let mut by_line = vec![Vec::new(); count + 1];
        for (index, token) in tokens.iter().enumerate() {
            if let Some(line) = by_line.get_mut(token.line as usize) {
                line.push(index);
            }
        }
        Self {
            file,
            tokens,
            lines,
            aligned_comments,
            boundaries,
            boundary,
            ignored,
            by_line,
            groups: Vec::new(),
            group_of_line: vec![None; count + 1],
            profiles: HashMap::new(),
            verdicts: HashMap::new(),
            spacing_index: None,
        }
    }

    /// `ignored_range?`: a gap starting inside a multi-line pair's gap.
    fn ignored(&self, start: usize) -> bool {
        let index = self.ignored.partition_point(|r| r.start <= start);
        self.ignored[..index]
            .iter()
            .rev()
            .any(|r| r.contains(&start))
    }

    fn is_boundary(&self, line: u32) -> bool {
        self.boundary.get(line as usize).copied().unwrap_or(false)
    }

    /// `aligned_tok?`.
    fn aligned(&mut self, previous: &Token, token: &Token) -> bool {
        if token.is_comment() {
            return self
                .aligned_comments
                .get(token.line as usize)
                .copied()
                .unwrap_or(false);
        }
        self.lines.aligned_with_something(
            self.file,
            token.start..token.end,
            token.line,
            token.column as usize,
            token.end_column as usize,
        ) && (assignment_or_comparison(token.kind) || self.spacing_varies(previous, token))
    }

    fn spacing(&self, previous: &Token, token: &Token) -> i64 {
        if previous.end_line == token.line {
            i64::from(token.column) - i64::from(previous.end_column)
        } else {
            token.start as i64 - previous.end as i64
        }
    }

    /// `spacing_varies?`.
    fn spacing_varies(&mut self, previous: &Token, token: &Token) -> bool {
        let group = self.group_profile(token.line);
        let profile = &self.profiles[&group];
        let column = token.column;
        let empty = (Vec::new(), Vec::new());
        let (lines, spacings) = profile.typed.get(&(column, token.kind)).unwrap_or(&empty);
        let entries = profile
            .by_column
            .get(&column)
            .map_or(&[][..], Vec::as_slice);
        if lines.len() > 1 {
            let key = (group, column, token.kind);
            if let Some(&verdict) = self.verdicts.get(&key) {
                return verdict;
            }
            let verdict =
                varies(spacings.iter().copied()) || self.other_column_varies(lines, column);
            self.verdicts.insert(key, verdict);
            verdict
        } else if entries.len() > 1 {
            varies(entries.iter().map(|&(_, spacing)| spacing))
        } else {
            self.nearest_spacing_varies(previous, token)
        }
    }

    /// Whether, on `lines` (sorted, as profiles are built from a group's
    /// sorted lines), the spacing before the tokens of some column other
    /// than `column` varies: the group profile's `by_column` restricted to
    /// `lines`, read from the lines instead of from every column's entries.
    fn other_column_varies(&self, lines: &[u32], column: u32) -> bool {
        let mut spaced = Vec::new();
        let mut previous = None;
        for &line in lines {
            if previous.replace(line) == Some(line) {
                continue;
            }
            let Some(indices) = self.by_line.get(line as usize) else {
                continue;
            };
            for pair in indices.windows(2) {
                let token = &self.tokens[pair[1]];
                if token.column != column {
                    spaced.push((token.column, self.spacing(&self.tokens[pair[0]], token)));
                }
            }
        }
        spaced.sort_unstable();
        spaced
            .chunk_by(|a, b| a.0 == b.0)
            .any(|run| run[0].1 != run[run.len() - 1].1)
    }

    /// `group_profile`: the profile of the alignment group of `line`.
    fn group_profile(&mut self, line: u32) -> usize {
        let group = self.alignment_lines(line);
        if !self.profiles.contains_key(&group) {
            let mut profile = Profile::default();
            for &line in &self.groups[group] {
                let Some(indices) = self.by_line.get(line as usize) else {
                    continue;
                };
                for pair in indices.windows(2) {
                    let (previous, token) = (&self.tokens[pair[0]], &self.tokens[pair[1]]);
                    let spacing = self.spacing(previous, token);
                    profile
                        .by_column
                        .entry(token.column)
                        .or_default()
                        .push((line, spacing));
                    let typed = profile.typed.entry((token.column, token.kind)).or_default();
                    typed.0.push(line);
                    typed.1.push(spacing);
                }
            }
            self.profiles.insert(group, profile);
        }
        group
    }

    /// `alignment_lines`: memoized per line; a later group overwrites the
    /// lines it contains, as upstream.
    fn alignment_lines(&mut self, line: u32) -> usize {
        if let Some(group) = self.group_of_line[line as usize] {
            return group;
        }
        let mut lines = self.relevant_lines(line, false);
        lines.extend(self.relevant_lines(line, true));
        lines.sort_unstable();
        lines.dedup();
        let group = self.groups.len();
        for &member in &lines {
            self.group_of_line[member as usize] = Some(group);
        }
        self.groups.push(lines);
        group
    }

    /// `relevant_lines` with the definition boundaries, keeping lines that
    /// are not aligned comment lines.
    fn relevant_lines(&self, original: u32, upward_in_file: bool) -> Vec<u32> {
        let count = self.lines.count() as u32;
        let indent = self.lines.indentation(original);
        let mut at_level = true;
        let mut result = Vec::new();
        let mut line = original;
        loop {
            let current = self.lines.indentation(line);
            let blank = self.lines.blank(line);
            if (line != original && self.is_boundary(line))
                || (current < indent && !blank)
                || (at_level && blank)
            {
                break;
            }
            if !self.lines.comment_line(line) && current == indent {
                result.push(line);
            }
            if !blank {
                at_level = current == indent;
            }
            if upward_in_file {
                if line >= count {
                    break;
                }
                line += 1;
            } else {
                if line <= 1 {
                    break;
                }
                line -= 1;
            }
        }
        result
    }

    /// `nearest_spacing_varies?`.
    fn nearest_spacing_varies(&mut self, previous: &Token, token: &Token) -> bool {
        let spacing = self.spacing(previous, token);
        self.build_spacing_index();
        let index = self.spacing_index.as_ref().unwrap();
        let Some(entries) = index.get(&token.column) else {
            return false;
        };
        let line = token.line;
        let split = entries.partition_point(|&(other, _)| other < line);
        let below = self.boundaries.partition_point(|&b| b < line);
        // Upward in the file: the nearest earlier line, unless a boundary
        // lies between (or on it).
        let earlier = split.checked_sub(1).map(|i| entries[i]);
        let earlier_boundary = below.checked_sub(1).map(|i| self.boundaries[i]);
        if let Some((other, other_spacing)) = earlier
            && earlier_boundary.is_none_or(|b| other > b)
            && other_spacing != spacing
        {
            return true;
        }
        let later_start = entries.partition_point(|&(other, _)| other <= line);
        let later = entries.get(later_start).copied();
        let later_boundary = self.boundaries[self.boundaries.partition_point(|&b| b <= line)..]
            .first()
            .copied();
        later.is_some_and(|(other, other_spacing)| {
            later_boundary.is_none_or(|b| other < b) && other_spacing != spacing
        })
    }

    fn build_spacing_index(&mut self) {
        if self.spacing_index.is_none() {
            let mut index: HashMap<u32, Vec<(u32, i64)>> = HashMap::new();
            for (line, indices) in self.by_line.iter().enumerate() {
                let line = line as u32;
                if line == 0 || self.lines.comment_line(line) {
                    continue;
                }
                let mut seen = std::collections::HashSet::new();
                for (position, &i) in indices.iter().enumerate() {
                    let column = self.tokens[i].column;
                    if !seen.insert(column) {
                        continue;
                    }
                    if position > 0 {
                        let spacing =
                            self.spacing(&self.tokens[indices[position - 1]], &self.tokens[i]);
                        index.entry(column).or_default().push((line, spacing));
                    }
                }
            }
            self.spacing_index = Some(index);
        }
    }
}

/// Whether `values` holds two different values.
fn varies(mut values: impl Iterator<Item = i64>) -> bool {
    values
        .next()
        .is_some_and(|first| values.any(|value| value != first))
}

#[cfg(test)]
mod tests;
