//! `Style/BlockComments` — replace document comments with ordinary comments.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

use super::correction::{contains_magic_comment_key, preserves_line_values, touches_directive};

pub struct BlockComments;

impl Cop for BlockComments {
    fn name(&self) -> &'static str {
        "Style/BlockComments"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
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
        // One bounded source scan, only in correction mode. Removing a leading
        // empty document can promote a previously inert encoding header.
        let header_magic = cx.corrections_requested() && contains_magic_comment_key(&file.bytes);
        for range in file.comments() {
            if !file.bytes[range.clone()].starts_with(b"=begin") {
                continue;
            }
            let (line, col) = file.line_col(range.start);
            let end_line = file.line_of(range.end.saturating_sub(1));
            let correction = cx
                .corrections_requested()
                .then(|| {
                    if (line <= 2 && header_magic)
                        || touches_directive(file, range.clone())
                        || contains_magic_comment_key(&file.bytes[range.clone()])
                    {
                        return None;
                    }
                    let edit = convert(file, range.clone(), line, end_line)?;
                    preserves_line_values(file, &edit).then_some(edit)
                })
                .flatten();
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message: "Do not use block comments.".into(),
                line,
                end_line,
                col,
            });
        }
    }
}

fn convert(
    file: &SourceFile,
    range: std::ops::Range<usize>,
    first: u32,
    last: u32,
) -> Option<Correction> {
    let mut replacement = Vec::new();
    for line in first + 1..last {
        let text = file.line_text(line);
        // Preserve existing comment prefixes except on the first content line,
        // matching the pinned canonical conversion. Empty interior lines get #.
        if line == first + 1 || (!text.is_empty() && !text.starts_with(b"#")) {
            replacement.extend_from_slice(b"# ");
        } else if text.is_empty() {
            replacement.push(b'#');
        }
        replacement.extend_from_slice(text);
        let content_end = file.line_start(line) + text.len();
        replacement.extend_from_slice(&file.bytes[content_end..file.line_start(line + 1)]);
    }
    if !file.bytes[range.clone()].ends_with(b"\n") {
        if replacement.ends_with(b"\r\n") {
            replacement.truncate(replacement.len() - 2);
        } else if replacement.ends_with(b"\n") {
            replacement.pop();
        }
    }
    Some(Correction {
        range,
        // Non-UTF-8 document contents stay detected but unmodified: the edit
        // representation is UTF-8, so never transcode or replace raw bytes.
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/block_comments_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/BlockComments")
            .collect()
    }

    #[test]
    fn pinned_offense_position_message_and_lookalikes() {
        let found = offenses("x\n=begin\ncomment\n=end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 4));
        assert_eq!(found[0].message, "Do not use block comments.");
        for source in [
            "# comment",
            "\"=begin\ntext\n=end\"",
            "x = <<~H\n=begin\ntext\n=end\nH\n",
            "__END__\n=begin\ntext\n=end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_canonical_corrections() {
        for (before, after) in [
            ("=begin\ncomment\n=end\n", "# comment\n"),
            (
                "=begin\ncomment line 1\n\ncomment line 2\n=end\ndef foo\nend\n",
                "# comment line 1\n#\n# comment line 2\ndef foo\nend\n",
            ),
            ("=begin\n=end\ndef foo\nend\n", "def foo\nend\n"),
            (
                "=begin\ncomment line 1\n\ncomment line 2\n=end\n",
                "# comment line 1\n#\n# comment line 2\n",
            ),
        ] {
            assert_correction(&BlockComments, before, after);
        }
    }

    #[test]
    fn preserves_crlf_existing_hashes_marker_suffixes_and_eof() {
        for (before, after) in [
            (
                "=begin\r\ntext\r\n\r\n# existing\r\n=end\r\n",
                "# text\r\n#\r\n# existing\r\n",
            ),
            ("=begin rdoc\ntext\n=end ignored\n", "# text\n"),
            ("=begin\ntext\n=end", "# text"),
            ("=begin\n=end", ""),
        ] {
            assert_correction(&BlockComments, before, after);
        }
    }

    #[test]
    fn withholds_line_value_magic_and_directive_activation() {
        for source in [
            "=begin\ntext\n=end\np __LINE__\n",
            "=begin\ncoding: ASCII-8BIT\n=end\n",
            "=begin\nfrozen_string_literal: true\n=end\nx = \"hello\"\n",
            "=begin\nfrozen-string-literal: true\n=end\nx = \"hello\"\n",
            "=begin\nFrozen_String_Literal: true\n=end\nx = \"hello\"\n",
            "=begin\nCoDiNg: ASCII-8BIT\n=end\n",
            "=begin\n=end\n# coding: ASCII-8BIT\n",
            "#!/usr/bin/ruby\n=begin\n=end\n# coding: ASCII-8BIT\n",
            "x\n=begin\n# rubocop:disable Lint/Debugger\n=end\nbinding.pry\n",
            "x\n=begin\nrustocop:disable Style\n=end\n",
        ] {
            let found = correction_offenses(&BlockComments, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &BlockComments,
            "p __LINE__\n=begin\ntext\n=end\n",
            "p __LINE__\n# text\n",
        );
    }
}
