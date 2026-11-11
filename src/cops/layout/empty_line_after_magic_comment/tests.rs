//! Examples ported from RuboCop's `spec/rubocop/cop/layout/empty_line_after_magic_comment_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 34
//! pinned examples run at the fixed `NumberOfEmptyLines: 1`: the
//! `NumberOfEmptyLines: 2` context and the four invalid-value examples are
//! replayed at that fixed value (the setting is not configurable, see the CLI
//! tests). Findings, messages and single-pass corrected bytes equal the fixed
//! replay recorded in the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/EmptyLineAfterMagicComment";
const MESSAGE: &str = "Expected at least 1 empty line after magic comments; found 0.";

fn positions(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MESSAGE);
            assert_eq!(o.end_line, o.line);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            (o.line, o.col)
        })
        .collect()
}

fn held(source: &str) {
    let found = correction_offenses(&EmptyLineAfterMagicComment, source);
    assert_eq!(found.len(), 1, "{source:?}");
    assert!(found[0].correction.is_none(), "{source:?}");
}

/// `(spec line, source, fixed-default finding, single-pass correction)`.
type Pinned = (u32, &'static str, Option<(u32, u32)>, &'static str);

const PINNED: &[Pinned] = &[
    (
        5,
        "# frozen_string_literal: true\nclass Foo; end\n",
        Some((2, 1)),
        "# frozen_string_literal: true\n\nclass Foo; end\n",
    ),
    (
        19,
        "# rbs_inline: enabled\nclass Foo; end\n",
        Some((2, 1)),
        "# rbs_inline: enabled\n\nclass Foo; end\n",
    ),
    (
        33,
        "# rbs_inline: disabled\nclass Foo; end\n",
        Some((2, 1)),
        "# rbs_inline: disabled\n\nclass Foo; end\n",
    ),
    (
        47,
        "# rbs_inline: invalid_value\nclass Foo; end\n",
        None,
        "",
    ),
    (
        54,
        "# typed: true\nclass Foo; end\n",
        Some((2, 1)),
        "# typed: true\n\nclass Foo; end\n",
    ),
    (
        68,
        "# warn_indent: true\nclass Foo; end\n",
        Some((2, 1)),
        "# warn_indent: true\n\nclass Foo; end\n",
    ),
    (
        82,
        "# frozen_string_literal: true\n# Documentation for Foo\nclass Foo; end\n",
        Some((2, 1)),
        "# frozen_string_literal: true\n\n# Documentation for Foo\nclass Foo; end\n",
    ),
    (
        98,
        "# encoding: utf-8\n# frozen_string_literal: true\nclass Foo; end\n",
        Some((3, 1)),
        "# encoding: utf-8\n# frozen_string_literal: true\n\nclass Foo; end\n",
    ),
    (
        114,
        "# frozen_string_literal: true\n# encoding: utf-8\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        123,
        "# frozen_string_literal: true\n# shareable_constant_value: literal\n# warn_indent: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        133,
        "# frozen_string_literal: true\n# shareable_constant_value: literal\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        140,
        "# shareable_constant_value: experimental_everything\n# frozen_string_literal: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        149,
        "# frozen_string_literal: true\n# rbs_inline: enabled\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        156,
        "# rbs_inline: enabled\n# frozen_string_literal: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        165,
        "# frozen_string_literal: true\n# rbs_inline: disabled\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        172,
        "# rbs_inline: disabled\n# frozen_string_literal: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        181,
        "# frozen_string_literal: true\n# typed: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        188,
        "# typed: true\n# frozen_string_literal: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        197,
        "# frozen_string_literal: true\n# shareable_constant_value: none\nclass Foo; end\n",
        Some((3, 1)),
        "# frozen_string_literal: true\n# shareable_constant_value: none\n\nclass Foo; end\n",
    ),
    (
        213,
        "# frozen_string_literal: true\n# Hello!\n",
        Some((2, 1)),
        "# frozen_string_literal: true\n\n# Hello!\n",
    ),
    (
        227,
        "# frozen_string_literal: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (235, "", None, ""),
    (239, "# frozen_string_literal: true", None, ""),
    (243, "# frozen_string_literal: true\n\n  \n\n", None, ""),
    (
        247,
        "# frozen_string_literal: true\n\n\nclass Foo; end\n",
        None,
        "",
    ),
    // `NumberOfEmptyLines: 2`, replayed at the fixed 1.
    (
        259,
        "# frozen_string_literal: true\nclass Foo; end\n",
        Some((2, 1)),
        "# frozen_string_literal: true\n\nclass Foo; end\n",
    ),
    (
        274,
        "# frozen_string_literal: true\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        290,
        "# frozen_string_literal: true\n\n\nclass Foo; end\n",
        None,
        "",
    ),
    (
        299,
        "# frozen_string_literal: true\n\n\n\nclass Foo; end\n",
        None,
        "",
    ),
    (309, "# frozen_string_literal: true", None, ""),
    // Invalid `NumberOfEmptyLines` values 0, -1, 1.5 and "two", replayed at
    // the fixed 1: the source alone has no finding.
    (319, "# frozen_string_literal: true", None, ""),
    (319, "# frozen_string_literal: true", None, ""),
    (319, "# frozen_string_literal: true", None, ""),
    (319, "# frozen_string_literal: true", None, ""),
];

#[test]
fn pinned_examples() {
    for &(line, source, finding, after) in PINNED {
        assert_eq!(
            positions(source),
            finding.into_iter().collect::<Vec<_>>(),
            "spec line {line}"
        );
        if finding.is_some() {
            assert_correction(&EmptyLineAfterMagicComment, source, after);
        }
    }
}

#[test]
fn last_magic_comment_before_code() {
    for (source, finding, after) in [
        // Editor forms are magic comments too.
        (
            "# -*- encoding: utf-8 -*-\nx = 1\n",
            Some((2, 1)),
            "# -*- encoding: utf-8 -*-\n\nx = 1\n",
        ),
        (
            "# vim: set ft=ruby, fileencoding=utf-8\nx = 1\n",
            Some((2, 1)),
            "# vim: set ft=ruby, fileencoding=utf-8\n\nx = 1\n",
        ),
        // The last magic comment before the first statement line counts.
        (
            "#!/usr/bin/env ruby\n# frozen_string_literal: true\n# doc\nx = 1\n",
            Some((3, 1)),
            "#!/usr/bin/env ruby\n# frozen_string_literal: true\n\n# doc\nx = 1\n",
        ),
        (
            "# frozen_string_literal: true\n\n# typed: true\n# doc\nx = 1\n",
            Some((4, 1)),
            "# frozen_string_literal: true\n\n# typed: true\n\n# doc\nx = 1\n",
        ),
        // A whitespace-only line is empty.
        ("# frozen_string_literal: true\n  \t\nx = 1\n", None, ""),
        // Comments after the first statement are not header comments.
        ("x = 1\n# frozen_string_literal: true\ny = 2\n", None, ""),
        ("x = 1 # frozen_string_literal: true\ny = 2\n", None, ""),
        // Not magic.
        ("# frozen_string_literal is true\nx = 1\n", None, ""),
        ("# rubocop:disable Style/Foo\nx = 1\n", None, ""),
        // The data section is not a line after the comment.
        ("# frozen_string_literal: true\n__END__\nx\n", None, ""),
        // CRLF and a BOM: the inserted line uses the comment's line ending.
        (
            "\u{feff}# frozen_string_literal: true\r\nx = 1\r\n",
            Some((2, 1)),
            "\u{feff}# frozen_string_literal: true\r\n\r\nx = 1\r\n",
        ),
        // A blank line keeps its own terminator.
        ("# frozen_string_literal: true\r\n\r\nx = 1\r\n", None, ""),
    ] {
        assert_eq!(
            positions(source),
            finding.into_iter().collect::<Vec<_>>(),
            "{source:?}"
        );
        if finding.is_some() {
            assert_correction(&EmptyLineAfterMagicComment, source, after);
        }
    }
}

#[test]
fn held_insertions() {
    for source in [
        "# frozen_string_literal: true\np __FILE__\n",
        "# frozen_string_literal: true\np caller\n",
        "# frozen_string_literal: true\np DATA.read\n__END__\nx\n",
        "# frozen_string_literal: true\np __LINE__\n",
        // Line 2 after a shebang is where Ruby reads the encoding.
        "#!/usr/bin/env ruby -*- frozen_string_literal: true -*-\n# coding=binary\nx = 1\n",
        // A block comment is never split.
        "=begin\n-*- frozen_string_literal: true -*-\n=end\nx = 1\n",
    ] {
        held(source);
    }
}

#[test]
fn metadata_is_fixed() {
    let cop = EmptyLineAfterMagicComment;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.department_selection_enables());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
}

#[test]
fn magic_comment_meaning_survives_the_insertion() {
    // Prism reads the same magic comments before and after the edit.
    for source in [
        "# frozen_string_literal: true\nx = 'a'\n",
        "# encoding: ascii-8bit\n# frozen_string_literal: true\nx = 'a'\n",
        "#!/usr/bin/env ruby\n# encoding: ascii-8bit\nx = 'a'\n",
    ] {
        let found = correction_offenses(&EmptyLineAfterMagicComment, source);
        let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        assert_eq!(applied.count, 1, "{source:?}");
        let before = ruby_prism::parse(source.as_bytes());
        let after = ruby_prism::parse(&applied.bytes);
        let magic = |result: &ruby_prism::ParseResult<'_>| {
            result
                .magic_comments()
                .map(|m| (m.key().to_vec(), m.value().to_vec()))
                .collect::<Vec<_>>()
        };
        assert_eq!(magic(&before), magic(&after), "{source:?}");
        assert_eq!(
            before.frozen_string_literals(),
            after.frozen_string_literals()
        );
    }
}
