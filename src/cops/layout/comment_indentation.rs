//! `Layout/CommentIndentation` — an own-line comment is indented like the next
//! non-blank line (one step deeper before `end`, `)`, `}` or `]`; a comment
//! before `else`/`when`/`rescue`-like keywords may also be one step deeper).
//! Fixed `AllowForAlignment: false`, `IndentationWidth` 2 and
//! AccessModifierIndentation `indent`.
use std::ops::Range;

use crate::{
    cops::Cop,
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct CommentIndentation;

const WIDTH: usize = 2;

impl Cop for CommentIndentation {
    fn name(&self) -> &'static str {
        "Layout/CommentIndentation"
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
        let comments: Vec<Range<usize>> = result
            .comments()
            .map(|c| c.location().start_offset()..c.location().end_offset())
            .collect();
        // `processed_source.lines` stop before a data section's `__END__`.
        let last_line = result.data_loc().map_or_else(
            || file.line_of(file.bytes.len()),
            |data| file.line_of(data.start_offset()).saturating_sub(1),
        );
        let lines = Lines { file };
        for (index, comment) in comments.iter().enumerate() {
            let line = file.line_of(comment.start);
            if !lines.own_line(line) {
                continue;
            }
            let next = (line + 1..=last_line)
                .map(|n| file.line_text(n))
                .find(|text| !blank(text));
            let correct = next.map_or(0, |text| {
                leading_space(text) + if less_indented(text) { WIDTH } else { 0 }
            });
            let column = column(file, comment.start);
            if correct == column {
                continue;
            }
            let mut reported = correct;
            if next.is_some_and(two_alternatives) {
                reported += WIDTH;
                if column == reported {
                    continue;
                }
            }
            let delta = correct as isize - column as isize;
            let correction = correction(file, &result.node(), &comments, index, &lines, delta);
            let (line, _) = file.line_col(comment.start);
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Incorrect indentation detected (column {column} instead of {reported})."
                ),
                line,
                end_line: line,
                col: column as u32 + 1,
            });
        }
    }
}

struct Lines<'a> {
    file: &'a SourceFile,
}

impl Lines<'_> {
    /// `own_line_comment?`: the comment's line matches `/\A\s*#/`.
    fn own_line(&self, line: u32) -> bool {
        let text = self.file.line_text(line);
        text.get(leading_space(text)) == Some(&b'#')
    }
}

/// Ruby's `/\s/`.
fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

/// `line =~ /\S/` for a non-blank line.
fn leading_space(text: &[u8]) -> usize {
    text.iter().take_while(|&&b| space(b)).count()
}

/// RuboCop's `String#blank?`: empty or only `lstrip` whitespace (and NUL).
fn blank(text: &[u8]) -> bool {
    text.iter().all(|&b| space(b) || b == 0)
}

/// `/\A\s*(end\b|[)}\]])/`; the fixed AccessModifierIndentation `indent`
/// style never outdents a bare access modifier.
fn less_indented(text: &[u8]) -> bool {
    let rest = &text[leading_space(text)..];
    matches!(rest.first(), Some(b')' | b'}' | b']')) || keyword(rest, b"end")
}

/// `/^\s*(else|elsif|when|in|rescue|ensure)\b/`.
fn two_alternatives(text: &[u8]) -> bool {
    let rest = &text[leading_space(text)..];
    [&b"else"[..], b"elsif", b"when", b"in", b"rescue", b"ensure"]
        .iter()
        .any(|word| keyword(rest, word))
}

/// `word\b`: the word followed by a non-word character or the end.
fn keyword(rest: &[u8], word: &[u8]) -> bool {
    rest.starts_with(word)
        && rest
            .get(word.len())
            .is_none_or(|&b| !(b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80))
}

/// The parser gem's character column of an own-line comment.
fn column(file: &SourceFile, offset: usize) -> usize {
    file.line_col(offset).1 as usize - 1
}

/// `autocorrect`: the comment and every preceding own-line comment at the
/// same column separated from it only by blank lines move by `delta`, as one
/// edit. Like `AlignmentCorrector`, a removal needs spaces/tabs right before
/// the `#`, and nothing inside literal text (interpolation) moves.
fn correction(
    file: &SourceFile,
    root: &ruby_prism::Node<'_>,
    comments: &[Range<usize>],
    index: usize,
    lines: &Lines<'_>,
    delta: isize,
) -> Option<Correction> {
    let mut chain = vec![index];
    let mut below = index;
    for above in (0..index).rev() {
        let (a, b) = (&comments[above], &comments[below]);
        let (a_line, b_line) = (file.line_of(a.start), file.line_of(b.start));
        if column(file, a.start) != column(file, b.start)
            || !lines.own_line(a_line)
            || !(a_line + 1..b_line).all(|n| blank(file.line_text(n)))
        {
            break;
        }
        chain.push(above);
        below = above;
    }
    let mut edits: Vec<(Range<usize>, usize)> = Vec::new();
    for &i in chain.iter().rev() {
        let start = comments[i].start;
        // A first-line `#!` never moves: indenting a shebang breaks direct
        // execution and Ruby's shebang switches, outdenting one creates it.
        if file.line_of(start) == 1 && file.bytes[start..].starts_with(b"#!") {
            continue;
        }
        let range = if delta > 0 {
            start..start
        } else {
            let range = start.checked_sub(delta.unsigned_abs())?..start;
            if !file.bytes[range.clone()]
                .iter()
                .all(|&b| matches!(b, b' ' | b'\t'))
            {
                continue;
            }
            range
        };
        if file.literal_interior_at(root, range.start) {
            continue;
        }
        edits.push((range, if delta > 0 { delta as usize } else { 0 }));
    }
    let span = edits.first()?.0.start..edits.last()?.0.end;
    let mut replacement = Vec::new();
    let mut at = span.start;
    for (range, spaces) in edits {
        replacement.extend_from_slice(&file.bytes[at..range.start]);
        replacement.extend(std::iter::repeat_n(b' ', spaces));
        at = range.end;
    }
    replacement.extend_from_slice(&file.bytes[at..span.end]);
    Some(Correction {
        range: span,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests;
