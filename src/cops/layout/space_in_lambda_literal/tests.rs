//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_in_lambda_literal_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 15 pinned examples
//! run at the fixed `EnforcedStyle: require_no_space`, the `require_space`
//! context's included; findings, messages and single-pass corrected bytes
//! equal the fixed replay recorded in the pinned RuboCop reference run
//! (each source and corrected SHA-256 was checked against it when the rows
//! were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceInLambdaLiteral";

fn findings(source: &str) -> Vec<(u32, u32, &'static str)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceInLambdaLiteral], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col, MSG)
        })
        .collect();
    found.sort_unstable();
    found
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceInLambdaLiteral, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (
    u32,
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (8, "a = ->(b, c) { b + c }\n", &[], ""),
    (
        19,
        "a = -> (b, c) { b + c }",
        &[(1, 7, MSG)],
        "a = ->(b, c) { b + c }",
    ),
    (
        23,
        "l = lambda do |a, b|\n  tmp = a * 7\n  tmp * b / 50\nend\n",
        &[],
        "",
    ),
    (32, "a = ->{ b + c }", &[], ""),
    (
        36,
        "a = -> (b = ->(c) {}, d) { b + d }\n",
        &[(1, 7, MSG)],
        "a = ->(b = ->(c) {}, d) { b + d }\n",
    ),
    (
        47,
        "a = ->(b = -> (c) {}, d) { b + d }\n",
        &[(1, 14, MSG)],
        "a = ->(b = ->(c) {}, d) { b + d }\n",
    ),
    (58, "a = ->(b = ->(c) {}, d) { b + d }\n", &[], ""),
    (
        74,
        "a = -> (b, c) { b + c }\n",
        &[(1, 7, MSG)],
        "a = ->(b, c) { b + c }\n",
    ),
    (85, "a = ->(b, c) { b + c }", &[], ""),
    (
        89,
        "l = lambda do |a, b|\n  tmp = a * 7\n  tmp * b / 50\nend\n",
        &[],
        "",
    ),
    (98, "a = -> { b + c }", &[], ""),
    (
        102,
        "a = ->   (b, c) { b + c }\n",
        &[(1, 7, MSG)],
        "a = ->(b, c) { b + c }\n",
    ),
    (
        113,
        "a = ->(b = -> (c) {}, d) { b + d }\n",
        &[(1, 14, MSG)],
        "a = ->(b = ->(c) {}, d) { b + d }\n",
    ),
    (
        124,
        "a = -> (b = ->(c) {}, d) { b + d }\n",
        &[(1, 7, MSG)],
        "a = ->(b = ->(c) {}, d) { b + d }\n",
    ),
    (
        135,
        "a = -> (b = -> (c) {}, d) { b + d }\n",
        &[(1, 7, MSG), (1, 15, MSG)],
        "a = ->(b = ->(c) {}, d) { b + d }\n",
    ),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 15);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceInLambdaLiteral, source).is_empty());
        } else {
            assert_correction(&SpaceInLambdaLiteral, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceInLambdaLiteral;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn every_parameter_list_after_a_gap() {
    for (source, expected, after) in [
        ("f = -> a { a }\n", &[(1, 7, MSG)][..], "f = ->a { a }\n"),
        ("f = -> a, b { a }\n", &[(1, 7, MSG)], "f = ->a, b { a }\n"),
        (
            "f = -> *a, &b { a }\n",
            &[(1, 7, MSG)],
            "f = ->*a, &b { a }\n",
        ),
        ("f = -> **k { k }\n", &[(1, 7, MSG)], "f = ->**k { k }\n"),
        ("f = -> k: 1 { k }\n", &[(1, 7, MSG)], "f = ->k: 1 { k }\n"),
        ("f = -> (;a) { a }\n", &[(1, 7, MSG)], "f = ->(;a) { a }\n"),
        (
            "f = -> (a) do a end\n",
            &[(1, 7, MSG)],
            "f = ->(a) do a end\n",
        ),
        ("f = ->\t (a) {}\n", &[(1, 7, MSG)], "f = ->(a) {}\n"),
        (
            "f = -> ((a, b)) {}\n",
            &[(1, 7, MSG)],
            "f = ->((a, b)) {}\n",
        ),
        (
            "p(-> (a) {}, -> (b) {})\n",
            &[(1, 5, MSG), (1, 16, MSG)],
            "p(->(a) {}, ->(b) {})\n",
        ),
        (
            "x = é; f = -> (a) {}\n",
            &[(1, 14, MSG)],
            "x = é; f = ->(a) {}\n",
        ),
        (
            "\u{feff}f = -> (a) {}\n",
            &[(1, 7, MSG)],
            "\u{feff}f = ->(a) {}\n",
        ),
        ("f = -> (a) {}\r\n", &[(1, 7, MSG)], "f = ->(a) {}\r\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        assert_correction(&SpaceInLambdaLiteral, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "f = ->(a) { a }\n",
        "f = ->a { a }\n",
        "f = -> () { 1 }\n",
        "f = -> { 1 }\n",
        "f = -> do 1 end\n",
        "f = -> { _1 }\n",
        "f = -> { it }\n",
        "f = lambda { |a| a }\n",
        "f = lambda do |a| a end\n",
        "f = proc { |a| a }\n",
        "x = \"-> (a)\"\n",
        "# -> (a) {}\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn a_line_continuation_keeps_the_finding_without_an_edit() {
    let source = "f = -> \\\n  (a) { a }\np __LINE__\n";
    assert_eq!(findings(source), [(1, 7, MSG)]);
    held(source);
    let found = lint_str_with(&[&SpaceInLambdaLiteral], source);
    assert_eq!((found[0].line, found[0].end_line), (1, 2));
}
