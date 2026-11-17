//! `Layout/LeadingEmptyLines` — no blank lines before the first token.
//!
//! Upstream reports `processed_source.tokens[0]` when it starts after line 1.
//! Every token counts, `#` and `=begin` comments included; blanks, escaped
//! newlines and a leading BOM are not tokens, and nothing in the `__END__`
//! data section is (Prism stops at `__END__` and at Ruby's `\0`, `^D` and
//! `^Z` end-of-script bytes).
//!
//! The correction removes everything before the token except a leading BOM.
//! It is withheld, finding kept, for the listed source observers
//! and when a comment before the first code token holds a `#!` or a
//! `coding` spelling: the removal, alone or together with the whole-line
//! removals of other cops in the same pass (`Layout/EmptyComment`,
//! `Layout/EmptyLines`), could move that line into lines 1-2, where Ruby
//! reads the shebang switches and the source encoding. An executable
//! `__LINE__` after the edit is held by the shared position barrier.
use super::space_before_comment::{blank, bom_end};
use crate::{
    cops::Cop,
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct LeadingEmptyLines;

const MSG: &str = "Unnecessary blank line at the beginning of the source.";

impl Cop for LeadingEmptyLines {
    fn name(&self) -> &'static str {
        "Layout/LeadingEmptyLines"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_parse(
        &self,
        _result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(token) = first_token(file) else {
            return;
        };
        let (line, col) = file.line_col(token);
        if line == 1 {
            return;
        }
        let correction = (!moves_header(file, token)).then(|| Correction {
            range: bom_end(file)..token,
            replacement: String::new(),
        });
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line: line,
            col,
        });
    }
}

/// The offset of the first token: the first byte past any BOM that is not a
/// blank or an escaped newline, before the data section. `None` when the
/// file holds no token.
fn first_token(file: &SourceFile) -> Option<usize> {
    let bytes = &file.bytes;
    let end = file.data_start().unwrap_or(bytes.len());
    let mut at = bom_end(file);
    while at < end {
        match bytes[at] {
            byte if blank(byte) => at += 1,
            b'\\' if bytes[at + 1..].starts_with(b"\n") => at += 2,
            b'\\' if bytes[at + 1..].starts_with(b"\r\n") => at += 3,
            // Ruby's end-of-script bytes.
            b'\0' | b'\x04' | b'\x1a' => return None,
            _ => return Some(at),
        }
    }
    None
}

/// Whether a comment between the first token and the first code token
/// spells a shebang or an encoding: the only lines that whole-line removals
/// in this pass could bring into Ruby's header lines.
fn moves_header(file: &SourceFile, token: usize) -> bool {
    let comments = file.comments();
    let mut at = token;
    for comment in comments.iter().skip_while(|c| c.end <= token) {
        // Any non-blank byte between two comments is code.
        if file.bytes[at..comment.start]
            .iter()
            .any(|&b| !blank(b) && b != b'\\')
        {
            break;
        }
        let text = &file.bytes[comment.clone()];
        if memchr::memmem::find(text, b"#!").is_some()
            || text
                .windows(6)
                .any(|word| word.eq_ignore_ascii_case(b"coding"))
        {
            return true;
        }
        at = comment.end;
    }
    false
}

#[cfg(test)]
mod tests;
