//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/empty_lines_around_begin_body_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 11
//! pinned examples run at the fixed `no_empty_lines` style; findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded
//! in the pinned RuboCop reference run (source and corrected SHA-256
//! of every row were checked against it when the rows were ported).
use super::*;
use crate::cops::testing::{assert_correction, correction_offenses};

const COP: &str = "Layout/EmptyLinesAroundBeginBody";

/// `(line, col, message)` of each finding, from the full default pipeline.
fn findings(source: &str) -> Vec<(u32, u32, String)> {
    crate::pipeline::lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert_eq!(o.end_line, o.line);
            (o.line, o.col, o.message)
        })
        .collect()
}

fn message(edge: &str) -> String {
    format!("Extra empty line detected at `begin` body {edge}.")
}

/// `(spec line, source, findings as (line, edge), corrected source)`.
type Pinned = (
    u32,
    &'static str,
    &'static [(u32, &'static str)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (
        11,
        "begin\n\n  foo\nend\n",
        &[(2, "beginning")],
        "begin\n  foo\nend\n",
    ),
    (
        27,
        "begin\n  foo\nensure\n  bar\n\nend\n",
        &[(5, "end")],
        "begin\n  foo\nensure\n  bar\nend\n",
    ),
    (
        47,
        "begin\n  foo\n\nend\n",
        &[(3, "end")],
        "begin\n  foo\nend\n",
    ),
    (
        63,
        "def bar\n  begin\n\n    foo\n  end\nend\n",
        &[(3, "beginning")],
        "def bar\n  begin\n    foo\n  end\nend\n",
    ),
    (
        83,
        "def bar\n  begin\n    foo\n\n  end\nend\n",
        &[(4, "end")],
        "def bar\n  begin\n    foo\n  end\nend\n",
    ),
    (
        103,
        "begin\n\n  foo\nrescue\n  bar\nend\n",
        &[(2, "beginning")],
        "begin\n  foo\nrescue\n  bar\nend\n",
    ),
    (
        123,
        "begin\n  foo\nrescue\n  bar\n\nend\n",
        &[(5, "end")],
        "begin\n  foo\nrescue\n  bar\nend\n",
    ),
    (
        143,
        "begin\n  foo\nrescue\n  bar\nelse\n  baz\n\nend\n",
        &[(7, "end")],
        "begin\n  foo\nrescue\n  bar\nelse\n  baz\nend\n",
    ),
    (
        167,
        "begin\n\n  do_something1\nrescue RuntimeError\n  do_something2\nrescue ArgumentError => ex\n  do_something3\nrescue\n  do_something3\nelse\n  do_something4\nensure\n  do_something4\n\nend\n",
        &[(2, "beginning"), (14, "end")],
        "begin\n  do_something1\nrescue RuntimeError\n  do_something2\nrescue ArgumentError => ex\n  do_something3\nrescue\n  do_something3\nelse\n  do_something4\nensure\n  do_something4\nend\n",
    ),
    // The shared `accepts` examples.
    (6, "begin\n  foo\nend\n", &[], ""),
    (6, "def foo\n  begin\n    bar\n  end\nend\n", &[], ""),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    for &(spec, source, expected, after) in PINNED {
        let found = findings(source);
        assert_eq!(
            found,
            expected
                .iter()
                .map(|&(line, edge)| (line, 1, message(edge)))
                .collect::<Vec<_>>(),
            "spec line {spec}"
        );
        if expected.is_empty() {
            assert!(correction_offenses(&EmptyLinesAroundBeginBody, source).is_empty());
        } else {
            assert_correction(&EmptyLinesAroundBeginBody, source, after);
        }
    }
}

#[test]
fn only_explicit_begin_keywords_are_bodies() {
    for source in [
        "",
        // Implicit begin bodies of methods, blocks, classes and programs.
        "def foo\n\n  bar\nrescue\n  baz\n\nend\n",
        "foo do\n\n  bar\nensure\n  baz\n\nend\n",
        "class C\n  x\nrescue\n  y\nend\n",
        "\nbar\nrescue\nbaz\n",
        // Parentheses, `BEGIN`, single-line forms and blank-free bodies.
        "(\n\n  foo\n\n)\n",
        "BEGIN {\n\n  foo\n\n}\n",
        "begin; foo; end\n\nbar\n",
        "begin foo end\n",
        "x = begin\n  foo\nend\n",
        // A line holding only whitespace is not empty.
        "begin\n  \n  foo\n \nend\n",
        // Comments are not blank lines.
        "begin\n  # lead\n\n  foo\n\n  # tail\nend\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
}

#[test]
fn begin_forms_nesting_and_empty_bodies() {
    for (source, expected, after) in [
        // Empty bodies report one beginning line.
        ("begin\n\nend\n", vec![(2, "beginning")], "begin\nend\n"),
        (
            "begin\n\n\nend\n",
            vec![(2, "beginning"), (3, "end")],
            "begin\nend\n",
        ),
        // Only the adjacent line at each edge; more padding needs a pass.
        (
            "begin\n\n\n  foo\n\n\nend\n",
            vec![(2, "beginning"), (6, "end")],
            "begin\n\n  foo\n\nend\n",
        ),
        // Assigned, post-loop, chained, argument and nested forms.
        (
            "x = begin\n\n  foo\n\nend\n",
            vec![(2, "beginning"), (4, "end")],
            "x = begin\n  foo\nend\n",
        ),
        (
            "begin\n\n  foo\nend while bar\n",
            vec![(2, "beginning")],
            "begin\n  foo\nend while bar\n",
        ),
        (
            "begin\n  foo\n\nend.to_s\n",
            vec![(3, "end")],
            "begin\n  foo\nend.to_s\n",
        ),
        (
            "p(begin\n\n  1\nend)\n",
            vec![(2, "beginning")],
            "p(begin\n  1\nend)\n",
        ),
        (
            "begin\n\n  begin\n\n    foo\n\n  end\n\nend\n",
            vec![(2, "beginning"), (4, "beginning"), (6, "end"), (8, "end")],
            "begin\n  begin\n    foo\n  end\nend\n",
        ),
        // Nested rescue/ensure keywords are not body edges.
        (
            "begin\n  foo\nrescue\n\n  bar\n\nensure\n\n  baz\nend\n",
            vec![],
            "",
        ),
        // A `begin` in a block or a method body edge belongs to this cop
        // only for its own lines.
        (
            "foo do\n  begin\n\n    bar\n  end\nend\n",
            vec![(3, "beginning")],
            "foo do\n  begin\n    bar\n  end\nend\n",
        ),
    ] {
        let found = findings(source);
        assert_eq!(
            found,
            expected
                .iter()
                .map(|&(line, edge)| (line, 1, message(edge)))
                .collect::<Vec<_>>(),
            "{source:?}"
        );
        if !expected.is_empty() {
            assert_correction(&EmptyLinesAroundBeginBody, source, after);
        }
    }
}

#[test]
fn guarded_removals() {
    // CRLF, a BOM and multibyte comments keep their bytes.
    let before = "\u{feff}# é\r\nbegin\r\n\r\n  # 日本\r\n  foo\r\n\r\nend\r\n";
    let found = correction_offenses(&EmptyLinesAroundBeginBody, before);
    assert_eq!(found.len(), 2);
    for o in &found {
        assert_eq!(
            &before.as_bytes()[o.correction.as_ref().unwrap().range.clone()],
            b"\r\n"
        );
    }
    assert_correction(
        &EmptyLinesAroundBeginBody,
        before,
        "\u{feff}# é\r\nbegin\r\n  # 日本\r\n  foo\r\nend\r\n",
    );
    for held in [
        // A later `__LINE__` value would change.
        "begin\n\n  foo\nend\np __LINE__\n",
        "begin\n\n  __LINE__\nend\n",
        // A heredoc body opened on the `begin` line.
        "x = <<~H + begin\n\nH\n  1\nend\n",
    ] {
        assert!(!findings(held).is_empty(), "{held:?}");
        assert_correction(&EmptyLinesAroundBeginBody, held, held);
    }
    // Directive comments next to the blank line do not hold the edit.
    for source in [
        "begin # rubocop:disable Lint/Debugger\n\n  foo\nend\n",
        "begin\n\n  # rustocop:enable Lint/Debugger\n  foo\nend\n",
    ] {
        assert_correction(
            &EmptyLinesAroundBeginBody,
            source,
            &source.replace("\n\n", "\n"),
        );
    }
    // Literal interiors and the data section are not padding.
    for source in [
        "begin\n  x = \"a\n\nb\"\nend\n",
        "begin\n  x = <<~H\n    a\n\n  H\nend\n",
        "begin\n  foo\nend\n__END__\nbegin\n\nend\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
}
