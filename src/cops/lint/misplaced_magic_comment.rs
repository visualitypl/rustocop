//! `Lint/MisplacedMagicComment` — ignored encoding/frozen pragmas and displaced shebangs.
use super::magic_comment_header::{first_code_offset, known_encoding, parse};
use crate::{cops::Cop, offense::Offense, source::SourceFile};
pub struct MisplacedMagicComment;
impl Cop for MisplacedMagicComment {
    fn name(&self) -> &'static str {
        "Lint/MisplacedMagicComment"
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
        let mut emit = |start: usize, end: usize, message: &str| {
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
                correction: None,
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
            );
        }
        let effective_line = if file.line_text(1).starts_with(b"#!") {
            2
        } else {
            1
        };
        let first_code = first_code_offset(file);
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
                    emit(
                        comment.start,
                        comment.end,
                        "The `encoding` magic comment is ignored unless placed on the first line (or below a shebang on the first line).",
                    );
                }
            } else if magic.frozen && first_code.is_some_and(|code| comment.start > code) {
                emit(
                    comment.start,
                    comment.end,
                    "The `frozen_string_literal` magic comment is ignored after any code.",
                );
            }
        }
    }
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
