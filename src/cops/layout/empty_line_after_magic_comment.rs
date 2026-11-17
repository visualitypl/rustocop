//! `Layout/EmptyLineAfterMagicComment` — an empty line follows the last
//! magic comment.
//!
//! Fixed `NumberOfEmptyLines: 1`. Upstream takes the comments that start
//! before the first statement's line (all of them without statements), and
//! the last one that `MagicComment.parse(...).any?` accepts: a simple
//! `frozen_string_literal`, `encoding`/`coding`, valid `rbs_inline`,
//! `warn_indent`, `shareable_constant_value` or `typed` comment, an Emacs
//! `-*- ... -*-` form or a Vim `fileencoding` form. The lines after it that
//! hold only whitespace are counted; when they run to the end of the file
//! (`__END__` ends it) that is `Layout/TrailingEmptyLines`' concern, otherwise
//! none found is reported at the start of the next line.
//!
//! The correction inserts one line break there, with the magic comment's own
//! line ending. It is withheld, finding kept, for the listed source
//! observers (the shared position barrier holds a later `__LINE__`), inside a
//! `=begin` block comment, and when line 1 is a shebang and line 2 spells
//! `coding`, which would move off the line where Ruby reads the encoding.
use super::space_before_comment::bom_end;
use crate::{
    cops::{Cop, style::magic_comment::MagicComment},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct EmptyLineAfterMagicComment;

const MSG: &str = "Expected at least 1 empty line after magic comments; found 0.";

impl Cop for EmptyLineAfterMagicComment {
    fn name(&self) -> &'static str {
        "Layout/EmptyLineAfterMagicComment"
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
        let code_line = root
            .as_program_node()
            .and_then(|program| program.statements().body().iter().next())
            .map(|first| file.line_of(first.location().start_offset()));
        let Some(magic) = file
            .comments()
            .iter()
            .take_while(|c| code_line.is_none_or(|line| file.line_of(c.start) < line))
            .filter(|c| MagicComment::parse(&file.bytes[(*c).clone()]).valid)
            .last()
        else {
            return;
        };
        let magic_line = file.line_of(magic.start);
        let next = magic_line + 1;
        // `processed_source[next - 1]` must exist and hold more than
        // whitespace (Ruby's `strip` also removes NUL).
        if next > lines(file) || file.line_text(next).iter().all(|&b| blank(b)) {
            return;
        }
        let at = file.line_start(next);
        let held = file.bytes[magic.clone()].starts_with(b"=begin") || moves_encoding(file, next);
        let newline = if file.bytes[..at].ends_with(b"\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        out.push(Offense {
            correction: (!held).then(|| Correction {
                range: at..at,
                replacement: newline.into(),
            }),
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line: next,
            end_line: next,
            col: 1,
        });
    }
}

fn blank(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' | b'\0'
    )
}

/// The number of `processed_source.lines`: the parser buffer's lines (a final
/// line break starts one more, empty line), ending before `__END__`.
fn lines(file: &SourceFile) -> u32 {
    let (code, data) = match file.data_start() {
        Some(start) => (&file.bytes[..start], true),
        None => (&file.bytes[..], false),
    };
    let breaks = memchr::memchr_iter(b'\n', code).count() as u32;
    if data || code.is_empty() {
        breaks
    } else {
        breaks + 1
    }
}

/// Inserting before line 2 moves it to line 3. Ruby reads the encoding from
/// line 2 only after a line-1 shebang.
fn moves_encoding(file: &SourceFile, next: u32) -> bool {
    next == 2
        && file.bytes[bom_end(file)..].starts_with(b"#!")
        && file
            .line_text(2)
            .windows(6)
            .any(|word| word.eq_ignore_ascii_case(b"coding"))
}

#[cfg(test)]
mod tests;
