//! `Layout/LeadingCommentSpace` — a `#` comment needs a space after its
//! leading `#`s.
//!
//! Upstream's matcher at the fixed defaults (every optional annotation
//! allowance — Doxygen, Gemfile `#ruby`, RBS::Inline, Steep, YARD `#-` —
//! false): a comment whose text matches `\A(?!#\+\+|#--)(#+[^#\s=])`, except
//! a `#!` comment or (in a file named `config.ru`) a `#\` rackup comment
//! starting on line 1, and a `#!` comment continuing an accepted `#!` comment
//! on the line before (multi-line nix-shell shebangs). `=begin` documents,
//! RDoc `#++`/`#--`, sprockets `#=` and `#` runs are accepted. The correction
//! inserts one space after the first `#` (withheld for the listed
//! source observers): Ruby's magic comments and
//! rubocop/rustocop directives allow blanks after `#`, so their meaning is
//! unchanged, and a comment's lexing ends at its line break.
use std::collections::{HashMap, HashSet};

use crate::{
    cops::Cop,
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct LeadingCommentSpace;

/// `\A(?!#\+\+|#--)(#+[^#\s=])`, with Ruby's ASCII `\s`.
fn missing_space(text: &[u8]) -> bool {
    if text.starts_with(b"#++") || text.starts_with(b"#--") {
        return false;
    }
    let hashes = text.iter().take_while(|&&b| b == b'#').count();
    hashes > 0
        && text
            .get(hashes)
            .is_some_and(|&b| b != b'=' && !super::space_before_comment::blank(b))
}

impl Cop for LeadingCommentSpace {
    fn name(&self) -> &'static str {
        "Layout/LeadingCommentSpace"
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
        let rackup = file
            .path
            .file_name()
            .is_some_and(|name| name == "config.ru");
        // `comment_at_line`: the last comment starting on each line.
        let mut at_line: HashMap<u32, usize> = HashMap::new();
        // Starts of the comments reported so far (`current_offense_locations`).
        let mut reported: HashSet<usize> = HashSet::new();
        for comment in result.comments() {
            let location = comment.location();
            let start = location.start_offset();
            let text = &file.bytes[start..location.end_offset()];
            let line = file.line_of(start);
            let previous = line.checked_sub(1).and_then(|l| at_line.get(&l)).copied();
            at_line.insert(line, start);
            if !missing_space(text) {
                continue;
            }
            let shebang = text.starts_with(b"#!");
            if line == 1 && (shebang || rackup && text.starts_with(b"#\\")) {
                continue;
            }
            let continuation = shebang
                && previous.is_some_and(|previous| {
                    file.bytes[previous..].starts_with(b"#!") && !reported.contains(&previous)
                });
            if continuation {
                continue;
            }
            reported.insert(start);
            let (line, col) = file.line_col(start);
            out.push(Offense {
                correction: Some(Correction {
                    range: start + 1..start + 1,
                    replacement: " ".into(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: "Missing space after `#`.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests;
