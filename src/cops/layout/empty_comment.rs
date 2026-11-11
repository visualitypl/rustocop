//! `Layout/EmptyComment` — a comment with no text.
//!
//! Fixed `AllowBorderComment: true` and `AllowMarginComment: true`: Prism's
//! comment tokens are chunked like upstream's `chunk_while` (each comment
//! starts on the line after the previous one, at the same character column),
//! and every comment of a chunk whose stripped texts are all exactly `#` is
//! reported. A chunk holding any text, a `##` border or an `=begin` document
//! is accepted.
//!
//! The correction is upstream's: an inline comment (code before it on its
//! line, including a heredoc opener) is removed with the spaces and tabs
//! around it; an own-line comment is removed with its whole line and line
//! break (a leading BOM stays). The edit is withheld, finding kept, for
//! the listed source observers, when the
//! removal would reach into a literal token (a blank `%q` delimiter), when the
//! comment touches an operator whose unary or binary reading depends on the
//! next byte, when the removals of the file's own-line empty comments
//! together would move a `#!` or `coding` comment into the first two lines,
//! where Ruby reads the shebang and the source encoding (every such removal
//! before that line is held), and when the line before the comment ends in a
//! backslash, whose continuation would join the next line instead.
use std::ops::Range;

use super::space_before_comment::{bom_end, listed_observer, sign_sensitive};
use crate::{
    cops::Cop,
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct EmptyComment;

struct Comment {
    range: Range<usize>,
    line: u32,
    col: u32,
    empty: bool,
}

/// Ruby's `String#strip` set: whitespace and NUL.
fn strippable(byte: u8) -> bool {
    matches!(
        byte,
        b'\0' | b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'
    )
}

impl Cop for EmptyComment {
    fn name(&self) -> &'static str {
        "Layout/EmptyComment"
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
        let comments: Vec<Comment> = result
            .comments()
            .map(|comment| {
                let location = comment.location();
                let range = text_range(file, location.start_offset(), location.end_offset());
                let text = &file.bytes[range.clone()];
                let kept = text.len() - text.iter().rev().take_while(|&&b| strippable(b)).count();
                let (line, col) = file.line_col(range.start);
                Comment {
                    empty: &text[..kept] == b"#",
                    range,
                    line,
                    col,
                }
            })
            .collect();
        let chunks: Vec<&[Comment]> = comments
            .chunk_by(|a, b| a.line + 1 == b.line && a.col == b.col)
            .filter(|chunk| chunk.iter().all(|comment| comment.empty))
            .collect();
        // Every line a whole-line removal could delete, in order: the
        // superset of the batch's line removals.
        let removable: Vec<u32> = chunks
            .iter()
            .flat_map(|chunk| chunk.iter())
            .filter(|comment| !inline(file, comment))
            .map(|comment| comment.line)
            .collect();
        let mut observed = None;
        for chunk in chunks {
            for comment in chunk {
                out.push(Offense {
                    correction: correction(file, result, comment, &removable).filter(|_| {
                        !*observed.get_or_insert_with(|| listed_observer(file, result))
                    }),
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Source code comment is empty.".into(),
                    line: comment.line,
                    end_line: comment.line,
                    col: comment.col,
                });
            }
        }
    }
}

/// The comment's own bytes, without a line break Prism may include.
fn text_range(file: &SourceFile, start: usize, mut end: usize) -> Range<usize> {
    if end > start && file.bytes[end - 1] == b'\n' {
        end -= 1;
    }
    if end > start && file.bytes[end - 1] == b'\r' && file.bytes.get(end) == Some(&b'\n') {
        end -= 1;
    }
    start..end
}

/// The start of the comment's line, past a leading BOM on line 1.
fn content_start(file: &SourceFile, comment: &Comment) -> usize {
    if comment.line == 1 {
        bom_end(file)
    } else {
        file.line_start(comment.line)
    }
}

/// Whether code precedes the comment on its line.
fn inline(file: &SourceFile, comment: &Comment) -> bool {
    file.bytes[content_start(file, comment)..comment.range.start]
        .iter()
        .any(|&b| !strippable(b))
}

fn correction(
    file: &SourceFile,
    result: &ruby_prism::ParseResult<'_>,
    comment: &Comment,
    removable: &[u32],
) -> Option<Correction> {
    let line_start = content_start(file, comment);
    let start = comment.range.start;
    let range = if inline(file, comment) {
        if sign_sensitive(file.bytes[start - 1]) {
            return None;
        }
        let space = |b: &u8| matches!(b, b' ' | b'\t');
        let before = file.bytes[line_start..start]
            .iter()
            .rev()
            .take_while(|b| space(b))
            .count();
        let after = file.bytes[comment.range.end..]
            .iter()
            .take_while(|b| space(b))
            .count();
        start - before..comment.range.end + after
    } else {
        if moves_header(file, comment.line, removable) || continued(file, comment.line) {
            return None;
        }
        line_start..file.line_start(comment.line + 1)
    };
    if file.overlaps_literal(&result.node(), &range) {
        return None;
    }
    Some(Correction {
        range,
        replacement: String::new(),
    })
}

/// Whether a later line that Ruby could read as a shebang or an encoding
/// comment would land in lines 1–2 if every line in `removable` (sorted) were
/// deleted. Holding every removal before such a line keeps it where it is,
/// so the removals that remain cannot move it either.
fn moves_header(file: &SourceFile, line: u32, removable: &[u32]) -> bool {
    let last = u32::try_from(removable.len())
        .unwrap_or(u32::MAX)
        .saturating_add(2);
    (line + 1..=last).any(|moved| {
        let removed_before = removable.iter().take_while(|&&r| r < moved).count();
        let text = file.line_text(moved);
        moved as usize - removed_before <= 2
            && (memchr::memmem::find(text, b"#!").is_some()
                || text
                    .windows(6)
                    .any(|word| word.eq_ignore_ascii_case(b"coding")))
    })
}

/// Whether the line before `line` ends in a backslash: deleting `line` would
/// make that continuation join the following line.
fn continued(file: &SourceFile, line: u32) -> bool {
    line > 1 && file.line_text(line - 1).last() == Some(&b'\\')
}

#[cfg(test)]
mod tests;
