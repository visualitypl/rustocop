//! `Lint/MisplacedMagicComment` — ignored encoding/frozen pragmas and displaced shebangs.
use super::magic_comment_header::{canonical_encoding, first_code_offset, known_encoding, parse};
use crate::{cops::Cop, offense::Offense, source::SourceFile};
pub struct MisplacedMagicComment;
impl Cop for MisplacedMagicComment {
    fn name(&self) -> &'static str {
        "Lint/MisplacedMagicComment"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        // Cache the valid whole-line prefix once. Rechecking all predecessors
        // for every comment would make a long header quadratic.
        let first_non_magic = (1..=file.line_count() as u32)
            .find(|&line| {
                let text = file.line_text(line);
                !(line == 1 && text.starts_with(b"#!")) && !parse(file, text).valid
            })
            .unwrap_or(file.line_count() as u32 + 1);
        let preceded_only_by_magic = |line: u32| line <= first_non_magic;
        let mut emit = |start: usize, end: usize, message: &str, correction| {
            // The reference source buffer composes UTF8-MAC before counting
            // columns; reuse the existing bounded buffer projection.
            let (line, col) = super::duplicate_match_pattern::buffer_line_col(start, file);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(end.saturating_sub(1).max(start)),
                message: message.into(),
                correction,
            });
        };
        // Only the FIRST later column-zero shebang is considered, even if an
        // unrelated earlier line prevents this particular finding.
        if let Some(comment) = file.comments().iter().find(|comment| {
            file.line_of(comment.start) > 1
                && comment.start == file.line_start(file.line_of(comment.start))
                && file.bytes[comment.start..comment.end].starts_with(b"#!")
        }) && preceded_only_by_magic(file.line_of(comment.start))
        {
            emit(
                comment.start,
                comment.end,
                "A magic comment above a shebang renders the shebang ineffective.",
                None,
            );
        }
        let effective_line = if file.line_text(1).starts_with(b"#!") {
            2
        } else {
            1
        };
        let first_code = first_code_offset(file);
        let starts_line = |comment: &std::ops::Range<usize>| {
            file.line_text(file.line_of(comment.start))
                .iter()
                .copied()
                .find(|b| !matches!(b, 0 | b' ' | b'\t'..=b'\r'))
                == Some(b'#')
        };
        // The encoding comment Ruby honors, on the first line (or the second,
        // below a shebang).
        let effective = file.comments().iter().find(|comment| {
            file.line_of(comment.start) == effective_line
                && starts_line(comment)
                && parse(file, &file.bytes[(*comment).clone()])
                    .encoding
                    .is_some()
        });
        let misplaced_encoding = |comment: &std::ops::Range<usize>| {
            let text = &file.bytes[comment.clone()];
            if !text.starts_with(b"#") || text[1..].contains(&b'#') {
                return false;
            }
            let magic = parse(file, text);
            let line = file.line_of(comment.start);
            magic.valid
                && magic.encoding.as_deref().is_some_and(known_encoding)
                && !(line == effective_line && starts_line(comment) || preceded_only_by_magic(line))
        };
        for comment in file.comments() {
            let text = &file.bytes[comment.clone()];
            // Pinned quoting guard: no second # anywhere, not just directly
            // after the opening hash. Embedded-document text is not a # range.
            if !text.starts_with(b"#") || text[1..].contains(&b'#') {
                continue;
            }
            let magic = parse(file, text);
            if !magic.valid {
                continue;
            }
            if let Some(encoding) = magic.encoding {
                if !known_encoding(&encoding) {
                    continue;
                }
                let line = file.line_of(comment.start);
                let starts_line = file
                    .line_text(line)
                    .iter()
                    .copied()
                    .find(|b| !matches!(b, 0 | b' ' | b'\t'..=b'\r'))
                    == Some(b'#');
                if !(line == effective_line && starts_line || preceded_only_by_magic(line)) {
                    // Move it to the effective line; with an effective comment
                    // there already, drop a duplicate and leave a conflict.
                    let correction = match effective {
                        None => move_comment(file, comment, effective_line),
                        Some(other) => {
                            let other = parse(file, &file.bytes[other.clone()]).encoding;
                            other
                                .is_some_and(|o| {
                                    canonical_encoding(&o) == canonical_encoding(&encoding)
                                })
                                .then(|| crate::offense::Correction {
                                    range: removal_range(file, comment),
                                    replacement: String::new(),
                                })
                        }
                    };
                    emit(
                        comment.start,
                        comment.end,
                        "The `encoding` magic comment is ignored unless placed on the first line (or below a shebang on the first line).",
                        correction,
                    );
                }
            } else if magic.frozen && first_code.is_some_and(|code| comment.start > code) {
                // Not while an encoding comment moves in: both would go to the
                // top in no reliable order.
                let moving_in =
                    effective.is_none() && file.comments().iter().any(misplaced_encoding);
                let target = if effective.is_some() {
                    effective_line + 1
                } else {
                    effective_line
                };
                let correction = (!moving_in)
                    .then(|| move_comment(file, comment, target))
                    .flatten();
                emit(
                    comment.start,
                    comment.end,
                    "The `frozen_string_literal` magic comment is ignored after any code.",
                    correction,
                );
            }
        }
    }
}
/// `comment_removal_range`: its whole lines when the comment starts its
/// line, else the comment with the blanks before it.
fn removal_range(file: &SourceFile, comment: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    let line = file.line_of(comment.start);
    let starts = file.bytes[file.line_start(line)..comment.start]
        .iter()
        .all(|b| matches!(b, 0 | b' ' | b'\t'..=b'\r'));
    if starts {
        return crate::cops::style::correction::whole_lines(file, comment.clone());
    }
    crate::offense::space_left(&file.bytes, comment.start)..comment.end
}

/// `move_comment`: remove it and put it on a line of its own at the target.
fn move_comment(
    file: &SourceFile,
    comment: &std::ops::Range<usize>,
    target: u32,
) -> Option<crate::offense::Correction> {
    let text = String::from_utf8(file.bytes[comment.clone()].to_vec()).ok()?;
    let start = file.line_start(target);
    let mut end = if (target as usize) < file.line_count() {
        file.line_start(target + 1)
    } else {
        file.bytes.len()
    };
    while end > start && matches!(file.bytes[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    let insertion = if file.line_of(comment.start) < target {
        (end..end, format!("\n{text}"))
    } else {
        (start..start, format!("{text}\n"))
    };
    crate::offense::Correction::from_edits(
        &file.bytes,
        vec![(removal_range(file, comment), String::new()), insertion],
    )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/misplaced_magic_comment_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn all_pinned_specs_shared_source_lifecycles_and_encoding_controls() {
        super::super::magic_comment_header::tests::check_rows(&MisplacedMagicComment);
    }
}
