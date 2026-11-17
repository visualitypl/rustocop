//! `Layout/SpaceBeforeSemicolon` — no space before a semicolon.
//!
//! A port of RuboCop's cop and its `SpaceBeforePunctuation` mixin (97f30c4,
//! MIT). Consecutive pairs of RuboCop's position-sorted tokens
//! (`processed_source.sorted_tokens`, `SourceFile::parser_tokens`) are read:
//! a real `;` token (never one inside a literal, comment or `$;`) that
//! starts on its previous token's line after a gap is reported from that
//! token's end (``Space found before semicolon.``). A block's or lambda's
//! `{` before the gap is exempt, as `Layout/SpaceInsideBlockBraces` is fixed
//! at `space`. Sorting puts a heredoc opener before the rest of its line.
//!
//! The correction removes the gap: blanks between two tokens on one line,
//! whose removal Ruby's lexer cannot notice before a `;`. It is withheld when
//! the gap holds anything but spaces, tabs, vertical tabs or form feeds, and
//! for the listed source observers.
use crate::{
    cops::Cop,
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceBeforeSemicolon;

impl Cop for SpaceBeforeSemicolon {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeSemicolon"
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
        let tokens = file.parser_tokens();
        if !tokens.iter().any(|t| t.kind == "tSEMI") {
            return;
        }
        // A stable sort by start: `sorted_tokens`.
        let mut sorted: Vec<_> = tokens.iter().collect();
        sorted.sort_by_key(|t| t.start);
        for pair in sorted.windows(2) {
            let (first, second) = (pair[0], pair[1]);
            if second.kind != "tSEMI"
                || first.line != second.line
                || second.start <= first.end
                || matches!(first.kind, "tLCURLY" | "tLAMBEG")
            {
                continue;
            }
            let range = first.end..second.start;
            let blank = file.bytes[range.clone()]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c'));
            let correction = blank.then(|| Correction {
                range: range.clone(),
                replacement: String::new(),
            });
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: "Space found before semicolon.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests;
