//! `Style/InlineComment` — opt-in, detection-only trailing comments.
use crate::{cops::Cop, offense::Offense, source::SourceFile};
pub struct InlineComment;
impl Cop for InlineComment {
    fn name(&self) -> &'static str {
        "Style/InlineComment"
    }
    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        for comment in file.comments() {
            let line = file.line_of(comment.start);
            let prefix = &file.bytes[file.line_start(line)..comment.start];
            let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
            let text = &file.bytes[comment.clone()];
            if text.starts_with(b"#") && prefix.iter().all(|b| super::is_ruby_whitespace(*b)) {
                continue;
            }
            if [b"# rubocop:".as_slice(), b"# rustocop:"].iter().any(|p| {
                text.strip_prefix(*p).is_some_and(|rest| {
                    [b"enable".as_slice(), b"disable", b"todo"]
                        .iter()
                        .any(|key| rest.starts_with(key))
                })
            }) {
                continue;
            }
            let (line, col) = file.line_col(comment.start);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: "Avoid trailing inline comments.".into(),
                line,
                end_line: file.line_of(comment.end.saturating_sub(1)),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/inline_comment_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::correction_offenses,
        pipeline::{lint_str, lint_str_with},
    };
    #[test]
    fn pinned_offense_and_no_correction() {
        let source = "two = 1 + 1 # A trailing inline comment\n";
        let found = lint_str_with(&[&InlineComment], source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 13));
        assert_eq!(found[0].message, "Avoid trailing inline comments.");
        assert!(
            correction_offenses(&InlineComment, source)[0]
                .correction
                .is_none()
        );
        assert!(!InlineComment.safe_fixes());
        assert!(!InlineComment.default_enabled());
        assert!(
            lint_str(source)
                .iter()
                .all(|o| o.cop != "Style/InlineComment")
        );
    }
    #[test]
    fn document_comments_follow_upstream_non_hash_classification() {
        let found = lint_str_with(&[&InlineComment], "=begin\ncomment\n=end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 3));
    }
    #[test]
    fn pinned_directives_standalone_comments_and_literal_lookalikes() {
        for source in [
            "two = 1 + 1 # rubocop:disable Layout/ExtraSpacing\n",
            "two = 1 + 1 # rubocop:todo Layout/ExtraSpacing\n",
            "x # rubocop:enable Layout/ExtraSpacing\n",
            "x # rustocop:disable Layout/ExtraSpacing\n",
            "x # rustocop:todo Layout/ExtraSpacing\n",
            "# A standalone comment",
            "  # comment\n",
            "\u{feff}# comment\n",
            "x = '# comment'\n",
            "x = <<~H\n# comment\nH\n",
            "__END__\nx # comment\n",
        ] {
            assert!(
                lint_str_with(&[&InlineComment], source).is_empty(),
                "{source}"
            );
        }
    }
}
