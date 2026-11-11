//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/empty_lines_around_exception_handling_keywords_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 28
//! pinned examples run at the fixed defaults (`Layout/EmptyLinesAroundBlockBody`
//! at `no_empty_lines`, so its `empty_lines` context is replayed at the fixed
//! default like the inventory's fixed results); the `:ruby25` and `:ruby27`
//! contexts (pinned at 3.3) apply unchanged at the fixed 3.4. Findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (source and corrected SHA-256 of
//! every row were checked against it when the rows were ported).
use super::*;
use crate::cops::testing::{assert_correction, correction_offenses};

const COP: &str = "Layout/EmptyLinesAroundExceptionHandlingKeywords";

/// `(line, col, message)` of each finding by line, from the full default
/// pipeline.
fn findings(source: &str) -> Vec<(u32, u32, String)> {
    let mut found: Vec<_> = crate::pipeline::lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert_eq!(o.end_line, o.line);
            (o.line, o.col, o.message)
        })
        .collect();
    found.sort();
    found
}

/// `(line, "before"/"after", keyword)`.
type Expected = (u32, &'static str, &'static str);

fn expect(source: &str, expected: &[Expected], after: &str) {
    let mut expected: Vec<_> = expected
        .iter()
        .map(|&(line, side, keyword)| {
            (
                line,
                1,
                format!("Extra empty line detected {side} the `{keyword}`."),
            )
        })
        .collect();
    expected.sort();
    assert_eq!(findings(source), expected, "{source:?}");
    if expected.is_empty() {
        assert!(correction_offenses(&EmptyLinesAroundExceptionHandlingKeywords, source).is_empty());
    } else {
        assert_correction(&EmptyLinesAroundExceptionHandlingKeywords, source, after);
    }
}

/// `(spec line, source, findings, corrected source)`.
type Pinned = (u32, &'static str, &'static [Expected], &'static str);

const COMPLEX_BODY: &str = "\n\n  do_something1\n\nrescue RuntimeError\n\n  do_something2\n\nrescue ArgumentError => ex\n\n  do_something3\n\nrescue\n\n  do_something3\n\nelse\n\n  do_something4\n\nensure\n\n  do_something4\n\nend\n";
const COMPLEX_FIXED: &str = "\n\n  do_something1\nrescue RuntimeError\n  do_something2\nrescue ArgumentError => ex\n  do_something3\nrescue\n  do_something3\nelse\n  do_something4\nensure\n  do_something4\n\nend\n";
const COMPLEX: &[Expected] = &[
    (22, "after", "ensure"),
    (20, "before", "ensure"),
    (18, "after", "else"),
    (16, "before", "else"),
    (6, "after", "rescue"),
    (4, "before", "rescue"),
    (10, "after", "rescue"),
    (8, "before", "rescue"),
    (14, "after", "rescue"),
    (12, "before", "rescue"),
];

const PINNED: &[Pinned] = &[
    (
        13,
        "begin\n  f1\n\nrescue\n  f2\nend\n",
        &[(3, "before", "rescue")],
        "begin\n  f1\nrescue\n  f2\nend\n",
    ),
    (
        33,
        "begin\n  f1\nrescue\n\n  f2\nend\n",
        &[(4, "after", "rescue")],
        "begin\n  f1\nrescue\n  f2\nend\n",
    ),
    (
        53,
        "begin\n  f1\nrescue\n  f2\n\nelse\n  f3\nend\n",
        &[(5, "before", "else")],
        "begin\n  f1\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    (
        77,
        "def foo\n  f1\nrescue\n  f2\n\nelse\n  f3\nend\n",
        &[(5, "before", "else")],
        "def foo\n  f1\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    // `:ruby25` blocks.
    (
        101,
        "foo do\n  f1\n\nrescue\n  f2\nelse\n  f3\nend\n",
        &[(3, "before", "rescue")],
        "foo do\n  f1\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    (
        125,
        "foo do\n  f1\nrescue\n\n  f2\nelse\n  f3\nend\n",
        &[(4, "after", "rescue")],
        "foo do\n  f1\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    (
        149,
        "foo do\n  f1\nrescue\n  f2\n\nelse\n  f3\nend\n",
        &[(5, "before", "else")],
        "foo do\n  f1\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    // `:ruby27` numbered-parameter blocks.
    (
        173,
        "foo do\n  f1(_1)\n\nrescue\n  f2\nelse\n  f3\nend\n",
        &[(3, "before", "rescue")],
        "foo do\n  f1(_1)\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    (
        197,
        "foo do\n  f1(_1)\nrescue\n\n  f2\nelse\n  f3\nend\n",
        &[(4, "after", "rescue")],
        "foo do\n  f1(_1)\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    (
        221,
        "foo do\n  f1(_1)\nrescue\n  f2\n\nelse\n  f3\nend\n",
        &[(5, "before", "else")],
        "foo do\n  f1(_1)\nrescue\n  f2\nelse\n  f3\nend\n",
    ),
    (
        319,
        "do_something do\n  foo\nrescue\n\nend\n",
        &[(4, "after", "rescue")],
        "do_something do\n  foo\nrescue\nend\n",
    ),
    // The shared `accepts` examples.
    (
        8,
        "begin\n  f1\nrescue\n  f2\nelse\n  f3\nensure\n  f4\nend\n",
        &[],
        "",
    ),
    (8, "begin\n\n  f1\n\nend\n", &[], ""),
    (8, "begin\nend\n", &[], ""),
    (8, "def foo\nend\n", &[], ""),
    (8, "begin; foo; rescue => e; end\n", &[], ""),
    (8, "begin\n  foo\nrescue => e; end\n", &[], ""),
    (8, "begin\n  foo\nrescue => x\nrescue => y; end\n", &[], ""),
    (8, "def do_something; foo; rescue => e; end\n", &[], ""),
    (8, "def do_something\nensure end\n", &[], ""),
    (8, "def do_something\nrescue\nelse end\n", &[], ""),
    (8, "def do_something\nfoo\nensure bar end\n", &[], ""),
    (8, "def do_something\nrescue\nelse foo end\n", &[], ""),
    // `Layout/EmptyLinesAroundBlockBody: empty_lines`, replayed at the fixed
    // `no_empty_lines`.
    (
        8,
        "do_something do\n  foo\nrescue\n\nend\n",
        &[(4, "after", "rescue")],
        "do_something do\n  foo\nrescue\nend\n",
    ),
    (
        350,
        "def do_something\n  foo\nensure\n\nend\n",
        &[(4, "after", "ensure")],
        "def do_something\n  foo\nensure\nend\n",
    ),
    (
        368,
        "begin\n  foo\nrescue\n\nend\n",
        &[(4, "after", "rescue")],
        "begin\n  foo\nrescue\nend\n",
    ),
];

/// The pinned rows, with the two complex examples (spec lines 387, 445).
fn pinned_rows() -> Vec<(u32, String, &'static [Expected], String)> {
    let mut rows: Vec<_> = PINNED
        .iter()
        .map(|&(spec, source, expected, after)| (spec, source.into(), expected, after.into()))
        .collect();
    for (spec, header) in [(387, "begin"), (445, "def foo")] {
        rows.push((
            spec,
            format!("{header}{COMPLEX_BODY}"),
            COMPLEX,
            format!("{header}{COMPLEX_FIXED}"),
        ));
    }
    rows
}

#[test]
fn pinned_examples_replay_the_fixed_results() {
    let rows = pinned_rows();
    assert_eq!(rows.len(), 28);
    for (spec, source, expected, after) in rows {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "spec line {spec}"
        );
        expect(&source, expected, &after);
    }
}

#[test]
fn owners_anchor_lines_and_same_line_skips() {
    for (source, expected, after) in [
        // `defs`, lambdas and `super`/safe-navigation blocks.
        (
            "def self.foo\n  a\n\nrescue\n  b\nend\n",
            vec![(3, "before", "rescue")],
            "def self.foo\n  a\nrescue\n  b\nend\n",
        ),
        (
            "f = -> do\n  a\nensure\n\n  b\nend\n",
            vec![(4, "after", "ensure")],
            "f = -> do\n  a\nensure\n  b\nend\n",
        ),
        (
            "def f\n  super do\n    a\n\n  rescue\n    b\n  end\nend\n",
            vec![(4, "before", "rescue")],
            "def f\n  super do\n    a\n  rescue\n    b\n  end\nend\n",
        ),
        (
            "x&.each do\n  a\nrescue\n\n  b\nend\n",
            vec![(4, "after", "rescue")],
            "x&.each do\n  a\nrescue\n  b\nend\n",
        ),
        // A block's line is its call's first line: a keyword there is
        // skipped, one on a later line is checked.
        (
            "foo(\n  1\n) do\n  a\n\nrescue\n\n  b\nend\n",
            vec![(5, "before", "rescue"), (7, "after", "rescue")],
            "foo(\n  1\n) do\n  a\nrescue\n  b\nend\n",
        ),
        (
            "x.\n  each do; a; rescue\n\n  b\nend\n",
            vec![(3, "after", "rescue")],
            "x.\n  each do; a; rescue\n  b\nend\n",
        ),
        ("foo do; a; rescue\n\n  b\nend\n", vec![], ""),
        // A body starting on the `def` line; keywords on the owner's line
        // are skipped.
        (
            "def foo; a\n\nrescue\n\n  b\nensure\n\n  c\nend\n",
            vec![
                (2, "before", "rescue"),
                (4, "after", "rescue"),
                (7, "after", "ensure"),
            ],
            "def foo; a\nrescue\n  b\nensure\n  c\nend\n",
        ),
        ("begin; a; rescue\n\n  b\nend\n", vec![], ""),
        // One blank line between two keywords is one finding: upstream
        // visits `ensure`, then `else`, then each `rescue`, and the first
        // message wins.
        (
            "begin\n  a\nrescue\n\nelse\n  b\nend\n",
            vec![(4, "before", "else")],
            "begin\n  a\nrescue\nelse\n  b\nend\n",
        ),
        (
            "begin\n  a\nrescue\n\nensure\n  b\nend\n",
            vec![(4, "before", "ensure")],
            "begin\n  a\nrescue\nensure\n  b\nend\n",
        ),
        // A rescue modifier that is the whole body (or a `begin`'s first
        // statement) is a parser `rescue` body.
        (
            "def foo\n\n  a rescue b\n\nend\n",
            vec![(2, "before", "rescue"), (4, "after", "rescue")],
            "def foo\n  a rescue b\nend\n",
        ),
        (
            "foo do\n\n  a rescue b\nend\n",
            vec![(2, "before", "rescue")],
            "foo do\n  a rescue b\nend\n",
        ),
        (
            "begin\n\n  a rescue b\n\n  c\nend\n",
            vec![(2, "before", "rescue"), (4, "after", "rescue")],
            "begin\n  a rescue b\n  c\nend\n",
        ),
        // Nested owners are checked separately.
        (
            "def foo\n  begin\n    a\n\n  rescue\n    b\n  end\n\nrescue\n  c\nend\n",
            vec![(4, "before", "rescue"), (8, "before", "rescue")],
            "def foo\n  begin\n    a\n  rescue\n    b\n  end\nrescue\n  c\nend\n",
        ),
    ] {
        expect(source, &expected, after);
    }
}

#[test]
fn no_offense_lookalikes() {
    for source in [
        "",
        // `it` blocks have no pinned callback.
        "foo do\n  it\n\nrescue\n\n  b\nend\n",
        "f = -> do\n  it\n\nrescue\n  b\nend\n",
        // Class bodies, the program and nested non-owner statements.
        "class C\n  a\n\nrescue\n\n  b\nend\n",
        "a\n\nrescue_me\n",
        "def foo\n  a\n\n  b rescue c\nend\n",
        "def foo\n  x = a rescue b\n\nend\n",
        // Whitespace-only lines are not empty.
        "begin\n  a\n  \nrescue\n \n  b\nend\n",
        // Comments are not blank lines.
        "begin\n  a\n  # c\nrescue\n  # d\n  b\nend\n",
        // Multi-line endless definitions have no `end`; upstream raises.
        "def foo =\n  a rescue b\n",
        "def foo = a rescue b\n",
        // Single-line forms.
        "begin; a; rescue; b; ensure; c; end\n\nx\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
}

#[test]
fn guarded_removals() {
    let before = "\u{feff}# é\r\nbegin\r\n  a\r\n\r\nrescue\r\n\r\n  # 日本\r\n  b\r\nend\r\n";
    let found = correction_offenses(&EmptyLinesAroundExceptionHandlingKeywords, before);
    assert_eq!(found.len(), 2);
    for o in &found {
        assert_eq!(
            &before.as_bytes()[o.correction.as_ref().unwrap().range.clone()],
            b"\r\n"
        );
    }
    assert_correction(
        &EmptyLinesAroundExceptionHandlingKeywords,
        before,
        "\u{feff}# é\r\nbegin\r\n  a\r\nrescue\r\n  # 日本\r\n  b\r\nend\r\n",
    );
    for held in [
        "begin\n  a\n\nrescue\n  b\nend\np __LINE__\n",
        "begin\n  a\n\nrescue\n  __LINE__\nend\n",
        // A backslash continuation before the blank line.
        "begin\n  a \\\n\nrescue\n  b\nend\n",
        // Directive comments next to the blank line.
        "begin\n  a # rubocop:disable Lint/Debugger\n\nrescue\n  b\nend\n",
        "begin\n  a\nrescue # rustocop:enable Lint/Debugger\n\n  b\nend\n",
        // A heredoc body opened on the keyword's line.
        "begin\n  a\nrescue then p <<~H\n\nH\nend\n",
    ] {
        assert!(!findings(held).is_empty(), "{held:?}");
        assert_correction(&EmptyLinesAroundExceptionHandlingKeywords, held, held);
    }
    // Literal text and `__END__` data are not lines around keywords.
    for source in [
        "begin\n  x = \"a\n\n\"\nrescue\n  b\nend\n",
        "begin\n  a\nend\n__END__\nbegin\n  a\n\nrescue\nend\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
}

#[test]
fn listed_source_observers_hold_every_correction() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:p).source_location",
        "p SCRIPT_LINES__",
        "p DATA.read\n__END__\nx",
    ] {
        let source = format!("begin\n  a\n\nrescue\n\n  b\nend\n{observer}\n");
        assert_eq!(findings(&source).len(), 2, "{source:?}");
        assert_correction(&EmptyLinesAroundExceptionHandlingKeywords, &source, &source);
    }
    assert_correction(
        &EmptyLinesAroundExceptionHandlingKeywords,
        "begin\n  a\n\nrescue\n  b\nend\np DATA\n",
        "begin\n  a\nrescue\n  b\nend\np DATA\n",
    );
}
