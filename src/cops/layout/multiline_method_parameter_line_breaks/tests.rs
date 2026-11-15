//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_method_parameter_line_breaks_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop
//! is opt-in; all 14 pinned examples run at the fixed
//! `AllowMultilineFinalElement: false`. The `ignore last element` context
//! configures `true`; it is replayed at the fixed `false`, so its multi-line
//! final parameters are reported too. Findings, messages and single-pass
//! corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected SHA-256
//! was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/MultilineMethodParameterLineBreaks";

/// `(line, col)` of each finding, checking message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineMethodParameterLineBreaks], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col)
        })
        .collect();
    found.sort();
    found
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&MultilineMethodParameterLineBreaks, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source is accepted.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (6, "def taz(abc)\nend\n", &[], ""),
    (15, "def taz\nend\n", &[], ""),
    (24, "def taz(\n  foo, bar\n)\nend\n", &[], ""),
    (
        35,
        "def taz(abc,\nfoo, bar,\nbaz\n)\nend\n",
        &[(2, 6)],
        "def taz(abc,\nfoo, \nbar,\nbaz\n)\nend\n",
    ),
    (
        57,
        "def taz(abc,\nfoo, bar, barz,\nbaz\n)\nend\n",
        &[(2, 6), (2, 11)],
        "def taz(abc,\nfoo, \nbar, \nbarz,\nbaz\n)\nend\n",
    ),
    (
        81,
        "def taz(abc,\nfoo, bar, z: \"barz\",\nx:\n)\nend\n",
        &[(2, 6), (2, 11)],
        "def taz(abc,\nfoo, \nbar, \nz: \"barz\",\nx:\n)\nend\n",
    ),
    (
        105,
        "def taz(abc, foo = {\n  foo: \"edf\",\n})\nend\n",
        &[(1, 14)],
        "def taz(abc, \nfoo = {\n  foo: \"edf\",\n})\nend\n",
    ),
    (
        125,
        "def taz(abc = {\n  foo: \"edf\",\n}, bar:)\nend\n",
        &[(3, 4)],
        "def taz(abc = {\n  foo: \"edf\",\n}, \nbar:)\nend\n",
    ),
    (
        145,
        "def do_something(foo, bar, baz,\n  quux)\nend\n",
        &[(1, 23), (1, 28)],
        "def do_something(foo, \nbar, \nbaz,\n  quux)\nend\n",
    ),
    // `AllowMultilineFinalElement: true`, replayed at the fixed `false`.
    (
        167,
        "def foo(abc, foo, bar = {\n  a: 1,\n})\nend\n",
        &[(1, 14), (1, 19)],
        "def foo(abc, \nfoo, \nbar = {\n  a: 1,\n})\nend\n",
    ),
    (
        176,
        "def foo(abc, foo, bar = {\n  a: 1,\n}, buz)\nend\n",
        &[(1, 14), (1, 19)],
        "def foo(abc, \nfoo, \nbar = {\n  a: 1,\n}, buz)\nend\n",
    ),
    (
        197,
        "def foo(abc, foo, ghi,\njkl)\nend\n",
        &[(1, 14), (1, 19)],
        "def foo(abc, \nfoo, \nghi,\njkl)\nend\n",
    ),
    (218, "def self.taz(\n  foo, bar\n)\nend\n", &[], ""),
    (
        229,
        "def self.taz(abc,\nfoo, bar,\nbaz\n)\nend\n",
        &[(2, 6)],
        "def self.taz(abc,\nfoo, \nbar,\nbaz\n)\nend\n",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 14);
    for &(line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineMethodParameterLineBreaks, source).is_empty(),
                "spec line {line}"
            );
        } else {
            assert_correction(&MultilineMethodParameterLineBreaks, source, after);
        }
    }
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(MultilineMethodParameterLineBreaks.name(), COP);
    assert!(!MultilineMethodParameterLineBreaks.default_enabled());
    assert!(!MultilineMethodParameterLineBreaks.department_selection_enables());
    assert!(MultilineMethodParameterLineBreaks.safe_fixes());
    assert!(
        MultilineMethodParameterLineBreaks
            .autocorrect_incompatible_with()
            .is_empty()
    );
    assert!(
        crate::pipeline::lint_str("def f(a, b,\n  c)\nend\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_parameter_kind_is_checked() {
    for (source, expected) in [
        ("def f(a, b,\n  c); end\n", vec![(1, 10)]),
        ("def f a, b,\n  c\nend\n", vec![(1, 10)]),
        ("def self.f(a, b,\n  c); end\n", vec![(1, 15)]),
        ("def a.f(a, b,\n  c); end\n", vec![(1, 12)]),
        ("def f(a, b,\n  c) = 1\n", vec![(1, 10)]),
        ("def f(a, b = 1,\n  c); end\n", vec![(1, 10)]),
        ("def f(a, *b,\n  c); end\n", vec![(1, 10)]),
        ("def f(a, *,\n  c); end\n", vec![(1, 10)]),
        ("def f(*a, b,\n  c); end\n", vec![(1, 11)]),
        ("def f(a, b:,\n  c: 1); end\n", vec![(1, 10)]),
        ("def f(a, **b,\n  &c); end\n", vec![(1, 10)]),
        ("def f(a, **nil,\n  &c); end\n", vec![(1, 10)]),
        ("def f(a, **,\n  &); end\n", vec![(1, 10)]),
        ("def f(a, ...,\n); end\n", vec![]),
        ("def f(a,\n  b, ...); end\n", vec![(2, 6)]),
        ("def f((a, b), c,\n  d); end\n", vec![(1, 15)]),
        ("def f(a, b, c,\n  d); end\n", vec![(1, 10), (1, 13)]),
        // A multi-line default value: the next parameter on its last line.
        ("def f(a = [\n  1\n], b); end\n", vec![(3, 4)]),
        // Character columns, not bytes.
        ("def f(é, b,\n  c); end\n", vec![(1, 10)]),
    ] {
        assert_eq!(positions(source), expected, "{source}");
    }
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "def f; end\n",
        "def f(); end\n",
        "def f(a, b); end\n",
        "def f(\n  a, b\n); end\n",
        "def f(a,\n  b,\n  c); end\n",
        "def f( # comment\n  a, b); end\n",
        // One parameter, however many lines its default takes.
        "def f(a = [\n  1, 2\n]); end\n",
        // Block and lambda parameters are not a definition's.
        "foo { |a, b,\n  c| }\n",
        "->(a, b,\n  c) {}\n",
        // Calls are another cop's.
        "f(a, b,\n  c)\n",
        // The parameter after a heredoc opened on its line.
        "def f(a,\n  b = <<~A, c = 1)\n  t\nA\nend\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        (
            "def f(a,\n  b, c # c\n); end\n",
            "def f(a,\n  b, \nc # c\n); end\n",
        ),
        // Upstream inserts `\n`, also in CRLF files.
        (
            "def f(a,\r\n  b, c); end\r\n",
            "def f(a,\r\n  b, \nc); end\r\n",
        ),
        (
            "\u{feff}def f(a,\n  b, c); end\n",
            "\u{feff}def f(a,\n  b, \nc); end\n",
        ),
        // Without parentheses the list continues after the comma.
        ("def f a, b,\n  c\nend\n", "def f a, \nb,\n  c\nend\n"),
        (
            "def f(a, *b, c:,\n  **d, &e); end\n",
            "def f(a, \n*b, \nc:,\n  **d, \n&e); end\n",
        ),
        (
            "def f(a, b = 1,\n  c) = a\n",
            "def f(a, \nb = 1,\n  c) = a\n",
        ),
    ] {
        assert_eq!(fixed(source), after, "{source:?}");
        assert_eq!(
            crate::parser::parse(after.as_bytes()).errors().count(),
            0,
            "{after:?}"
        );
    }
}
