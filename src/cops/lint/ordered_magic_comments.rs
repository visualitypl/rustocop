//! `Lint/OrderedMagicComments` — encoding must precede other header pragmas.
use super::magic_comment_header::{leading_lines, parse};
use crate::{cops::Cop, offense::Offense, source::SourceFile};
pub struct OrderedMagicComments;
impl Cop for OrderedMagicComments {
    fn name(&self) -> &'static str {
        "Lint/OrderedMagicComments"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        let (mut encoding, mut other) = (None, None);
        for line in 1..=leading_lines(file) {
            let magic = parse(file, file.line_text(line));
            if magic.encoding.is_some() {
                encoding = Some(line);
            } else if magic.valid {
                other = Some(line);
            }
            // Upstream retains the last index of each kind UNTIL both have
            // been seen, then returns the first pair. No later run is checked.
            if let (Some(encoding), Some(other)) = (encoding, other) {
                if encoding > other {
                    // Swap the two lines' contents.
                    let content = |line: u32| {
                        let start = file.line_start(line);
                        let end = file.bytes[start..]
                            .iter()
                            .position(|&b| b == b'\n')
                            .map_or(file.bytes.len(), |i| start + i);
                        start..end
                    };
                    let (a, b) = (content(encoding), content(other));
                    let text = |r: &std::ops::Range<usize>| {
                        String::from_utf8(file.bytes[r.clone()].to_vec()).ok()
                    };
                    let correction = text(&a).zip(text(&b)).and_then(|(ta, tb)| {
                        crate::offense::Correction::from_edits(
                            &file.bytes,
                            vec![(a.clone(), tb), (b.clone(), ta)],
                        )
                    });
                    out.push(Offense {
                        cop: self.name(),
                        severity: self.severity(),
                        line: encoding,
                        col: 1,
                        end_line: encoding,
                        message:
                            "The encoding magic comment should precede all other magic comments."
                                .into(),
                        correction,
                    });
                }
                break;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/ordered_magic_comments_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn all_pinned_specs_shared_source_lifecycles_and_encoding_controls() {
        super::super::magic_comment_header::tests::check_rows(&OrderedMagicComments);
    }
}
