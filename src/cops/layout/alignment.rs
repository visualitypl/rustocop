//! Small fixed-default projection shared by the alignment consumers.
//! Anchors are display columns; diagnostics and edits retain original byte ranges.
use crate::{offense::Correction, source::SourceFile};
use ruby_prism::Node;
use std::ops::Range;
mod display_width;

pub(super) const INDENT: usize = 2;

fn prefix(file: &SourceFile, offset: usize) -> &[u8] {
    let start = file.line_start(file.line_of(offset));
    let prefix = &file.bytes[start..offset];
    if start == 0 {
        prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix)
    } else {
        prefix
    }
}

pub(super) fn effective_column(file: &SourceFile, offset: usize) -> usize {
    let projected = file.match_pattern_source(prefix(file, offset));
    std::str::from_utf8(&projected).map_or(projected.len(), |s| s.chars().count())
}

pub(super) fn display_column(file: &SourceFile, offset: usize) -> usize {
    let prefix = prefix(file, offset);
    // The existing bounded reference-buffer projection handles legacy encodings
    // and UTF8-MAC without transcoding offsets or changing diagnostic columns.
    let projected = file.match_pattern_source(prefix);
    std::str::from_utf8(&projected).map_or(projected.len(), display_width::width)
}

pub(super) fn starts_line(file: &SourceFile, offset: usize) -> bool {
    // Ruby's /\s/ includes vertical tab, unlike Rust's ASCII whitespace set.
    prefix(file, offset)
        .iter()
        .all(|b| b.is_ascii_whitespace() || *b == b'\x0b')
}

/// Port of `EndKeywordAlignment#check_end_kw_alignment` for one fixed style:
/// `end` is aligned when it is on the anchor's first line or at the anchor's
/// display column. Otherwise returns the upstream message, whose columns are
/// 0-based characters.
pub(super) fn misaligned_end(
    file: &SourceFile,
    anchor: &Range<usize>,
    end: usize,
) -> Option<String> {
    let (align_line, align_col) = file.line_col(anchor.start);
    let (line, col) = file.line_col(end);
    if line == align_line || display_column(file, anchor.start) == display_column(file, end) {
        return None;
    }
    Some(format!(
        "`end` at {line}, {} is not aligned with `{}` at {align_line}, {}.",
        col - 1,
        String::from_utf8_lossy(&file.bytes[anchor.clone()]),
        align_col - 1,
    ))
}

/// `EndKeywordAlignment#start_line_range`: the line holding `offset`, from its
/// first to its last non-whitespace character (Ruby's `\s`).
pub(super) fn start_line_range(file: &SourceFile, offset: usize) -> Range<usize> {
    let line = file.line_of(offset);
    let mut start = file.line_start(line);
    let mut text = file.line_text(line);
    if line == 1
        && let Some(rest) = text.strip_prefix(b"\xef\xbb\xbf")
    {
        start += 3;
        text = rest;
    }
    let blank = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r');
    let first = text.iter().position(|b| !blank(b)).unwrap_or(0);
    let last = text
        .iter()
        .rposition(|b| !blank(b))
        .map_or(first, |i| i + 1);
    start + first..start + last.max(first)
}

/// RuboCop's `AlignmentCorrector.align_end`: the whitespace before `end`
/// becomes `indentation`, or `end` moves to its own, indented line.
pub(super) fn align_end(file: &SourceFile, end: usize, indentation: &str) -> Correction {
    let line = file.line_of(end);
    let mut start = file.line_start(line);
    if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
        start = start.max(3);
    }
    if file.bytes[start..end].iter().all(u8::is_ascii_whitespace) {
        Correction {
            range: start..end,
            replacement: indentation.to_owned(),
        }
    } else {
        Correction {
            range: end..end,
            replacement: format!("\n{indentation}"),
        }
    }
}

/// Port of `AlignmentCorrector.align_end` for a closing keyword (`end` or
/// `}`) that begins its line: the bytes between the line start and `end` are
/// replaced by `indentation`. Like `Layout/EndAlignment`, no newline is ever
/// inserted before an `end` that follows code, so only a prefix made of spaces
/// and tabs is rewritten. The edit is withheld inside literal text (heredoc
/// bodies, interpolation in literal interiors) and when the file contains one
/// of the closed list of source observers. Callers check
/// `corrections_requested` first.
pub(super) fn end_indentation(
    file: &SourceFile,
    root: &Node<'_>,
    end: usize,
    indentation: &str,
) -> Option<Correction> {
    let line = file.line_of(end);
    let mut start = file.line_start(line);
    if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
        start = start.max(3);
    }
    let range = start..end;
    if !file.bytes[range.clone()]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t'))
        || file.bytes[range.clone()] == *indentation.as_bytes()
        || file.literal_interior_at(root, start)
        || file.overlaps_literal(root, &range)
    {
        return None;
    }
    Some(Correction {
        range,
        replacement: indentation.to_owned(),
    })
}

pub(super) struct Alignment {
    pub expression: Range<usize>,
    pub delta: i64,
}
impl Alignment {
    pub fn new(file: &SourceFile, expression: Range<usize>, expected: usize) -> Option<Self> {
        let delta = expected as i64 - display_column(file, expression.start) as i64;
        (delta != 0).then_some(Self { expression, delta })
    }

    /// Port of the space-style AlignmentCorrector, with source-preserving
    /// withholding. No newlines/tokens are introduced; literal bytes and DATA
    /// boundaries stay untouched. The existing file-wide literal index is lazy,
    /// shared and binary searched, not rebuilt for each RHS or modifier.
    pub fn correction(&self, file: &SourceFile, root: &Node<'_>) -> Option<Correction> {
        let first = file.line_of(self.expression.start);
        let last = file.line_of(self.expression.end.saturating_sub(1));
        if !starts_line(file, self.expression.start) {
            return None;
        }
        shift_lines(file, root, first..=last, self.expression.end, self.delta)
    }
}

/// Shift the indentation of `lines` (ascending, 1-based) by `delta` columns
/// as one correction ending at `end`, with the safeguards of
/// [`Alignment::correction`]. Used directly where upstream realigns a set of
/// whole lines (`Layout/MultilineMethodCallIndentation`'s block chains).
pub(super) fn shift_lines(
    file: &SourceFile,
    root: &Node<'_>,
    lines: impl IntoIterator<Item = u32> + Clone,
    end: usize,
    delta: i64,
) -> Option<Correction> {
    let first = lines.clone().into_iter().next()?;
    let start = file.line_start(first);
    // Reuse the existing lazy observer walk, but not older families' broader
    // source-reader proofs. Listed observers hold even without a raw read.
    let comments = file.comments();
    let index = comments.partition_point(|c| c.end <= start);
    if comments[index..]
        .iter()
        .take_while(|c| c.start < end)
        .any(|c| file.bytes[c.clone()].starts_with(b"=begin"))
    {
        return None;
    }
    let mut replacement = Vec::new();
    let mut cursor = start;
    for line in lines {
        let line_start = file.line_start(line);
        let text = file.line_text(line);
        let prefix = text
            .iter()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'\x0b'))
            .count();
        if prefix == text.len() || line_start + prefix >= end {
            continue;
        }
        // Shift before literal openers, never inside a literal, including
        // interpolation and empty indentation at heredoc boundaries.
        if file.literal_interior_at(root, line_start)
            || file.overlaps_literal(root, &(line_start..line_start + prefix))
        {
            continue;
        }
        // Tabs have a one-column reference width but native indentation
        // uses tab stops. Removing bytes (from opposite prefix ends on the
        // first and later lines) need not preserve keyword/end alignment.
        // Vertical tabs have zero display width, so removing prefix bytes
        // is not a column-preserving shift either. Withhold the complete
        // negative shift, including earlier edits.
        if delta < 0 && text[..prefix].iter().any(|b| matches!(b, b'\t' | b'\x0b')) {
            return None;
        }
        let target = prefix as i64 + delta;
        // AlignmentCorrector removes only whitespace: a line indented less
        // than the shift stays as it is.
        if target < 0 {
            continue;
        }
        if target == 0 && &text[prefix..] == b"__END__" {
            return None;
        }
        let range = line_start..line_start + prefix;
        replacement.extend_from_slice(&file.bytes[cursor..range.start]);
        if delta > 0 {
            replacement.extend_from_slice(&file.bytes[range.clone()]);
            replacement.extend(std::iter::repeat_n(b' ', delta as usize));
        } else {
            let remove = (-delta) as usize;
            let retained = if line == first {
                range.start..range.end - remove
            } else {
                range.start + remove..range.end
            };
            replacement.extend_from_slice(&file.bytes[retained]);
        }
        cursor = range.end;
    }
    replacement.extend_from_slice(&file.bytes[cursor..end]);
    let replacement = String::from_utf8(replacement).ok()?;
    (replacement.as_bytes() != &file.bytes[start..end]).then_some(Correction {
        range: start..end,
        replacement,
    })
}

/// `Layout/EndAlignment`'s `EnforcedStyleAlignWith`, which
/// `Layout/ElseAlignment` and `Layout/IndentationWidth` read too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EndAlignWith {
    #[default]
    Keyword,
    Variable,
    StartOfLine,
}

impl EndAlignWith {
    /// The style a `rustocop.toml` names.
    pub fn parse(name: &str) -> Self {
        match name {
            "variable" => EndAlignWith::Variable,
            "start_of_line" => EndAlignWith::StartOfLine,
            _ => EndAlignWith::Keyword,
        }
    }

    /// `EndKeywordAlignment#variable_alignment?`: whether a construct on the
    /// right of `owner` (see [`assignment_owner`]) takes the owner as its
    /// base: in a style other than `keyword`, when the construct starts on
    /// the owner's first line.
    pub(super) fn aligns_with_owner(
        self,
        file: &SourceFile,
        owner: &Node<'_>,
        rhs: &Node<'_>,
    ) -> bool {
        self != EndAlignWith::Keyword
            && file.line_of(rhs.location().start_offset())
                <= file.line_of(owner.location().start_offset())
    }
}

fn span(node: &Node<'_>) -> (usize, usize) {
    (node.location().start_offset(), node.location().end_offset())
}

/// RuboCop's `CheckAssignment`, from the right-hand side: the assignment,
/// or the call with arguments, whose `extract_rhs` (the value, or the last
/// argument) leads to `node` through `first_part_of_call_chain` (receivers
/// of calls). With `leading`, as `EndAlignment#leading_node`, the way then
/// goes on through `(…)`, `||` and `&&` (their first child). A call with
/// `&.` counts only with `csend` (a cop that also registers `on_csend`).
/// `ancestors` are innermost first.
pub(super) fn assignment_owner<'a, 'pr: 'a>(
    node: &Node<'pr>,
    ancestors: impl Iterator<Item = &'a Node<'pr>>,
    leading: bool,
    csend: bool,
) -> Option<&'a Node<'pr>> {
    let ancestors: Vec<&Node<'pr>> = ancestors.collect();
    let mut current = span(node);
    let mut i = 0;
    while leading && let Some(ancestor) = ancestors.get(i) {
        let first_statement = ancestor
            .as_statements_node()
            .and_then(|statements| statements.body().iter().next());
        if first_statement.is_some_and(|first| span(&first) == current)
            && let Some(parentheses) = ancestors
                .get(i + 1)
                .filter(|p| p.as_parentheses_node().is_some())
        {
            current = span(parentheses);
            i += 2;
        } else if ancestor
            .as_or_node()
            .is_some_and(|or| span(&or.left()) == current)
            || ancestor
                .as_and_node()
                .is_some_and(|and| span(&and.left()) == current)
        {
            current = span(ancestor);
            i += 1;
        } else {
            break;
        }
    }
    while let Some(call) = ancestors.get(i).and_then(|n| n.as_call_node())
        && call
            .receiver()
            .is_some_and(|receiver| span(&receiver) == current)
    {
        current = span(ancestors[i]);
        i += 1;
    }
    let parent = ancestors.get(i)?;
    if crate::cops::ast::is_assignment(parent) {
        let value = super::multiline_expression::assignment_value(parent)?;
        return (span(&value) == current).then_some(*parent);
    }
    let last = parent.as_arguments_node()?.arguments().iter().last()?;
    let owner = ancestors.get(i + 1)?;
    let call = owner.as_call_node()?;
    let block_pass = call
        .block()
        .is_some_and(|b| b.as_block_argument_node().is_some());
    (span(&last) == current && !block_pass && (csend || !call.is_safe_navigation()))
        .then_some(*owner)
}

#[cfg(test)]
mod tests {
    use crate::cops::{
        Cop,
        layout::{AccessModifierIndentation, ArgumentAlignment, AssignmentIndentation},
        testing::assert_correction,
    };

    #[test]
    fn out_of_scope_readers_and_observer_lookalikes_allow_every_consumer() {
        for (cop, stem, fixed) in [
            (
                &AssignmentIndentation as &dyn Cop,
                "value =\n1\n",
                "value =\n  1\n",
            ),
            (
                &AccessModifierIndentation,
                "class C\nprivate\n  def f; end\nend\n",
                "class C\n  private\n  def f; end\nend\n",
            ),
            (&ArgumentAlignment, "f(1,\n2)\n", "f(1,\n  2)\n"),
        ] {
            for reader in [
                "p File.read('example.rb')",
                "F = File; p F.read(path)",
                "p STDIN.read",
                "p input.read",
                "system('cat example.rb')",
                "p `cat example.rb`",
                "for line in STDIN; p line; end",
                "p [*STDIN]",
                "p STDIN.tell",
                "p STDIN.eof?",
                "C.new",
                "p DATA",
                "p 'Object.const_get(:DATA)'\n__END__\npayload",
                "# __FILE__ __dir__ $0 $PROGRAM_NAME caller caller_locations source_location SCRIPT_LINES__ DATA\np 'caller __FILE__ SCRIPT_LINES__'",
                "__END__\n__FILE__ __dir__ DATA $0 $PROGRAM_NAME caller caller_locations source_location SCRIPT_LINES__",
            ] {
                assert_correction(
                    cop,
                    &format!("{stem}{reader}\n"),
                    &format!("{fixed}{reader}\n"),
                );
            }
        }
    }
}
