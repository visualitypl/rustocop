//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_inside_string_interpolation_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 12
//! pinned examples run at the fixed `EnforcedStyle: no_space`, the `space`
//! context's included; findings, messages and single-pass corrected bytes
//! equal the fixed replay recorded in the pinned RuboCop reference run
//! (each source and corrected SHA-256 was checked against it when the rows
//! were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceInsideStringInterpolation";

fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceInsideStringInterpolation], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.line, o.end_line);
            (o.line, o.col)
        })
        .collect();
    found.sort_unstable();
    found
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceInsideStringInterpolation, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const ILL: &str = "\"#{ var}\"\n\"#{var }\"\n\"#{   var   }\"\n\"#{var\t}\"\n\"#{\tvar\t}\"\n\"#{\tvar}\"\n\"#{ \t var \t \t}\"\n";
const ILL_FINDINGS: &[(u32, u32)] = &[
    (1, 4),
    (2, 7),
    (3, 4),
    (3, 10),
    (4, 7),
    (5, 4),
    (5, 8),
    (6, 4),
    (7, 4),
    (7, 10),
];
const ILL_FIXED: &str =
    "\"#{var}\"\n\"#{var}\"\n\"#{var}\"\n\"#{var}\"\n\"#{var}\"\n\"#{var}\"\n\"#{var}\"\n";

const PINNED: &[Pinned] = &[
    (9, ILL, ILL_FINDINGS, ILL_FIXED),
    (
        41,
        "/regexp #{ var}/\n`backticks #{ var}`\n:\"symbol #{ var}\"\n",
        &[(1, 11), (2, 14), (3, 12)],
        "/regexp #{var}/\n`backticks #{var}`\n:\"symbol #{var}\"\n",
    ),
    (60, "\"#{ var }\"\n", &[(1, 4), (1, 8)], "\"#{var}\"\n"),
    (73, "\"#{ a; b }\"\n", &[(1, 4), (1, 9)], "\"#{a; b}\"\n"),
    (
        86,
        "\"Variable is    #{var}      \"\n\"  Variable is  #{var}\"\n",
        &[],
        "",
    ),
    (94, "\"#{}\"", &[], ""),
    (99, "\"#{\n  code\n}\"\n", &[], ""),
    (
        107,
        "def foo\n  \"#{ # comment\n    code\n  }\"\nend\n",
        &[],
        "",
    ),
    // The `EnforcedStyle: space` context, replayed at the fixed `no_space`.
    (123, ILL, ILL_FINDINGS, ILL_FIXED),
    (152, "\"#{var}\"\n", &[], ""),
    (
        166,
        "\"Variable is    #{ var }      \"\n\"  Variable is  #{ var }\"\n",
        &[(1, 19), (1, 23), (2, 19), (2, 23)],
        "\"Variable is    #{var}      \"\n\"  Variable is  #{var}\"\n",
    ),
    (174, "\"#{}\"", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 12);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceInsideStringInterpolation, source).is_empty());
        } else {
            assert_correction(&SpaceInsideStringInterpolation, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceInsideStringInterpolation;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn every_interpolating_literal_is_checked() {
    for (source, expected, after) in [
        (
            "x = \"a #{ b } c\"\n",
            &[(1, 10), (1, 12)][..],
            "x = \"a #{b} c\"\n",
        ),
        ("x = :\"#{ b}\"\n", &[(1, 9)], "x = :\"#{b}\"\n"),
        ("x = /#{ b }/i\n", &[(1, 8), (1, 10)], "x = /#{b}/i\n"),
        ("x = %r{#{b }}\n", &[(1, 11)], "x = %r{#{b}}\n"),
        ("x = `ls #{ b}`\n", &[(1, 11)], "x = `ls #{b}`\n"),
        ("x = %x(ls #{b })\n", &[(1, 14)], "x = %x(ls #{b})\n"),
        (
            "x = %W(a#{ b } c)\n",
            &[(1, 11), (1, 13)],
            "x = %W(a#{b} c)\n",
        ),
        ("x = %I(a#{ b })\n", &[(1, 11), (1, 13)], "x = %I(a#{b})\n"),
        ("x = %(#{ b })\n", &[(1, 9), (1, 11)], "x = %(#{b})\n"),
        ("x = %Q[#{ b }]\n", &[(1, 10), (1, 12)], "x = %Q[#{b}]\n"),
        ("x = %s(#{ b })\n", &[], ""),
        (
            "if /#{ b }/ then 1 end\n",
            &[(1, 7), (1, 9)],
            "if /#{b}/ then 1 end\n",
        ),
        (
            "x = <<~A\n  #{ b }\nA\n",
            &[(2, 5), (2, 7)],
            "x = <<~A\n  #{b}\nA\n",
        ),
        (
            "x = <<-A\n#{ b }\n  A\n",
            &[(2, 3), (2, 5)],
            "x = <<-A\n#{b}\n  A\n",
        ),
        (
            "x = \"#{ \"#{ b }\" }\"\n",
            &[(1, 8), (1, 12), (1, 14), (1, 17)],
            "x = \"#{\"#{b}\"}\"\n",
        ),
        ("x = \"a\" \"#{ b}\"\n", &[(1, 12)], "x = \"a\" \"#{b}\"\n"),
        (
            "x = \"#{ [1].map { _1 } }\"\n",
            &[(1, 8), (1, 23)],
            "x = \"#{[1].map { _1 }}\"\n",
        ),
        (
            "x = \"#{ -1 }#{ ..1 }\"\n",
            &[(1, 8), (1, 11), (1, 15), (1, 19)],
            "x = \"#{-1}#{..1}\"\n",
        ),
        ("x = \"é#{ b }\"\n", &[(1, 9), (1, 11)], "x = \"é#{b}\"\n"),
        (
            "\u{feff}x = \"#{ b }\"\n",
            &[(1, 8), (1, 10)],
            "\u{feff}x = \"#{b}\"\n",
        ),
        ("x = \"#{ b }\"\r\n", &[(1, 8), (1, 10)], "x = \"#{b}\"\r\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        if !expected.is_empty() {
            assert_correction(&SpaceInsideStringInterpolation, source, after);
        }
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = \"#{b}\"\n",
        "x = \"#{ }\"\n",
        "x = \"#{\t}\"\n",
        "x = \"#{  }\"\n",
        "x = \"#@a #$b #@@c\"\n",
        "x = ' #{ b } '\n",
        "x = %q( #{ b } )\n",
        "x = %w( #{ b } )\n",
        "x = %i(#{ b })\n",
        "x = \"#{\n  b }\"\n",
        "x = \"#{ b\n}\"\n",
        "x = \"#{ b \\\n}\"\n",
        "x = \"\\#{ b }\"\n",
        "# \"#{ b }\"\n",
        "x = <<~'A'\n  #{ b }\nA\n",
        "x = \"#{\x0bb\x0c}\"\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn a_statement_separator_alone_is_not_empty() {
    assert_eq!(findings("x = \"#{ ; }\"\n"), [(1, 8), (1, 10)]);
    assert_correction(
        &SpaceInsideStringInterpolation,
        "x = \"#{ ; }\"\n",
        "x = \"#{;}\"\n",
    );
}

#[test]
fn blanks_inside_the_interpolated_code_are_not_layout() {
    // `?\ ` is a character literal holding a space and `% x ` a string with
    // space delimiters: their blanks are code, adjacent to `}` as upstream's
    // tokens are, and removing them would change the value.
    for (source, expected, after) in [
        ("p \"#{?\\ }}\"\n", &[][..], ""),
        ("p \"#{?\\ }\"\n", &[], ""),
        ("p \"#{ ?\\ }\"\n", &[(1, 6)], "p \"#{?\\ }\"\n"),
        ("p \"#{% x }\"\n", &[], ""),
        ("p \"#{ % x  }\"\n", &[(1, 6), (1, 11)], "p \"#{% x }\"\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        if !expected.is_empty() {
            assert_correction(&SpaceInsideStringInterpolation, source, after);
        }
    }
}

#[test]
fn listed_observers_hold_every_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:p).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\nx",
    ] {
        held(&format!("x = \"#{{ 1 }}\"\n{observer}\n"));
    }
    assert_correction(
        &SpaceInsideStringInterpolation,
        "x = \"#{ 1 }\"\np File.read('example.rb')\n",
        "x = \"#{1}\"\np File.read('example.rb')\n",
    );
}
