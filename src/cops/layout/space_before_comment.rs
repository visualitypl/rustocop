//! `Layout/SpaceBeforeComment` — an end-of-line comment must not touch the
//! token before it.
//!
//! Upstream compares each comment token with the previous sorted token: the
//! offense is a comment that starts on the same line exactly where that
//! token ends. Prism's comment list gives the comment tokens; the previous
//! token ends at the comment when the byte before it is code (not blank and
//! not a leading BOM), or when a literal token ends there with a blank
//! closing delimiter (`%q x #`). The correction inserts one space; it is
//! withheld for the listed source observers and after an operator
//! whose unary or binary reading depends on the next byte.
use crate::{
    cops::Cop,
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceBeforeComment;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// Bytes Ruby skips as whitespace between tokens.
pub(super) fn blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// Operators whose unary or binary reading depends on whether a blank
/// follows them (`foo -#c` is `foo(-…)`, `foo - #c` is `foo - …`). Changing
/// the byte after one of them can change the AST, so edits there are held.
pub(super) fn sign_sensitive(byte: u8) -> bool {
    matches!(byte, b'+' | b'-' | b'*' | b'&' | b':')
}

/// The end of a leading UTF-8 BOM, which is not a token.
pub(super) fn bom_end(file: &SourceFile) -> usize {
    if file.bytes.starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    }
}

impl Cop for SpaceBeforeComment {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeComment"
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
        let bom = bom_end(file);
        for comment in result.comments() {
            let start = comment.location().start_offset();
            // `=begin` documents start their line; only `#` comments can
            // follow a token.
            if start <= bom || file.bytes.get(start) != Some(&b'#') {
                continue;
            }
            let before = file.bytes[start - 1];
            let adjacent = if blank(before) {
                before != b'\n' && literal_ends_at(file, result, start)
            } else {
                true
            };
            if !adjacent {
                continue;
            }
            let (line, col) = file.line_col(start);
            out.push(Offense {
                correction: (!sign_sensitive(before)).then(|| Correction {
                    range: start..start,
                    replacement: " ".into(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: "Put a space before an end-of-line comment.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

/// Whether a literal token (a blank closing delimiter) ends at `offset`.
fn literal_ends_at(file: &SourceFile, result: &ruby_prism::ParseResult<'_>, offset: usize) -> bool {
    let ranges = file.literal_token_ranges(&result.node());
    let index = ranges.partition_point(|range| range.end < offset);
    ranges.get(index).is_some_and(|range| range.end == offset)
}

#[cfg(test)]
mod tests;
