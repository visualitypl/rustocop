//! Examples ported from RuboCop's `spec/rubocop/cop/layout/empty_line_after_multiline_condition_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop
//! is opt-in (`Enabled: false` upstream); all 31 pinned examples run with it
//! enabled at its only defaults. Findings, messages and single-pass corrected
//! bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run.
use super::*;
use crate::{cops::testing::assert_correction, pipeline::lint_str_with};

const COP: &str = "Layout/EmptyLineAfterMultilineCondition";
const MESSAGE: &str = "Use empty line after multiline condition.";

fn positions(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        ruby_prism::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&EmptyLineAfterMultilineCondition], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MESSAGE);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            (o.line, o.col)
        })
        .collect();
    found.sort_unstable();
    found
}

fn check(source: &str, finding: &[(u32, u32)], after: &str) {
    assert_eq!(positions(source), finding, "{source}");
    if !finding.is_empty() {
        assert_correction(&EmptyLineAfterMultilineCondition, source, after);
    }
}

/// `(spec line, source, fixed-default finding, single-pass correction)`.
type Pinned = (u32, &'static str, Option<(u32, u32)>, &'static str);

const PINNED: &[Pinned] = &[
    (
        5,
        "if multiline &&\n   condition\n  do_something\nend\n",
        Some((1, 4)),
        "if multiline &&\n   condition\n\n  do_something\nend\n",
    ),
    (
        23,
        "if multiline &&\n   condition\n\n  do_something\nend\n",
        None,
        "",
    ),
    (33, "if singleline\n  do_something\nend\n", None, ""),
    (
        41,
        "do_something if multiline &&\n                condition\ndo_something_else\n",
        Some((1, 17)),
        "do_something if multiline &&\n                condition\n\ndo_something_else\n",
    ),
    (
        57,
        "do_something if multiline &&\n                condition\n\ndo_something_else\n",
        None,
        "",
    ),
    (
        67,
        "def m\n  do_something if multiline &&\n                condition\nend\n",
        None,
        "",
    ),
    (
        76,
        "def m\n  do_something if multiline &&\n                  condition\nrescue\n  handle_error\nend\n",
        None,
        "",
    ),
    (
        87,
        "begin\n  do_something while multiline &&\n                     condition\nrescue\n  handle_error\nend\n",
        None,
        "",
    ),
    (
        98,
        "def m\n  do_something if multiline &&\n                  condition\nensure\n  do_something_else\nend\n",
        None,
        "",
    ),
    (109, "do_something if condition\n", None, ""),
    (
        115,
        "if condition\n  do_something\nelsif multiline &&\n   condition\n  do_something_else\nend\n",
        Some((3, 7)),
        "if condition\n  do_something\nelsif multiline &&\n   condition\n\n  do_something_else\nend\n",
    ),
    (
        137,
        "if condition\n  do_something\nelsif multiline &&\n   condition\n\n  do_something_else\nend\n",
        None,
        "",
    ),
    (
        149,
        "while multiline &&\n   condition\n  do_something\nend\n",
        Some((1, 7)),
        "while multiline &&\n   condition\n\n  do_something\nend\n",
    ),
    (
        167,
        "until multiline &&\n   condition\n  do_something\nend\n",
        Some((1, 7)),
        "until multiline &&\n   condition\n\n  do_something\nend\n",
    ),
    (
        185,
        "while multiline &&\n   condition\n\n  do_something\nend\n",
        None,
        "",
    ),
    (195, "while singleline\n  do_something\nend\n", None, ""),
    (
        203,
        "begin\n  do_something\nend while multiline &&\n      condition\ndo_something_else\n",
        Some((3, 11)),
        "begin\n  do_something\nend while multiline &&\n      condition\n\ndo_something_else\n",
    ),
    (
        223,
        "begin\n  do_something\nend while multiline &&\n      condition\n\ndo_something_else\n",
        None,
        "",
    ),
    (
        235,
        "def m\n  begin\n    do_something\n  end while multiline &&\n        condition\nend\n",
        None,
        "",
    ),
    (
        246,
        "do_something while multiline &&\n                   condition\ndo_something_else\n",
        Some((1, 20)),
        "do_something while multiline &&\n                   condition\n\ndo_something_else\n",
    ),
    (
        262,
        "def m\n  do_something while multiline &&\n                     condition\nend\n",
        None,
        "",
    ),
    (
        271,
        "case x\nwhen foo,\n    bar\n  do_something\nend\n",
        Some((2, 1)),
        "case x\nwhen foo,\n    bar\n\n  do_something\nend\n",
    ),
    (
        291,
        "case x\nwhen foo,\n    bar\n\n  do_something\nend\n",
        None,
        "",
    ),
    (
        302,
        "case x\nwhen foo,\n    bar then do_something\nend\n",
        None,
        "",
    ),
    (311, "if foo &&\n   bar then do_something\nend\n", None, ""),
    (
        319,
        "while foo &&\n      bar do do_something\nend\n",
        None,
        "",
    ),
    (
        327,
        "begin\n  do_something\nrescue FooError,\n  BarError then handle_error\nend\n",
        None,
        "",
    ),
    (
        337,
        "case x\nwhen foo, bar\n  do_something\nend\n",
        None,
        "",
    ),
    (
        346,
        "begin\n  do_something\nrescue FooError,\n  BarError\n  handle_error\nend\n",
        Some((3, 1)),
        "begin\n  do_something\nrescue FooError,\n  BarError\n\n  handle_error\nend\n",
    ),
    (
        368,
        "begin\n  do_something\nrescue FooError,\n  BarError\n\n  handle_error\nend\n",
        None,
        "",
    ),
    (
        380,
        "begin\n  do_something\nrescue FooError\n  handle_error\nend\n",
        None,
        "",
    ),
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
            assert_correction(&EmptyLineAfterMultilineCondition, source, after);
        }
    }
}

#[test]
fn condition_shapes() {
    for (source, finding, after) in [
        // `unless`, a parenthesized condition, an empty body.
        (
            "unless a &&\n  b\n  c\nend\n",
            &[(1, 8)][..],
            "unless a &&\n  b\n\n  c\nend\n",
        ),
        (
            "if (a &&\n    b)\n  c\nend\n",
            &[(1, 4)],
            "if (a &&\n    b)\n\n  c\nend\n",
        ),
        ("if a &&\n   b\nend\n", &[(1, 4)], "if a &&\n   b\n\nend\n"),
        // A ternary is never checked.
        ("x = a &&\n  b ? 1 : 2\ny\n", &[], ""),
        // A modifier used as a value is not followed by a statement.
        ("x = (y if a &&\n  b)\nz\n", &[], ""),
        // `begin ... end until` and modifier `until`.
        (
            "begin\n  x\nend until a ||\n  b\ny\n",
            &[(3, 11)],
            "begin\n  x\nend until a ||\n  b\n\ny\n",
        ),
        (
            "x until a ||\n  b\ny\n",
            &[(1, 9)],
            "x until a ||\n  b\n\ny\n",
        ),
        // Every multi-line `when` and `rescue` clause, single-line ones not.
        (
            "case x\nwhen a,\n  b\n  1\nwhen c\n  2\nwhen d,\n  e\n  3\nend\n",
            &[(2, 1), (7, 1)],
            "case x\nwhen a,\n  b\n\n  1\nwhen c\n  2\nwhen d,\n  e\n\n  3\nend\n",
        ),
        (
            "begin\n  x\nrescue A,\n  B => e\n  1\nrescue C\n  2\nrescue D,\n  E\n  3\nend\n",
            &[(3, 1), (8, 1)],
            "begin\n  x\nrescue A,\n  B => e\n\n  1\nrescue C\n  2\nrescue D,\n  E\n\n  3\nend\n",
        ),
        // One multi-line exception is not a multi-line list; upstream's
        // `when` check compares the first and last condition lines only.
        ("begin\n  x\nrescue A::\n  B\n  1\nend\n", &[], ""),
        (
            "case x\nwhen foo(\n  1)\n  2\nend\n",
            &[(2, 1)],
            "case x\nwhen foo(\n  1)\n\n  2\nend\n",
        ),
        // `case ... in` has no `when` branches.
        ("case x\nin [a,\n  b]\n  1\nend\n", &[], ""),
        // CRLF keeps its line ending; whitespace-only lines are empty.
        (
            "if a &&\r\n   b\r\n  c\r\nend\r\n",
            &[(1, 4)],
            "if a &&\r\n   b\r\n\r\n  c\r\nend\r\n",
        ),
        ("if a &&\n   b\n  \t\n  c\nend\n", &[], ""),
    ] {
        check(source, finding, after);
    }
}

#[test]
fn metadata_is_opt_in_and_never_selected_by_department() {
    let cop = EmptyLineAfterMultilineCondition;
    assert!(!cop.default_enabled() && cop.safe_fixes());
    assert!(!cop.department_selection_enables());
    assert_eq!(cop.severity(), crate::offense::Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
    // Not part of the default selection.
    assert!(
        crate::pipeline::lint_str("if a &&\n  b\n  c\nend\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}
