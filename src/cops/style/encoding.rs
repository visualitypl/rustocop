//! `Style/Encoding` — unnecessary UTF-8 declarations in the magic header.
use super::{
    correction::{preserves_line_values, touches_directive},
    magic_comment::{MagicComment, leading_end},
};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct Encoding;
impl Cop for Encoding {
    fn name(&self) -> &'static str {
        "Style/Encoding"
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
        // Removing a header line can promote a previously inert declaration
        // into Ruby's encoding slot. Compute this conservative guard only once.
        let safe_header = cx.corrections_requested().then(|| safe_header(file));
        let mut comments = file.comments().iter().peekable();
        for line in 1..=file.code_line_count() as u32 {
            let mut start = file.line_start(line);
            if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
                start += 3;
            }
            while comments.peek().is_some_and(|c| c.start < start) {
                comments.next();
            }
            let text = &file.bytes[start..file.line_start(line) + file.line_text(line).len()];
            if text.starts_with(b"#!") {
                continue;
            }
            if !comments.peek().is_some_and(|c| c.start == start) {
                break;
            }
            let comment = MagicComment::parse(text);
            if !comment.valid {
                break;
            }
            if comment
                .encoding
                .is_some_and(|e| e.eq_ignore_ascii_case(b"utf-8"))
            {
                let correction = safe_header.filter(|safe| *safe).and_then(|_| {
                    if touches_directive(file, start..start + text.len()) {
                        return None;
                    }
                    let replacement = comment.without_encoding()?;
                    if replacement.as_bytes() == text {
                        return None;
                    }
                    // Remove only this line's terminator, never surrounding
                    // whitespace: preserve CRLF and directive line boundaries.
                    let end = if replacement.is_empty() {
                        file.line_start(line + 1)
                    } else {
                        start + text.len()
                    };
                    let edit = Correction {
                        range: start..end,
                        replacement,
                    };
                    preserves_line_values(file, &edit).then_some(edit)
                });
                out.push(Offense {
                    correction,
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Unnecessary utf-8 encoding comment.".into(),
                    line,
                    end_line: line,
                    col: 1,
                });
            }
        }
    }
}

fn safe_header(file: &SourceFile) -> bool {
    use regex::bytes::Regex;
    use std::sync::LazyLock;
    static ENCODINGS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i-u:coding\s*[:=]\s*([a-z0-9_-]+))").unwrap());
    if !file.is_utf8() {
        return false;
    }
    let end = leading_end(file);
    !file
        .comments()
        .iter()
        .take_while(|c| c.start < end)
        .any(|c| {
            // Do not promote a noninitial shebang into an executable header.
            if file.bytes[c.clone()].starts_with(b"#!") && file.line_of(c.start) > 1 {
                return true;
            }
            ENCODINGS
                .captures_iter(&file.bytes[c.clone()])
                .any(|m| !m.get(1).unwrap().as_bytes().eq_ignore_ascii_case(b"utf-8"))
        })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/encoding_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str_with;

    #[test]
    fn pinned_detection_positions_and_message() {
        for (source, lines) in [
            ("# encoding: utf-8\ndef foo() end\n", vec![1]),
            ("# Encoding: UTF-8\n", vec![1]),
            (
                "#!/usr/bin/env ruby\n# encoding: utf-8\ndef foo() end\n",
                vec![2],
            ),
            (
                "# encoding: utf-8\n# coding: utf-8\ndef foo() end\n",
                vec![1, 2],
            ),
            (
                "# frozen_string_literal: true\n# encoding: utf-8\nx\n",
                vec![2],
            ),
            ("# vim:filetype=ruby, fileencoding=utf-8\nx\n", vec![1]),
            ("# -*- encoding : utf-8 -*-\ndef foo() 'ä' end\n", vec![1]),
            ("# -*- encoding : utf-8; mode: enh-ruby -*-\nx\n", vec![1]),
        ] {
            let found = lint_str_with(&[&Encoding], source);
            assert_eq!(
                found.iter().map(|o| o.line).collect::<Vec<_>>(),
                lines,
                "{source}"
            );
            for o in found {
                assert_eq!(o.col, 1);
                assert_eq!(o.message, "Unnecessary utf-8 encoding comment.");
            }
        }
    }

    #[test]
    fn pinned_corrections() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("# encoding: utf-8\ndef foo() end\n", "def foo() end\n"),
            ("# Encoding: UTF-8\n", ""),
            (
                "#!/usr/bin/env ruby\n# encoding: utf-8\ndef foo() end\n",
                "#!/usr/bin/env ruby\ndef foo() end\n",
            ),
            (
                "# encoding: utf-8\n# coding: utf-8\ndef foo() end\n",
                "def foo() end\n",
            ),
            (
                "# frozen_string_literal: true\n# encoding: utf-8\nx\n",
                "# frozen_string_literal: true\nx\n",
            ),
            (
                "# vim:filetype=ruby, fileencoding=utf-8\nx\n",
                "# vim: filetype=ruby\nx\n",
            ),
            (
                "# -*- encoding : utf-8 -*-\ndef foo() 'ä' end\n",
                "def foo() 'ä' end\n",
            ),
            (
                "# -*- encoding : utf-8; mode: enh-ruby -*-\nx\n",
                "# -*- mode: enh-ruby -*-\nx\n",
            ),
            (
                "\u{feff}# encoding: utf-8\r\nx\r\n__END__\r\npayload\r\n",
                "\u{feff}x\r\n__END__\r\npayload\r\n",
            ),
            (
                "# -*- encoding: utf-8; frozen_string_literal: true -*-\nx\n",
                "# -*- frozen_string_literal: true -*-\nx\n",
            ),
        ] {
            assert_correction(&Encoding, before, after);
        }
    }

    #[test]
    fn unsafe_header_edits_remain_reported_without_fixes() {
        use crate::cops::testing::assert_correction;
        for source in [
            "# encoding: utf-8\np __LINE__\n",
            "# encoding: utf-8\n# coding: ASCII-8BIT\np \"é\".encoding\n",
            "# coding: ASCII-8BIT\n# encoding: utf-8\np \"é\".encoding\n",
            "# encoding: utf-8 # rubocop:disable Lint/Debugger\nbinding.pry\n",
            "# encoding: utf-8\n# frozen_string_literal: true\np __LINE__\n",
            "# -*- encoding: utf-8 -*- suffix\nx\n",
            "# prefix -*- encoding: utf-8 -*-\nx\n",
            "# -*- encoding: utf-8; \0frozen_string_literal: true -*-\nx\n",
            "# encoding: utf-8; frozen_string_literal: true\nx\n",
            "# frozen_string_literal: true encoding: utf-8\nx\n",
        ] {
            assert_correction(&Encoding, source, source);
            let found = crate::cops::testing::correction_offenses(&Encoding, source);
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }

    #[test]
    fn ambiguous_emacs_delimiters_keep_offenses_without_corrections() {
        for source in [
            "# -*- encoding: utf-8; encoding: utf-8 -*-; frozen_string_literal: true -*-\np \"a\".frozen?\n",
            "# -*- encoding: utf-8; encoding: utf-8 -*-; frozen_string_literal: true -*- -*-\np \"a\".frozen?\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&Encoding, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn vim_split_discards_raw_trailing_empty_fields_before_trimming() {
        for source in [
            "# vim: fileencoding=utf-8, \nx\n",
            "# vim: fileencoding=utf-8, , \nx\n",
            "# vim: fileencoding=utf-8, , , \nx\n",
        ] {
            assert!(lint_str_with(&[&Encoding], source).is_empty(), "{source}");
        }
        // Whitespace-only and interior empty fields survive Ruby's split,
        // even though strip subsequently makes those tokens empty.
        for source in [
            "# vim: fileencoding=utf-8,  \nx\n",
            "# vim: fileencoding=utf-8, , filetype=ruby, , \nx\n",
        ] {
            let found = lint_str_with(&[&Encoding], source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
    }

    #[test]
    fn misplaced_shebangs_are_not_promoted_by_deleting_a_header() {
        crate::cops::testing::assert_correction(
            &Encoding,
            "# encoding: utf-8\n#!/usr/bin/env ruby\nx\n",
            "# encoding: utf-8\n#!/usr/bin/env ruby\nx\n",
        );
    }

    #[test]
    fn many_header_fixes_share_one_bounded_safety_scan() {
        let source = "# encoding: utf-8\n".repeat(5000) + "x\n";
        let plain = lint_str_with(&[&Encoding], &source);
        assert_eq!(plain.len(), 5000);
        assert!(plain.iter().all(|o| o.correction.is_none()));
        let fixed = crate::cops::testing::correction_offenses(&Encoding, &source);
        assert_eq!(fixed.len(), 5000);
        assert!(fixed.iter().all(|o| o.correction.is_some()));
        assert_eq!(
            crate::corrections::apply(source.as_bytes(), &fixed)
                .unwrap()
                .bytes,
            b"x\n"
        );
    }

    #[test]
    fn editor_vertical_tab_whitespace_is_ruby_whitespace() {
        crate::cops::testing::assert_correction(
            &Encoding,
            "# -*- \u{b}encoding: utf-8\u{b} -*-\nx\n",
            "x\n",
        );
    }

    #[test]
    fn pinned_negative_and_literal_lookalikes() {
        for source in [
            "def foo() end\n",
            "# encoding: us-ascii\nx\n",
            "# frozen_string_literal: true\nx\n",
            "# frozen_string_literal: true\n\n# encoding: utf-8\nx\n",
            "x\n# encoding: utf-8\n",
            "# ordinary\n# encoding: utf-8\n",
            "x = <<~H\n# encoding: utf-8\nH\n",
            "=begin\n# encoding: utf-8\n=end\n",
            "__END__\n# encoding: utf-8\n",
        ] {
            assert!(lint_str_with(&[&Encoding], source).is_empty(), "{source}");
        }
    }
}
