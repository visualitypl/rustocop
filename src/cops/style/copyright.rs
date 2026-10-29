//! `Style/Copyright` — opt-in, fixed notice pattern before any code.
//!
//! The pinned AutocorrectNotice is empty. Keep missing notices visible, but
//! never invent an author/year or insert an empty (nonconforming) notice.
use std::sync::LazyLock;

use regex::bytes::Regex;

use crate::{cops::Cop, offense::Offense, source::SourceFile};

pub struct Copyright;

const NOTICE: &str = r"^Copyright (\(c\) )?2[0-9]{3} .+";
static NOTICE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("(?-u:{NOTICE})")).expect("fixed copyright notice"));

impl Cop for Copyright {
    fn name(&self) -> &'static str {
        "Style/Copyright"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        let mut previous = if file.bytes.starts_with(b"\xef\xbb\xbf") {
            3
        } else {
            0
        };
        for comment in file.comments() {
            // Whitespace/escaped newlines are not code tokens. A semicolon,
            // literal, __END__, etc. ends the leading comment sequence.
            if !only_trivia(&file.bytes[previous..comment.start]) {
                break;
            }
            let text = &file.bytes[comment.clone()];
            // RuboCop normalizes one '#' prefix per token (not per physical
            // line of an =begin document), followed by literal spaces only.
            let text = text.strip_prefix(b"#").map_or(text, |text| {
                &text[text.iter().take_while(|b| **b == b' ').count()..]
            });
            if text
                .split(|b| *b == b'\n')
                .any(|line| NOTICE_PATTERN.is_match(line.strip_suffix(b"\r").unwrap_or(line)))
            {
                return;
            }
            previous = comment.end;
        }
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Include a copyright notice matching /{NOTICE}/ before any code."),
            line: 1,
            end_line: 1,
            col: 1,
        });
    }
}

/// Gaps between cached comments contain no other comment tokens. Recognize
/// Ruby's escaped LF/CRLF as trivia without lexing/parsing the source again.
fn only_trivia(mut gap: &[u8]) -> bool {
    loop {
        gap = &gap[gap
            .iter()
            .take_while(|&&b| super::is_ruby_whitespace(b))
            .count()..];
        if gap.is_empty() {
            return true;
        }
        let Some(rest) = gap
            .strip_prefix(b"\\\n")
            .or_else(|| gap.strip_prefix(b"\\\r\n"))
        else {
            return false;
        };
        gap = rest;
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/copyright_spec.rb at fixed defaults
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::correction_offenses,
        pipeline::{lint_str, lint_str_with},
    };

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&Copyright], source)
    }

    #[test]
    fn pinned_missing_notice_positions_message_and_empty_notice_withholding() {
        for source in [
            "",
            "\n",
            " \t\n",
            "names = []\n",
            "# test\n# test2\nnames = []\n",
            "#!/usr/bin/env ruby\nnames = []\n",
            "# encoding: utf-8\nnames = []\n",
            "#!/usr/bin/env ruby\n# encoding: utf-8\nnames = []\n",
            "# test2\nnames = []\n# Copyright (c) 2015 Acme Inc.\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!(
                found[0].message,
                format!("Include a copyright notice matching /{NOTICE}/ before any code.")
            );
            let found = correction_offenses(&Copyright, source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert!(found[0].correction.is_none());
        }
        assert!(
            lint_str("names = []\n")
                .iter()
                .all(|o| o.cop != "Style/Copyright")
        );
    }

    #[test]
    fn accepts_pinned_leading_inline_and_document_comment_notices() {
        for source in [
            "# Copyright 2015 Acme Inc.\n# test2\nnames = []\n",
            "# test2\n# Copyright (c) 2015 Acme Inc.\nnames = []\n",
            "=begin\nblah, blah, blah\nCopyright 2015 Acme Inc.\n=end\nnames = []\n",
            "\n  # test\n\n  #Copyright 2999 Anyone\n\nnames = []\n",
            "# Copyright 2000 X\n",
            "# Copyright 2026 My Name",
            "\\\n# Copyright 2026 X\np 1\n",
            "# header\n\\\r\n# Copyright 2026 X\np 1\n",
            "#!/usr/bin/env ruby\n# encoding: UTF-8\n# frozen_string_literal: true\n# Copyright 2026 X\np __LINE__\n",
            "\u{feff}# encoding: UTF-8\r\n# Copyright (c) 2026 X\r\np __LINE__\r\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn vertical_tabs_are_trivia_before_leading_notices() {
        for source in [
            "\x0b# Copyright 2026 X\n",
            "# header\n \t\x0b# Copyright 2026 X\n",
            "\x0b\\\n\x0b# Copyright 2026 X\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        assert_eq!(offenses("\x0bx = 1\n\x0b# Copyright 2026 X\n").len(), 1);
    }

    #[test]
    fn only_fixed_pattern_and_pre_code_comments_count() {
        for source in [
            "# copyright 2026 X\n",
            "# Copyright 1999 X\n",
            "# Copyright 3000 X\n",
            "# Copyright 20260 X\n",
            "# Copyright 2026 \n",
            "# Copyright 2026 \r\n",
            "# Copyright (C) 2026 X\n",
            "# Copyright © 2026 X\n",
            "# prefix Copyright 2026 X\n",
            "#\tCopyright 2026 X\n",
            "## Copyright 2026 X\n",
            "# Copyright ２０２６ X\n",
            "x = 1 # Copyright 2026 X\n",
            "\"# Copyright 2026 X\"\n",
            "x = <<~H\n# Copyright 2026 X\nH\n",
            "__END__\n# Copyright 2026 X\n",
            "=begin\n# Copyright 2026 X\n=end\n",
            "=begin\ntext\n=end\nx = 1\n# Copyright 2026 X\n",
            "\"\nCopyright 2026 X\n\"\n",
            "; # Copyright 2026 X\n",
            "p 1 \\\n# Copyright 2026 X\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source:?}");
        }
        // .+ permits even one space, not just an organization name.
        assert!(offenses("# Copyright 2026  \n").is_empty());
    }

    #[test]
    fn directives_filter_the_first_line_offense_for_both_prefixes() {
        for prefix in ["rubocop", "rustocop"] {
            let source = format!("# {prefix}:disable Style/Copyright\np __LINE__\n");
            assert!(offenses(&source).is_empty());
            let source = format!("# header\n# {prefix}:disable Style/Copyright\np __LINE__\n");
            assert_eq!(offenses(&source).len(), 1, "the offense is on line 1");
        }
    }

    #[test]
    fn scans_many_leading_comments_once_and_stops_at_first_code() {
        let header = "# ordinary comment\n".repeat(20_000);
        assert!(offenses(&format!("{header}# Copyright 2026 X\np 1\n")).is_empty());
        assert_eq!(
            offenses(&format!("{header}p 1\n# Copyright 2026 X\n")).len(),
            1
        );
    }
}
