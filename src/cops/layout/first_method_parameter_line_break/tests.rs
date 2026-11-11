//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/first_method_parameter_line_break_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop is
//! opt-in; all 11 pinned examples run at the fixed
//! `AllowMultilineFinalElement: false`, so the `AllowMultilineFinalElement:
//! true` context is replayed at that default and its sources are reported.
//! Findings, messages and single-pass corrected bytes equal the fixed replay
//! recorded in the pinned RuboCop reference run (each source and
//! corrected SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/FirstMethodParameterLineBreak";

/// `(line, col)` of each finding, checking message and severity.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&FirstMethodParameterLineBreak], source)
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
    let offenses = correction_offenses(&FirstMethodParameterLineBreak, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&FirstMethodParameterLineBreak, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source is accepted.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (
        5,
        "def foo(bar,\n  baz)\n  do_something\nend\n",
        &[(1, 9)],
        "def foo(\nbar,\n  baz)\n  do_something\nend\n",
    ),
    (
        23,
        "def self.foo(bar,\n  baz)\n  do_something\nend\n",
        &[(1, 14)],
        "def self.foo(\nbar,\n  baz)\n  do_something\nend\n",
    ),
    (
        41,
        "def foo(bar, baz, bing)\n  do_something\nend\n",
        &[],
        "",
    ),
    (49, "def foo bar,\n  baz\n  do_something\nend\n", &[], ""),
    (58, "def foo(bar, baz) ; bing ; end", &[], ""),
    (62, "def foo\n  bing\nend\n", &[], ""),
    (
        70,
        "def foo(bar = [],\n  baz = 2)\n  do_something\nend\n",
        &[(1, 9)],
        "def foo(\nbar = [],\n  baz = 2)\n  do_something\nend\n",
    ),
    // `AllowMultilineFinalElement: true`, replayed at the fixed `false`.
    (
        91,
        "def foo(bar, baz = {\n  a: b\n})\n  do_something\nend\n",
        &[(1, 9)],
        "def foo(\nbar, baz = {\n  a: b\n})\n  do_something\nend\n",
    ),
    (
        101,
        "def foo(bar = {\n  a: b\n})\n  do_something\nend\n",
        &[(1, 9)],
        "def foo(\nbar = {\n  a: b\n})\n  do_something\nend\n",
    ),
    (
        111,
        "def foo(bar, baz = {\n  a: b\n}, qux = false)\n  do_something\nend\n",
        &[(1, 9)],
        "def foo(\nbar, baz = {\n  a: b\n}, qux = false)\n  do_something\nend\n",
    ),
    (
        131,
        "def foo(bar, baz,\n  qux = false)\n  do_something\nend\n",
        &[(1, 9)],
        "def foo(\nbar, baz,\n  qux = false)\n  do_something\nend\n",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 11);
    for &(line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&FirstMethodParameterLineBreak, source).is_empty(),
                "spec line {line}"
            );
        } else {
            assert_correction(&FirstMethodParameterLineBreak, source, after);
        }
    }
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(FirstMethodParameterLineBreak.name(), COP);
    assert!(!FirstMethodParameterLineBreak.default_enabled());
    assert!(!FirstMethodParameterLineBreak.department_selection_enables());
    assert!(FirstMethodParameterLineBreak.safe_fixes());
    // Upstream's pass-ordering incompatibility with the default-on
    // Style/MethodDefParentheses is not declared (it would withhold every
    // edit in the default selection); the two never edit the same list.
    assert!(
        FirstMethodParameterLineBreak
            .autocorrect_incompatible_with()
            .is_empty()
    );
    assert!(
        crate::pipeline::lint_str("def foo(bar,\n  baz)\nend\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_parameter_kind_and_definition_is_checked() {
    for (source, expected) in [
        ("def f(a,\n  b); end\n", (1, 7)),
        ("def f(a = 1,\n  b); end\n", (1, 7)),
        ("def f(*a,\n  b); end\n", (1, 7)),
        ("def f(*,\n  b:); end\n", (1, 7)),
        ("def f(a:,\n  b: 1); end\n", (1, 7)),
        ("def f(a: 1,\n  **b); end\n", (1, 7)),
        ("def f(**a,\n  &b); end\n", (1, 7)),
        ("def f((a, b),\n  c); end\n", (1, 7)),
        ("def f(a,\n  ...); end\n", (1, 7)),
        ("def f(a,\n  **nil); end\n", (1, 7)),
        ("def f( a,\n  b); end\n", (1, 8)),
        ("def f (a,\n  b); end\n", (1, 8)),
        ("def self.f(a,\n  b); end\n", (1, 12)),
        ("def A.f(a,\n  b); end\n", (1, 9)),
        ("private def f(a,\n  b); end\n", (1, 15)),
        ("def f(a,\n  b) = a\n", (1, 7)),
        ("def é(a,\n  b); end\n", (1, 7)),
        ("class A\n  def f(a,\n    b); end\nend\n", (2, 9)),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
    }
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "def f; end\n",
        "def f(); end\n",
        "def f(a, b); end\n",
        "def f(\n  a,\n  b); end\n",
        "def f( # c\n  a, b); end\n",
        "def f(a, b)\n  c\nend\n",
        // Without parentheses.
        "def f a,\n  b\nend\n",
        "def self.f a,\n  b\nend\n",
        // Blocks and lambdas have no `def`.
        "foo { |a,\n  b| }\n",
        "->(a,\n  b) {}\n",
        // Calls are another cop's.
        "foo(a,\n  b)\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        ("def f(a, # c\n  b); end\n", "def f(\na, # c\n  b); end\n"),
        // Upstream inserts `\n`, also in CRLF files.
        ("def f(a,\r\n  b); end\r\n", "def f(\na,\r\n  b); end\r\n"),
        (
            "\u{feff}def f(a,\n  b); end\n",
            "\u{feff}def f(\na,\n  b); end\n",
        ),
        ("def f (a,\n  b); end\n", "def f (\na,\n  b); end\n"),
        ("def f(*,\n  **); end\n", "def f(\n*,\n  **); end\n"),
        ("def f(a,\n  b) = a\n", "def f(\na,\n  b) = a\n"),
        ("def f((a, b),\n  c); end\n", "def f(\n(a, b),\n  c); end\n"),
        // A definition in a default value: both insertions in one pass.
        (
            "def f(a = def g(b,\n  c); end,\n  d); end\n",
            "def f(\na = def g(\nb,\n  c); end,\n  d); end\n",
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

#[test]
fn listed_observers_line_numbers_and_heredoc_bodies_hold_the_edit() {
    for observer in [
        "__FILE__",
        "__dir__",
        "$0",
        "$PROGRAM_NAME",
        "caller",
        "caller_locations",
        "method(:a).source_location",
        "SCRIPT_LINES__",
    ] {
        held(&format!("y = {observer}\ndef f(a,\n  b); end\n"));
    }
    held("def f(a,\n  b); end\nDATA.read\n__END__\nz\n");
    held("def f(a,\n  b); end\np __LINE__\n");
    // An earlier heredoc opener on the line would take the inserted break.
    held("x = <<~A; def f(a,\n  body\nA\n  b); end\n");
    // An opener after the insertion point moves with its line.
    assert_eq!(
        fixed("def f(a = <<~A, b = 1,\n  body\nA\n  c); end\n"),
        "def f(\na = <<~A, b = 1,\n  body\nA\n  c); end\n"
    );
    assert_eq!(
        fixed("y = 1\ndef f(a,\n  b); end\n"),
        "y = 1\ndef f(\na,\n  b); end\n"
    );
}

#[test]
fn findings_only_without_requested_corrections() {
    let offenses = lint_str_with(&[&FirstMethodParameterLineBreak], "def f(a,\n  b); end\n");
    assert_eq!(offenses.len(), 1);
    assert!(offenses[0].correction.is_none());
}
