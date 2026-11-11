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
        || file
            .class_method_bindings(root)
            .has_listed_source_observer(file.data_start().is_some())
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
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
    {
        return None;
    }
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
        if target < 0 || (target == 0 && &text[prefix..] == b"__END__") {
            // A partial shift can accidentally align an untouched clause
            // with a moved opener and erase a compiler warning. Preserve
            // the complete source rather than guessing about that change.
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

#[cfg(test)]
mod tests {
    use crate::cops::{
        Cop,
        layout::{AccessModifierIndentation, ArgumentAlignment, AssignmentIndentation},
        testing::{assert_correction, correction_offenses},
    };

    #[test]
    fn closed_observers_hold_every_consumer_without_requiring_a_reader() {
        for (cop, stem) in [
            (&AssignmentIndentation as &dyn Cop, "value =\n1\n"),
            (
                &AccessModifierIndentation,
                "class C\nprivate\n  def f; end\nend\n",
            ),
            (&ArgumentAlignment, "def f(*v); v; end\nf(1,\n2)\n"),
        ] {
            for observer in [
                "p __FILE__",
                "p __dir__",
                "p $0",
                "p $PROGRAM_NAME",
                "p caller",
                "p caller_locations",
                "p method(:f).source_location",
                "SCRIPT_LINES__ = {}; p SCRIPT_LINES__",
                "p DATA\n__END__\npayload",
                "p Object.const_get(:DATA).read\n__END__\npayload",
                "p Object.const_get(\"DATA\").read\n__END__\npayload",
                "p ::Object.const_get('::Object::DATA')\n__END__\npayload",
                "Root = Object; p Root.const_get((:DATA))\n__END__\npayload",
            ] {
                let source = format!("{stem}{observer}\n");
                let found = correction_offenses(cop, &source);
                assert_eq!(found.len(), 1, "{}: {source}", cop.name());
                assert!(found[0].correction.is_none(), "{}: {source}", cop.name());
                assert_correction(cop, &source, &source);
            }
        }
    }

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
