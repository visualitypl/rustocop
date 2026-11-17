//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/empty_lines_around_block_body_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 28
//! pinned examples run at the fixed `no_empty_lines` style: the
//! `EnforcedStyle: empty_lines` contexts are replayed at the fixed default,
//! like the inventory's fixed results, and the `:ruby27` numbered-parameter
//! context (pinned at 3.3) applies unchanged at the fixed 3.4. Findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded
//! in the pinned RuboCop reference run (source and corrected SHA-256
//! of every row were checked against it when the rows were ported).
use super::*;
use crate::cops::testing::{assert_correction, correction_offenses};

const COP: &str = "Layout/EmptyLinesAroundBlockBody";

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
    format!("Extra empty line detected at block body {edge}.")
}

fn expect(source: &str, expected: &[(u32, &str)], after: &str) {
    assert_eq!(
        findings(source),
        expected
            .iter()
            .map(|&(line, edge)| (line, 1, message(edge)))
            .collect::<Vec<_>>(),
        "{source:?}"
    );
    if expected.is_empty() {
        assert!(correction_offenses(&EmptyLinesAroundBlockBody, source).is_empty());
    } else {
        assert_correction(&EmptyLinesAroundBlockBody, source, after);
    }
}

/// `(spec line, source, findings as (line, edge), corrected source)`.
type Pinned = (
    u32,
    &'static str,
    &'static [(u32, &'static str)],
    &'static str,
);

/// The examples run for `{ }` and `do end`, with `%o`/`%c` for the
/// delimiters.
const BOTH_DELIMITERS: &[Pinned] = &[
    (
        12,
        "some_method %o\n\n  do_something\n%c\n",
        &[(2, "beginning")],
        "some_method %o\n  do_something\n%c\n",
    ),
    (
        28,
        "some_method %o\n  do_something\n\n  %c\n",
        &[(3, "end")],
        "some_method %o\n  do_something\n  %c\n",
    ),
    // Ruby 2.7: a numbered-parameter block.
    (
        45,
        "some_method %o\n  _1\n\n  %c\n",
        &[(3, "end")],
        "some_method %o\n  _1\n  %c\n",
    ),
    // Ruby 3.4: an `it` block.
    (
        63,
        "some_method %o\n  it\n\n  %c\n",
        &[(3, "end")],
        "some_method %o\n  it\n  %c\n",
    ),
    (80, "some_method %o\n  \n  do_something\n%c\n", &[], ""),
    (
        90,
        "some_method arg,\n  another_arg %o\n\n  do_something\n%c\n",
        &[(3, "beginning")],
        "some_method arg,\n  another_arg %o\n  do_something\n%c\n",
    ),
    (
        101,
        "some_method %o do_something %c\n\nsomething_else\n",
        &[],
        "",
    ),
    // `EnforcedStyle: empty_lines`, replayed at the fixed default.
    (113, "some_method %o\n  do_something\n%c\n", &[], ""),
    (131, "some_method %o\n%c", &[], ""),
    (
        135,
        "some_method %o do_something %c\nsomething_else\n",
        &[],
        "",
    ),
];

const SINGLE: &[Pinned] = &[
    // `EnforcedStyle: empty_lines` contexts, replayed at the fixed default.
    (147, "aa { bb { cc }\n.dd }\n", &[], ""),
    (154, "aa { bb\n&.cc }\n", &[], ""),
    (
        161,
        "aa {\n\n  bb\n  .cc }\n",
        &[(2, "beginning")],
        "aa {\n  bb\n  .cc }\n",
    ),
    (174, "foo :bar \\\ndo\n  baz\nend\n", &[], ""),
    (
        198,
        "foo :bar \\\ndo\n\n  baz\nend\n",
        &[(3, "beginning")],
        "foo :bar \\\ndo\n  baz\nend\n",
    ),
    (
        220,
        "foo do |bar,\n        baz|\n  do_something\nend\n",
        &[],
        "",
    ),
    (240, "foo {\n  |bar| do_something }\n", &[], ""),
    (
        251,
        "foo do |bar,\n        baz|\n\n  do_something\nend\n",
        &[(3, "beginning")],
        "foo do |bar,\n        baz|\n  do_something\nend\n",
    ),
];

/// A [`Pinned`] row with its delimiters filled in.
type Row = (u32, String, &'static [(u32, &'static str)], String);

fn pinned_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    for (open, close) in [("{", "}"), ("do", "end")] {
        for &(spec, source, expected, after) in BOTH_DELIMITERS {
            let fill = |text: &str| text.replace("%o", open).replace("%c", close);
            rows.push((spec, fill(source), expected, fill(after)));
        }
    }
    for &(spec, source, expected, after) in SINGLE {
        rows.push((spec, source.into(), expected, after.into()));
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
fn body_start_after_parameters_and_implicit_parameters() {
    for (source, expected, after) in [
        // Multi-line parameters move the start; a blank line inside them is
        // not body padding.
        ("foo do |a,\n\n  b|\n  a\nend\n", vec![], ""),
        (
            "foo { |a,\n  b|\n\n  a\n\n}\n",
            vec![(3, "beginning"), (5, "end")],
            "foo { |a,\n  b|\n  a\n}\n",
        ),
        (
            "foo do |a;\n  b|\n\n  a\nend\n",
            vec![(3, "beginning")],
            "foo do |a;\n  b|\n  a\nend\n",
        ),
        (
            "foo do |\n  a\n|\n\n  a\nend\n",
            vec![(4, "beginning")],
            "foo do |\n  a\n|\n  a\nend\n",
        ),
        // Empty pipes are no arguments: the start stays at `do`.
        (
            "foo do ||\n\n  1\nend\n",
            vec![(2, "beginning")],
            "foo do ||\n  1\nend\n",
        ),
        // Numbered and `it` parameters are implicit.
        (
            "foo do\n\n  _1 + _2\n\nend\n",
            vec![(2, "beginning"), (4, "end")],
            "foo do\n  _1 + _2\nend\n",
        ),
        (
            "foo {\n\n  it\n}\n",
            vec![(2, "beginning")],
            "foo {\n  it\n}\n",
        ),
        // Lambda literals are parser blocks.
        (
            "f = ->(a,\n  b) do\n\n  a\n\nend\n",
            vec![(3, "beginning"), (5, "end")],
            "f = ->(a,\n  b) do\n  a\nend\n",
        ),
        (
            "f = -> {\n\n  1\n}\n",
            vec![(2, "beginning")],
            "f = -> {\n  1\n}\n",
        ),
        (
            "f = lambda do |x|\n\n  x\nend\n",
            vec![(2, "beginning")],
            "f = lambda do |x|\n  x\nend\n",
        ),
        // `super` and safe-navigation calls take blocks too.
        (
            "def f\n  super do\n\n    1\n  end\nend\n",
            vec![(3, "beginning")],
            "def f\n  super do\n    1\n  end\nend\n",
        ),
        (
            "a&.each do\n\n  1\nend\n",
            vec![(2, "beginning")],
            "a&.each do\n  1\nend\n",
        ),
        // Empty bodies report one beginning line; more padding needs another
        // pass.
        ("foo do\n\nend\n", vec![(2, "beginning")], "foo do\nend\n"),
        (
            "foo do\n\n\n\nend\n",
            vec![(2, "beginning"), (4, "end")],
            "foo do\n\nend\n",
        ),
        // Bodies with rescue/ensure keep only their outer edges.
        (
            "foo do\n\n  a\nrescue\n\n  b\n\nensure\n  c\n\nend\n",
            vec![(2, "beginning"), (10, "end")],
            "foo do\n  a\nrescue\n\n  b\n\nensure\n  c\nend\n",
        ),
        // Nested blocks each report their own edges.
        (
            "a do\n\n  b do\n\n    c\n\n  end\n\nend\n",
            vec![(2, "beginning"), (4, "beginning"), (6, "end"), (8, "end")],
            "a do\n  b do\n    c\n  end\nend\n",
        ),
    ] {
        expect(source, &expected, after);
    }
}

#[test]
fn no_offense_lookalikes() {
    for source in [
        "",
        "foo(&blk)\n\nbar\n",
        "foo { |x| x }\n\nbar\n",
        "foo do\n  # lead\n\n  bar\n\n  # tail\nend\n",
        "foo do\n \n  bar\n\t\nend\n",
        // Not blocks: loops, BEGIN/END, hashes and parentheses.
        "for x in y do\n\n  x\n\nend\n",
        "while x do\n\n  y\n\nend\n",
        "BEGIN {\n\n  x\n\n}\n",
        "END {\n\n  x\n\n}\n",
        "h = {\n\n  a: 1\n\n}\n",
        "x = (\n\n  1\n\n)\n",
        // Method and class bodies belong to their own cops.
        "def f\n\n  1\n\nend\n",
        "class C\n\n  x\n\nend\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
}

#[test]
fn guarded_removals() {
    let before = "\u{feff}# é\r\nfoo do |x|\r\n\r\n  # 日本\r\n  x\r\n\r\nend\r\n";
    let found = correction_offenses(&EmptyLinesAroundBlockBody, before);
    assert_eq!(found.len(), 2);
    for o in &found {
        assert_eq!(
            &before.as_bytes()[o.correction.as_ref().unwrap().range.clone()],
            b"\r\n"
        );
    }
    assert_correction(
        &EmptyLinesAroundBlockBody,
        before,
        "\u{feff}# é\r\nfoo do |x|\r\n  # 日本\r\n  x\r\nend\r\n",
    );
    for held in [
        "foo do\n\n  bar\nend\np __LINE__\n",
        "foo do\n\n  __LINE__\nend\n",
        // An explicit continuation ends on the opening line.
        "foo do |x| \\\n\n  x\nend\n",
        // Directive comments next to the blank line.
        "foo do # rubocop:disable Lint/Debugger\n\n  bar\nend\n",
        "foo do\n  bar\n\nend # rustocop:enable Lint/Debugger\n",
        // A heredoc body opened on the block's line.
        "foo(<<~H) do\n\nH\n  1\nend\n",
        // A percent array's interpolated block.
        "%W(a#{foo do\n\n  1\nend})\n",
    ] {
        assert!(!findings(held).is_empty(), "{held:?}");
        assert_correction(&EmptyLinesAroundBlockBody, held, held);
    }
    // Code in string interpolation is corrected; string text is not padding.
    assert_correction(
        &EmptyLinesAroundBlockBody,
        "x = \"#{foo do\n\n  1\nend}\"\n",
        "x = \"#{foo do\n  1\nend}\"\n",
    );
    for source in [
        "foo do\n  x = \"a\n\nb\"\nend\n",
        "foo do\n  x = <<~H\n    a\n\n  H\nend\n",
        "foo do\nend\n__END__\nfoo do\n\nend\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
}
