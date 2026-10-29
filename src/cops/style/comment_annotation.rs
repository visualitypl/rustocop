//! `Style/CommentAnnotation` — six fixed annotation keywords, with a colon.
use std::sync::LazyLock;

use regex::bytes::Regex;
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

use super::correction::{contains_magic_comment_key, touches_directive};

// Ruby's \s is ASCII whitespace, but its word boundaries are Unicode-aware.
// Byte captures retain legacy-encoding note bytes without transcoding them.
static ANNOTATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?im)^(# ?)(TODO|FIXME|OPTIMIZE|HACK|REVIEW|NOTE)\b",
        r"([ \t\r\n\x0b\x0c]*:)?([ \t\r\n\x0b\x0c]+)?",
        r"(?-u:([^ \t\r\n\x0b\x0c]+))?",
    ))
    .expect("fixed annotation pattern")
});

/// Classification shared with documentation checks, at the same fixed keywords.
pub(super) fn annotation_comment(text: &[u8]) -> bool {
    ANNOTATION.captures(text).is_some_and(|parts| {
        let keyword = parts.get(2).expect("keyword capture").as_bytes();
        let colon = parts.get(3).is_some();
        let space = parts.get(4).is_some();
        (colon || space)
            && !(keyword[0].is_ascii_uppercase()
                && keyword[1..].iter().all(u8::is_ascii_lowercase)
                && !colon
                && space
                && parts.get(5).is_some())
    })
}

pub struct CommentAnnotation;

impl Cop for CommentAnnotation {
    fn name(&self) -> &'static str {
        "Style/CommentAnnotation"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let mut previous = None;
        for comment in file.comments() {
            let line = file.line_of(comment.start);
            let prefix = &file.bytes[file.line_start(line)..comment.start];
            let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
            let text = &file.bytes[comment.clone()];
            // Upstream classifies the whole source line: a document's
            // `=begin` line is not a standalone `#` comment, even at column 1.
            let inline = !text.starts_with(b"#") || prefix.iter().any(|b| !b.is_ascii_whitespace());
            let first = previous.is_none_or(|previous| previous < line.saturating_sub(1));
            previous = Some(line);
            if !first && !inline {
                continue;
            }
            // Prism retains the CR in line-comment tokens. It belongs to
            // the CRLF terminator, not to annotation whitespace.
            let text = if text.starts_with(b"#") && file.bytes.get(comment.end) == Some(&b'\n') {
                text.strip_suffix(b"\r").unwrap_or(text)
            } else {
                text
            };
            let Some(parts) = ANNOTATION.captures(text) else {
                continue;
            };
            let keyword = parts.get(2).expect("keyword capture");
            let keyword = std::str::from_utf8(keyword.as_bytes()).expect("ASCII keyword");
            let colon = parts.get(3);
            let space = parts.get(4);
            let note = parts.get(5);
            if colon.is_none() && space.is_none() {
                continue;
            }
            // Capitalized words without a colon are ordinary sentences.
            if colon.is_none()
                && note.is_some()
                && keyword[1..].bytes().all(|b| b.is_ascii_lowercase())
                && keyword.as_bytes()[0].is_ascii_uppercase()
            {
                continue;
            }
            let upper = keyword.to_ascii_uppercase();
            if keyword == upper && colon.is_some() && space.is_some() && note.is_some() {
                continue;
            }
            // AnnotationComment#bounds is relative to the comment token,
            // even when its multiline regex matched inside a document body.
            // Preserve that pinned highlight, but never edit such a document.
            let keyword_match = parts.get(2).unwrap();
            let start = comment.start + parts.get(1).unwrap().len();
            let end =
                start + space.or(colon).unwrap_or(keyword_match).end() - keyword_match.start();
            let message = if note.is_some() {
                format!(
                    "Annotation keywords like `{keyword}` should be all upper case, followed by a colon, and a space, then a note describing the problem."
                )
            } else {
                format!("Annotation comment, with keyword `{keyword}`, is missing a note.")
            };
            let (line, col) = file.line_col(start);
            out.push(Offense {
                correction: (cx.corrections_requested() && note.is_some()
                    // A document's whitespace capture can cross lines. Do not
                    // join it or change an inert magic/directive spelling.
                    && text.starts_with(b"#")
                    && !contains_magic_comment_key(text)
                    && !touches_directive(file, comment.clone()))
                .then(|| Correction {
                    range: start..end,
                    replacement: format!("{upper}: "),
                }),
                cop: self.name(),
                severity: self.severity(),
                message,
                line,
                end_line: file.line_of(end.saturating_sub(1)),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/comment_annotation_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/CommentAnnotation")
            .collect()
    }

    #[test]
    fn pinned_bad_annotations_have_exact_positions_messages_and_fixes() {
        for (bad, keyword, good) in [
            ("TODO make better", "TODO", "TODO: make better"),
            ("TODO:make better", "TODO", "TODO: make better"),
            ("fixme: does not work", "fixme", "FIXME: does not work"),
            (
                "Optimize: does not work",
                "Optimize",
                "OPTIMIZE: does not work",
            ),
            ("note improve this", "note", "NOTE: improve this"),
            ("REVIEW\tthis", "REVIEW", "REVIEW: this"),
        ] {
            let before = format!("  # {bad}\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (1, 5));
            assert_eq!(
                found[0].message,
                format!(
                    "Annotation keywords like `{keyword}` should be all upper case, followed by a colon, and a space, then a note describing the problem."
                )
            );
            assert_correction(&CommentAnnotation, &before, &format!("  # {good}\n"));
        }
    }

    #[test]
    fn missing_notes_remain_uncorrected() {
        for source in ["# HACK:\n", "# HACK \n", "# HACK:  \n"] {
            let found = correction_offenses(&CommentAnnotation, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert_eq!(
                found[0].message,
                "Annotation comment, with keyword `HACK`, is missing a note."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn pinned_correct_comments_and_sentence_lookalikes() {
        for source in [
            "# REVIEW: not sure about this",
            "# OPTIMIZE",
            "# Todo.destroy(1)",
            "# Optimize if you want. I wouldn't recommend it.\n# Hack is a fun game.\n",
            "# Example: There are three reviews, with ranks 1, 2, and 3. A new\n# review is saved with rank 2. The two reviews that originally had\n# ranks 2 and 3 will have their ranks increased to 3 and 4.\n",
            "#  TODO missing colon",
            "## TODO missing colon",
            "# TODO_more",
            "# TODOé text",
            "# TODO : note",
            "# TODO:\t  note",
            "# FIXME.abc",
            "# ISSUE note",
            "\"# TODO text\"",
            "x = <<~H\n# TODO text\nH\n",
            "__END__\n# TODO text\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn only_first_standalone_comment_but_every_inline_comment() {
        assert_correction(
            &CommentAnnotation,
            "# TODO line 1\n# TODO line 2\n# TODO line 3\n",
            "# TODO: line 1\n# TODO line 2\n# TODO line 3\n",
        );
        let source = "items = [\n  '', # TODO Item 1\n  '', # TODO Item 2\n]\n";
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 9), (3, 9)]
        );
        assert_correction(
            &CommentAnnotation,
            source,
            "items = [\n  '', # TODO: Item 1\n  '', # TODO: Item 2\n]\n",
        );
        assert!(offenses("# introductory paragraph\n# TODO something\n").is_empty());
        assert_eq!(
            offenses("# introductory paragraph\n\n# TODO something\n").len(),
            1
        );
    }

    #[test]
    fn withholds_magic_and_directive_edits_and_document_annotations() {
        for source in [
            "# TODO coding: ASCII-8BIT\n",
            "# todo frozen_string_literal: true\n",
            "# todo Frozen-String-Literal: true\n",
            "# TODO # rubocop:disable Lint/Debugger\n",
            "=begin\n# TODO something\n=end\n",
        ] {
            let found = correction_offenses(&CommentAnnotation, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn document_annotation_uses_pinned_document_relative_highlight_without_a_fix() {
        for (source, line) in [
            ("=begin\n# TODO something\n=end\n", 1),
            ("# heading\n=begin\n# TODO more\n=end\n", 2),
        ] {
            let found = correction_offenses(&CommentAnnotation, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, 3));
            assert_eq!(
                found[0].message,
                "Annotation keywords like `TODO` should be all upper case, followed by a colon, and a space, then a note describing the problem."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn crlf_terminators_are_not_annotation_whitespace() {
        for keyword in ["TODO", "FIXME", "OPTIMIZE", "HACK", "REVIEW", "NOTE"] {
            for margin in ["#", "# "] {
                let bare = format!("{margin}{keyword}\r\n");
                assert!(offenses(&bare).is_empty(), "{bare:?}");
            }
        }
        for source in ["# HACK \r\n", "# HACK:\r\n"] {
            let found = correction_offenses(&CommentAnnotation, source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert_eq!(
                found[0].message,
                "Annotation comment, with keyword `HACK`, is missing a note."
            );
            assert!(found[0].correction.is_none());
        }
        assert_correction(
            &CommentAnnotation,
            "# todo improve\r\n",
            "# TODO: improve\r\n",
        );
    }

    #[test]
    fn many_comments_share_the_main_parse_and_do_no_correction_work_in_plain_lint() {
        // 2,000 independent paragraphs/documents; one comment pass per cop,
        // not one full-source scan for every AST node or individual edit.
        let source = "x = 1 # todo improve\n=begin\ntext\n=end\n\n".repeat(2000);
        let plain = crate::pipeline::lint_str_with(
            &[&CommentAnnotation, &super::super::BlockComments],
            &source,
        );
        assert_eq!(plain.len(), 4000);
        assert!(plain.iter().all(|o| o.correction.is_none()));
        let found = crate::pipeline::lint_str_with_corrections(
            &[&CommentAnnotation, &super::super::BlockComments],
            &source,
        );
        assert_eq!(found.len(), 4000);
        assert!(found.iter().all(|o| o.correction.is_some()));
    }

    #[test]
    fn all_six_keywords_and_no_space_margin_work() {
        for keyword in ["TODO", "FIXME", "OPTIMIZE", "HACK", "REVIEW", "NOTE"] {
            assert_correction(
                &CommentAnnotation,
                &format!("#{keyword} something\n"),
                &format!("#{keyword}: something\n"),
            );
        }
    }
}
