//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/line_continuation_spacing_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 32 pinned
//! examples, the `EnforcedStyle: no_space` context included, are replayed at
//! the fixed `EnforcedStyle: space`; findings, messages and single-pass
//! corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/LineContinuationSpacing";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str_with(&[&LineContinuationSpacing], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let found = correction_offenses(&LineContinuationSpacing, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

pub(super) const PINNED: &[Pinned] = &[
    // EnforcedStyle: space
    (
        8,
        "if 2 + 2\\\n  == 4\n  foo\nend\n",
        &[(1, 9)],
        "if 2 + 2 \\\n  == 4\n  foo\nend\n",
    ),
    (
        25,
        "if 2 + 2  \\\n  == 4\n  foo\nend\n",
        &[(1, 9)],
        "if 2 + 2 \\\n  == 4\n  foo\nend\n",
    ),
    (
        42,
        "foo\nbar\nbaz\nif 2 + 2    \\\n  == 4\n  foo\nend\n",
        &[(4, 9)],
        "foo\nbar\nbaz\nif 2 + 2 \\\n  == 4\n  foo\nend\n",
    ),
    (65, "[\n  :foo  \\\n]\n", &[(2, 7)], "[\n  :foo \\\n]\n"),
    (80, "if 2 + 2 \\\n  == 4\n  foo\nend\n", &[], ""),
    (89, "# this\\\n<<-X\n  is\\\n  ok\nX\n", &[], ""),
    (99, "%i[\n  foo  \\\n]\n", &[], ""),
    (107, "write($\\) if $\\\nnil\n", &[], ""),
    (114, "foo\nbar\n__END__\nbaz    \\\nqux\n", &[], ""),
    (124, "", &[], ""),
    (128, "'foo\\\nbar'\n", &[], ""),
    (135, "%q{foo\\\nbar}\n", &[], ""),
    (142, "/foo\\\nbar/\n", &[], ""),
    (149, "%r{foo\\\nbar}\n", &[], ""),
    (156, "`foo\\\nbar`\n", &[], ""),
    (163, "%x{foo\\\nbar}\n", &[], ""),
    // EnforcedStyle: no_space, replayed at the fixed space style
    (174, "if 2 + 2 \\\n  == 4\n  foo\nend\n", &[], ""),
    (
        191,
        "if 2 + 2  \\\n  == 4\n  foo\nend\n",
        &[(1, 9)],
        "if 2 + 2 \\\n  == 4\n  foo\nend\n",
    ),
    (
        208,
        "foo\nbar\nbaz\nif 2 + 2    \\\n  == 4\n  foo\nend\n",
        &[(4, 9)],
        "foo\nbar\nbaz\nif 2 + 2 \\\n  == 4\n  foo\nend\n",
    ),
    (
        231,
        "if 2 + 2\\\n  == 4\n  foo\nend\n",
        &[(1, 9)],
        "if 2 + 2 \\\n  == 4\n  foo\nend\n",
    ),
    (240, "[\n  :foo  \\\n]\n", &[(2, 7)], "[\n  :foo \\\n]\n"),
    (255, "# this \\\n<<-X\n  is  \\\n  ok\nX\n", &[], ""),
    (265, "%i[\n  foo  \\\n]\n", &[], ""),
    (273, "foo\nbar\n__END__\nbaz    \\\nqux\n", &[], ""),
    (283, "", &[], ""),
    (287, "'foo     \\\nbar'\n", &[], ""),
    (294, "%q{foo     \\\nbar}\n", &[], ""),
    (301, "\"#{foo}     \\\nbar\"\n", &[], ""),
    (308, "/foo     \\\nbar/\n", &[], ""),
    (315, "%r{foo     \\\nbar}\n", &[], ""),
    (322, "`foo     \\\nbar`\n", &[], ""),
    (329, "%x{foo     \\\nbar}\n", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 32);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&LineContinuationSpacing, source).is_empty());
        } else {
            assert_correction(&LineContinuationSpacing, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    assert!(LineContinuationSpacing.default_enabled());
    assert!(LineContinuationSpacing.department_selection_enables());
    assert!(LineContinuationSpacing.safe_fixes());
    assert!(
        LineContinuationSpacing
            .autocorrect_incompatible_with()
            .is_empty()
    );
}

#[test]
fn missing_and_extra_blanks_before_code_continuations() {
    for (source, expected, after) in [
        ("x = 1\\\n  + 2\n", (1, 6), "x = 1 \\\n  + 2\n"),
        ("x = 1\t\t\\\n  + 2\n", (1, 6), "x = 1 \\\n  + 2\n"),
        ("x = 1 \t \\\n  + 2\n", (1, 6), "x = 1 \\\n  + 2\n"),
        ("foo(a,\\\n  b)\n", (1, 7), "foo(a, \\\n  b)\n"),
        ("foo(a)\\\n  .b\n", (1, 7), "foo(a) \\\n  .b\n"),
        ("x = a[1]\\\n  .b\n", (1, 9), "x = a[1] \\\n  .b\n"),
        ("x = {}\\\n  .b\n", (1, 7), "x = {} \\\n  .b\n"),
        ("x = 'a'\\\n  'b'\n", (1, 8), "x = 'a' \\\n  'b'\n"),
        ("x = \"a\"\\\n  \"b\"\n", (1, 8), "x = \"a\" \\\n  \"b\"\n"),
        ("x = `a`\\\n  .b\n", (1, 8), "x = `a` \\\n  .b\n"),
        ("x = %w[a]\\\n  .b\n", (1, 10), "x = %w[a] \\\n  .b\n"),
        ("x =\\\n  1\n", (1, 4), "x = \\\n  1\n"),
        ("x = a ||\\\n  b\n", (1, 9), "x = a || \\\n  b\n"),
        ("é = 1\\\n  + 2\n", (1, 6), "é = 1 \\\n  + 2\n"),
        (
            "\u{feff}x = 1\\\n  + 2\n",
            (1, 6),
            "\u{feff}x = 1 \\\n  + 2\n",
        ),
        // A blank-only line before the backslash.
        ("foo(1,\n    \\\n  2)\n", (2, 1), "foo(1,\n \\\n  2)\n"),
        // Interpolation code is not literal text.
        ("/#{a\\\n}/\n", (1, 5), "/#{a \\\n}/\n"),
    ] {
        assert_eq!(findings(source), [expected], "{source:?}");
        assert_correction(&LineContinuationSpacing, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = 1 \\\n  + 2\n",
        // Inside literals, heredoc bodies and comments.
        "x = \"a  \\\nb\"\n",
        "x = \"#{a}\\\nb\"\n",
        "x = ?\\\\\n",
        "x = <<~A\n  a  \\\n  b\nA\n",
        "x = <<~A\n  #{a}\\\n  b\nA\n",
        "x = <<~`A`\n  a  \\\n  b\nA\n",
        "x = %W[a  \\\nb]\n",
        "x = %I[a\\\nb]\n",
        "x = %w(a b)  # c  \\\n",
        "=begin\na  \\\n=end\n",
        "if /a  \\\nb/\nend\n",
        "x = /#{a}  \\\nb/\n",
        "x = `#{a}  \\\nb`\n",
        "p $\\\n",
        // CRLF: upstream's `\\$` never matches before `\r\n`.
        "x = 1\\\r\n  + 2\r\n",
        "x = 1  \\\r\n  + 2\r\n",
        // A backslash that is not at the end of its line.
        "x = '\\\\' if a\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn symbols_are_reported_as_upstream_but_never_edited() {
    // Upstream ignores only `str`/`dstr`, arrays, comments and `$\`: a
    // backslash-newline inside a symbol is reported. It is an escape there,
    // and the blanks are part of the symbol, so the edit is withheld.
    for (source, at) in [
        ("x = :\"a  \\\nb\"\n", (1, 8)),
        ("x = :\"a\\\nb\"\n", (1, 8)),
        ("x = :\"#{a}  \\\nb\"\n", (1, 11)),
        ("x = { \"a  \\\nb\": 1 }\n", (1, 9)),
        ("x = %s(a  \\\nb)\n", (1, 9)),
    ] {
        assert_eq!(findings(source), [at], "{source:?}");
        held(source);
    }
}

#[test]
fn space_sensitive_insertions_are_withheld() {
    // `foo -\` passes `-1`; `foo - \` subtracts: the byte before a missing
    // blank decides whether one can be inserted.
    for source in [
        "foo -\\\n1\n",
        "foo +\\\n1\n",
        "foo *\\\na\n",
        "foo &\\\nb\n",
        "x = a &&\\\n  b\n",
        "foo ::\\\nA\n",
        "x = a.\\\n  b\n",
        "x = defined?\\\n  (a)\n",
        "x = !\\\n  a\n",
        "foo(\\\n  1)\n",
        "\\\nfoo\n",
    ] {
        held(source);
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
        "p method(:f).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\nx",
    ] {
        held(&format!("x = 1\\\n  + 2\n{observer}\n"));
        held(&format!("x = 1  \\\n  + 2\n{observer}\n"));
    }
    // Out-of-scope readers do not hold the edit.
    assert_correction(
        &LineContinuationSpacing,
        "x = 1\\\n  + 2\np File.read('example.rb')\n",
        "x = 1 \\\n  + 2\np File.read('example.rb')\n",
    );
}

#[test]
fn every_offense_of_a_file_is_corrected_in_one_pass() {
    let source = "x = 1\\\n  + 2  \\\n  + 3\\\n  + 4\n";
    assert_eq!(findings(source), [(1, 6), (2, 6), (3, 6)]);
    assert_correction(
        &LineContinuationSpacing,
        source,
        "x = 1 \\\n  + 2 \\\n  + 3 \\\n  + 4\n",
    );
}

#[test]
fn data_after_the_last_token_line_is_not_checked() {
    assert_eq!(findings("x = 1\n__END__\na\\\nb  \\\n"), []);
    assert_eq!(findings("x = 1\\\n  + 2\n__END__\na\\\n"), [(1, 6)]);
    // A comment is a token: the lines up to it are checked.
    assert_eq!(findings("x = 1\n# c\n"), []);
}
