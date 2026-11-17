//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/block_end_newline_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 18 pinned examples
//! (including the `:ruby27` numbered-parameter and `:ruby34` `it` contexts,
//! which apply at the fixed Ruby 3.4) run at the fixed defaults. Findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded
//! in the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported). The related
//! `spec/rubocop/cop/cop_spec.rb` context names the cop only as a sample
//! class for generic `Cop` behaviour, with no example about this cop's rule.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/BlockEndNewline";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn findings(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&BlockEndNewline], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(
                o.message,
                format!(
                    "Expression at {}, {} should be on its own line.",
                    o.line, o.col
                )
            );
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect();
    found.sort();
    found
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&BlockEndNewline, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (5, "test do foo end", &[], ""),
    (9, "test do\n  foo\nend\n", &[], ""),
    (17, "test do\n  foo\n; end\n", &[], ""),
    (
        25,
        "test do\n  foo end\n",
        &[(2, 7)],
        "test do\n  foo\nend\n",
    ),
    (39, "test {\n  foo }\n", &[(2, 7)], "test {\n  foo\n}\n"),
    (54, "test {\n  |foo| }\n", &[(2, 9)], "test {\n  |foo|\n}\n"),
    (
        69,
        "test {\n  foo }.bar.baz\n",
        &[(2, 7)],
        "test {\n  foo\n}.bar.baz\n",
    ),
    (
        84,
        "foo(one {\n  x }, two {\n  y })\n",
        &[(2, 5), (3, 5)],
        "foo(one {\n  x\n}, two {\n  y\n})\n",
    ),
    (
        103,
        "test {\n  foo(<<~EOS) }\n    Heredoc text.\n  EOS\n",
        &[(2, 15)],
        "test {\n  foo(<<~EOS)\n    Heredoc text.\n  EOS\n}\n",
    ),
    (
        122,
        "test {\n  foo(<<~FIRST, <<~SECOND) }\n    Heredoc text.\n  FIRST\n    Heredoc text.\n  SECOND\n",
        &[(2, 28)],
        "test {\n  foo(<<~FIRST, <<~SECOND)\n    Heredoc text.\n  FIRST\n    Heredoc text.\n  SECOND\n}\n",
    ),
    (
        145,
        "test {\n  foo(<<~EOS).bar }\n    Heredoc text.\n  EOS\n",
        &[(2, 19)],
        "test {\n  foo(<<~EOS).bar\n    Heredoc text.\n  EOS\n}\n",
    ),
    (
        164,
        "test {\n  foo(<<~FIRST).bar(<<~SECOND) }\n    Heredoc text.\n  FIRST\n    Heredoc text.\n  SECOND\n",
        &[(2, 32)],
        "test {\n  foo(<<~FIRST).bar(<<~SECOND)\n    Heredoc text.\n  FIRST\n    Heredoc text.\n  SECOND\n}\n",
    ),
    (
        186,
        "foo {\n  { bar: :baz } }\n",
        &[(2, 17)],
        "foo {\n  { bar: :baz }\n}\n",
    ),
    (
        200,
        "foo {\n  bar(baz: :quux) }\n",
        &[(2, 19)],
        "foo {\n  bar(baz: :quux)\n}\n",
    ),
    (
        216,
        "test {\n  _1 }.bar.baz\n",
        &[(2, 6)],
        "test {\n  _1\n}.bar.baz\n",
    ),
    (
        231,
        "test {\n  _1.push(<<~EOS) }\n    Heredoc text.\n  EOS\n",
        &[(2, 19)],
        "test {\n  _1.push(<<~EOS)\n    Heredoc text.\n  EOS\n}\n",
    ),
    (
        252,
        "test {\n  it }.bar.baz\n",
        &[(2, 6)],
        "test {\n  it\n}.bar.baz\n",
    ),
    (
        267,
        "test {\n  it.push(<<~EOS) }\n    Heredoc text.\n  EOS\n",
        &[(2, 19)],
        "test {\n  it.push(<<~EOS)\n    Heredoc text.\n  EOS\n}\n",
    ),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 18);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&BlockEndNewline, source).is_empty());
        } else {
            assert_correction(&BlockEndNewline, source, corrected);
        }
    }
}

#[test]
fn single_line_and_own_line_ends_are_accepted() {
    for source in [
        "test { foo }\n",
        "test do |x| foo end\n",
        "test do\n  foo\n  end\n",
        "test {\n\tfoo\n\t}\n",
        "test do\n  foo\n;end\n",
        "test do\n  foo;\n  ; end\n",
        "test do |x|\n  foo\nend.bar\n",
        "-> do\n  foo\nend\n",
        "foo(&blk)\n",
        "for x in y do\n  foo end\n",
        "while x do\n  foo end\n",
        "BEGIN {\n  foo }\n",
        "def f = (\n  foo )\n",
        // The parser's last child is a location-less `args` node: upstream
        // raises.
        "foo do\n  ; end\n",
        "foo {\n  ; }\n",
    ] {
        assert_eq!(findings(source), [], "{source}");
    }
}

#[test]
fn every_parser_block_kind_is_checked() {
    for (source, expected, after) in [
        ("super do\n  foo end\n", (2, 7), "super do\n  foo\nend\n"),
        ("a&.b {\n  foo }\n", (2, 7), "a&.b {\n  foo\n}\n"),
        ("-> (x) {\n  x }\n", (2, 5), "-> (x) {\n  x\n}\n"),
        ("-> (x) {\n  ; }\n", (2, 5), "-> (x) {\n  ; }\n"),
        (
            "foo(\n  1) {\n  |x| }\n",
            (3, 7),
            "foo(\n  1) {\n  |x|\n}\n",
        ),
        ("foo {\n  || }\n", (2, 6), "foo {\n  ||\n}\n"),
        ("-> (\n  x) {\n  x }\n", (3, 5), "-> (\n  x) {\n  x\n}\n"),
        (
            "test do\n  foo\nrescue\n  bar end\n",
            (4, 7),
            "test do\n  foo\nrescue\n  bar\nend\n",
        ),
        (
            "test do\n  foo\nensure end\n",
            (3, 8),
            "test do\n  foo\nensure\nend\n",
        ),
        (
            "test do\n  foo\n  bar end\n",
            (3, 7),
            "test do\n  foo\n  bar\nend\n",
        ),
        ("test {\n  foo }  \n", (2, 7), "test {\n  foo\n}  \n"),
        ("test {\n  foo  }\n", (2, 8), "test {\n  foo\n}\n"),
        (
            "test {\n  foo \\\n  ; }\n",
            (3, 5),
            "test {\n  foo \\\n  ; }\n",
        ),
        ("é {\n  é }\n", (2, 5), "é {\n  é\n}\n"),
        // A CRLF line keeps its ending (upstream inserts a bare `\n`).
        ("test {\r\n  foo }\r\n", (2, 7), "test {\r\n  foo\r\n}\r\n"),
    ] {
        assert_eq!(findings(source), [expected], "{source}");
        if source != after {
            assert_correction(&BlockEndNewline, source, after);
        }
    }
}

/// Rubocop-ast's `BlockNode#single_line?` compares the lines of
/// `loc.begin` and `loc.end`, not the call's start. A one-line block after a
/// multi-line receiver chain or argument list is single-line (each row was
/// checked against the pinned Team, source only).
#[test]
fn one_line_blocks_after_multi_line_receivers_or_arguments_are_single_line() {
    for source in [
        "result = client\n  .fetch(id)\n  .map { |r| r + 1 }\n",
        "result = client\n  .fetch(id)\n  .map do |r| r + 1 end\n",
        "result = client\n  &.fetch(id)\n  &.map { _1 + 1 }\n",
        "client\n  .fetch(id)\n  .each { it }\n",
        "client\r\n  .fetch(id)\r\n  .map { |r| r + 1 }\r\n",
        "\u{feff}client\n  .fetch(id)\n  .map { |r| r }\n",
        "client # c\n  .fetch(id) # d\n  .map { |r| r + 1 } # e\n",
        "client\n  .fetch(<<~A)\n  x\nA\n  .map { |r| r }\n",
        "def run\n  items\n    .select(&:ok?)\n    .each { |i| process(i) }\nend\n",
        "x = { a: list\n  .map { |v| v } }\n",
        "foo(list\n  .map { |v| v })\n",
        "list\n  .map { |v| v }\n  .each do |w| w end\n",
        "foo(\n  1\n) { bar }\n",
        "foo bar,\n  baz do |x| x end\n",
        "super(a,\n  b) { |x| x }\n",
        "-> (\n  x) { }\n",
        "->(a,\n  b) do a end\n",
        "foo(\n  1) { |x| }\n",
        "foo(\n  1) { || }\n",
        "foo(\n  1) {}\n",
        "foo(\n  1) do end\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
        assert!(
            correction_offenses(&BlockEndNewline, source).is_empty(),
            "{source:?}"
        );
    }
}

/// A block that genuinely spans lines at the end of a multi-line
/// receiver chain is still reported and corrected as the pinned Team does.
#[test]
fn multi_line_blocks_after_multi_line_receivers_are_still_reported() {
    for (source, expected, after) in [
        (
            "result = client\n  .fetch(id)\n  .map { |r|\n  r + 1 }\n",
            (4, 9),
            "result = client\n  .fetch(id)\n  .map { |r|\n  r + 1\n}\n",
        ),
        (
            "client\n  .fetch(id)\n  .map { _1 +\n  1 }\n",
            (4, 5),
            "client\n  .fetch(id)\n  .map { _1 +\n  1\n}\n",
        ),
        (
            "client\n  .fetch(id)\n  .map { it +\n  1 }\n",
            (4, 5),
            "client\n  .fetch(id)\n  .map { it +\n  1\n}\n",
        ),
        (
            "client\n  .fetch(id)\n  .map do |a,\n  b| a end\n",
            (4, 8),
            "client\n  .fetch(id)\n  .map do |a,\n  b| a\nend\n",
        ),
        (
            "client\r\n  .fetch(id)\r\n  .map { |r|\r\n  r + 1 }\r\n",
            (4, 9),
            "client\r\n  .fetch(id)\r\n  .map { |r|\r\n  r + 1\r\n}\r\n",
        ),
    ] {
        assert_eq!(findings(source), [expected], "{source:?}");
        assert_correction(&BlockEndNewline, source, after);
    }
}

#[test]
fn heredoc_arguments_move_the_end_after_the_terminator() {
    for (before, after) in [
        (
            "test {\n  foo(<<~X, 1) }\n  t\nX\n",
            "test {\n  foo(<<~X, 1)\n  t\nX\n}\n",
        ),
        (
            "test {\n  foo(<<-X) }\n  t\n  X\n",
            "test {\n  foo(<<-X)\n  t\n  X\n}\n",
        ),
        (
            "test {\r\n  foo(<<~X) }\r\n  t\r\nX\r\n",
            "test {\r\n  foo(<<~X)\r\n  t\r\nX\r\n}\r\n",
        ),
        (
            "test {\n  foo <<~X }\n  t\nX",
            "test {\n  foo <<~X\n  t\nX\n}",
        ),
    ] {
        assert_eq!(fixed(before), after, "{before}");
        assert_correction(&BlockEndNewline, before, after);
    }
}
