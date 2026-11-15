//! Examples ported from RuboCop's `spec/rubocop/cop/layout/leading_empty_lines_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All nine
//! `expect_offense`/`expect_no_offenses` examples run at the fixed defaults;
//! findings, messages and single-pass corrected bytes equal the fixed replay
//! recorded in the pinned RuboCop reference run. The `in
//! collaboration` example (spec line 97), which the ordinary shim cannot run
//! (`ArgumentError`, recorded as a shim error), is replayed with its actual
//! two-cop lifecycle below.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/LeadingEmptyLines";
const MESSAGE: &str = "Unnecessary blank line at the beginning of the source.";

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

/// The finding is kept and offers no edit.
fn held(source: &str) {
    let found = correction_offenses(&LeadingEmptyLines, source);
    assert_eq!(found.len(), 1, "{source:?}");
    assert!(found[0].correction.is_none(), "{source:?}");
}

/// `(spec line, source, fixed-default finding, single-pass correction)`.
type Pinned = (u32, &'static str, Option<(u32, u32)>, &'static str);

const PINNED: &[Pinned] = &[
    (5, "", None, ""),
    (9, "\n", None, ""),
    (13, "class Foo\nend\n", None, ""),
    (20, "puts 1\n", None, ""),
    (26, "# something\n", None, ""),
    (32, "\nclass Foo\nend\n", Some((2, 1)), "class Foo\nend\n"),
    (46, "\nputs 1\n", Some((2, 1)), "puts 1\n"),
    (58, "\n# something\n", Some((2, 1)), "# something\n"),
    (70, "\n\nclass Foo\nend\n", Some((3, 1)), "class Foo\nend\n"),
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
            assert_correction(&LeadingEmptyLines, source, after);
        }
    }
}

/// Spec line 97 (`autocorrect in collaboration`): with
/// `Layout/SpaceAroundEqualsInParameterDefault` (fixed `space`) both edits
/// apply in one pass without a conflict.
#[test]
fn collaboration_with_space_around_equals_in_parameter_default() {
    let peer = crate::cops::layout::SpaceAroundEqualsInParameterDefault;
    let source = "\ndef bar(arg =1); end\n";
    let offenses = crate::pipeline::lint_str_with_corrections(&[&LeadingEmptyLines, &peer], source);
    assert_eq!(offenses.len(), 2, "{offenses:?}");
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    assert_eq!(applied.count, 2);
    assert_eq!(applied.bytes, b"def bar(arg = 1); end\n");
}

#[test]
fn first_token_lookalikes() {
    for (source, finding, after) in [
        // A BOM is not a token and stays in place.
        ("\u{feff}\n\nputs 1\n", Some((3, 1)), "\u{feff}puts 1\n"),
        ("\u{feff}puts 1\n", None, ""),
        // Blank, whitespace-only and CRLF lines all go.
        ("  \n\t\n\nputs 1\n", Some((4, 1)), "puts 1\n"),
        ("\r\n\r\nputs 1\r\n", Some((3, 1)), "puts 1\r\n"),
        // Upstream's range ends at the token: its indentation goes too.
        ("\n  puts 1\n", Some((2, 3)), "puts 1\n"),
        // A block comment is a token.
        (
            "\n=begin\nx\n=end\nputs 1\n",
            Some((2, 1)),
            "=begin\nx\n=end\nputs 1\n",
        ),
        // An escaped newline is not a token.
        ("\\\n\nputs 1\n", Some((3, 1)), "puts 1\n"),
        ("puts 1\n\n", None, ""),
        ("  puts 1\n", None, ""),
        // Only the data section follows: no token at all.
        ("\n\n__END__\nx\n", None, ""),
        ("\n", None, ""),
        ("\n\n  \n", None, ""),
    ] {
        assert_eq!(
            positions(source),
            finding.into_iter().collect::<Vec<_>>(),
            "{source:?}"
        );
        if finding.is_some() {
            assert_correction(&LeadingEmptyLines, source, after);
        }
    }
}

#[test]
fn shebang_and_encoding_lines_never_move_into_the_header() {
    for source in [
        // The removal itself would make the shebang or the encoding active.
        "\n#!/usr/bin/env ruby -w\nputs 1\n",
        "\n# coding: binary\np \"\u{e9}\".encoding\n",
        "\n\n# -*- encoding: ascii-8bit -*-\np 1\n",
        // A later comment line that other whole-line removals of the same
        // pass (EmptyComment, EmptyLines) could bring into lines 1-2.
        "\n#\n  # coding: binary\np \"\u{e9}\".encoding\n",
        "\n# frozen_string_literal: true\n\n\n# vim: x=1, fileencoding=binary\np 1\n",
        "\n# a\n#!ruby -w\np 1\n",
    ] {
        held(source);
    }
    // After the first code token nothing can reach lines 1-2.
    assert_correction(
        &LeadingEmptyLines,
        "\nputs 1\n# coding: binary\n",
        "puts 1\n# coding: binary\n",
    );
    assert_correction(
        &LeadingEmptyLines,
        "\n# frozen_string_literal: true\nputs 1\n",
        "# frozen_string_literal: true\nputs 1\n",
    );
}

#[test]
fn metadata_is_fixed() {
    let cop = LeadingEmptyLines;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert!(cop.department_selection_enables());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
}
