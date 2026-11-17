//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/empty_lines_around_arguments_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 22 pinned
//! examples run at the fixed defaults; findings, messages and single-pass
//! corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (source and corrected SHA-256 of
//! every row were checked against it when the rows were ported).
use super::*;
use crate::cops::testing::{assert_correction, correction_offenses};

const COP: &str = "Layout/EmptyLinesAroundArguments";

/// The lines of the findings, from the full default pipeline.
fn findings(source: &str) -> Vec<u32> {
    let mut found: Vec<_> = crate::pipeline::lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert_eq!(o.message, "Empty line detected around arguments.");
            assert_eq!((o.col, o.end_line), (1, o.line));
            o.line
        })
        .collect();
    found.sort_unstable();
    found
}

fn expect(source: &str, expected: &[u32], after: &str) {
    assert_eq!(findings(source), expected, "{source:?}");
    if expected.is_empty() {
        assert!(correction_offenses(&EmptyLinesAroundArguments, source).is_empty());
    } else {
        assert_correction(&EmptyLinesAroundArguments, source, after);
    }
}

/// `(spec line, source, finding lines, corrected source)`.
type Pinned = (u32, &'static str, &'static [u32], &'static str);

const PINNED: &[Pinned] = &[
    (8, "foo(\n\n  bar\n)\n", &[2], "foo(\n  bar\n)\n"),
    (
        24,
        "bar(\n  [baz, qux]\n\n)\n",
        &[3],
        "bar(\n  [baz, qux]\n)\n",
    ),
    (
        40,
        "foo.do_something(\n  baz,\n\n  qux: 0\n)\n",
        &[3],
        "foo.do_something(\n  baz,\n  qux: 0\n)\n",
    ),
    (
        58,
        "foo(\n  baz,\n\n  qux,\n\n  biz,\n\n)\n",
        &[3, 5, 7],
        "foo(\n  baz,\n  qux,\n  biz,\n)\n",
    ),
    (
        82,
        "foo(biz,\n\n    baz: 0)\n",
        &[2],
        "foo(biz,\n    baz: 0)\n",
    ),
    (
        96,
        "Foo.prepend(\n  a,\n\n  Module.new do\n    def something; end\n\n    def anything; end\n  end\n)\n",
        &[3],
        "Foo.prepend(\n  a,\n  Module.new do\n    def something; end\n\n    def anything; end\n  end\n)\n",
    ),
    (
        122,
        "class Foo\n\n  include Bar\n\n  def baz(qux)\n    fizz(\n      qux,\n\n      10\n    )\n  end\nend\n",
        &[8],
        "class Foo\n\n  include Bar\n\n  def baz(qux)\n    fizz(\n      qux,\n      10\n    )\n  end\nend\n",
    ),
    (
        154,
        "something(1, 5)\nsomething_else\n\nfoo(biz,\n\n    qux)\n\nquux.map do\n\nend.another.thing(\n\n  [baz]\n)\n",
        &[5, 11],
        "something(1, 5)\nsomething_else\n\nfoo(biz,\n    qux)\n\nquux.map do\n\nend.another.thing(\n  [baz]\n)\n",
    ),
    (206, "bar(qux,\n\n    78)\n", &[2], "bar(qux,\n    78)\n"),
    // Safe navigation.
    (
        189,
        "receiver&.foo(\n\n  bar\n)\n",
        &[2],
        "receiver&.foo(\n  bar\n)\n",
    ),
    (222, "foo(bar)\n", &[], ""),
    (
        228,
        "foo(\n  bar,\n  [],\n  baz = nil,\n  qux: 2\n)\n",
        &[],
        "",
    ),
    (239, "foo(bar,\n    [],\n    qux: 2)\n", &[], ""),
    (
        247,
        "Foo.prepend(Module.new do\n  def something; end\n\n  def anything; end\nend)\n",
        &[],
        "",
    ),
    (
        257,
        "fred.map do\n  <<-EOT\n    bar\n\n    foo\n  EOT\nend.join(\"\n\")\n",
        &[],
        "",
    ),
    (269, "foo.baz do\n\n  bar\nend.compact\n", &[], ""),
    (
        278,
        "bar(<<-DOCS)\n  foo\n\nDOCS\n  .call!(true)\n",
        &[],
        "",
    ),
    (288, "foo.\n\n  bar(arg)\n", &[], ""),
    (296, "foo.(\n  arg\n)\n", &[], ""),
    (304, "format('%d\n\n', 1)[0]\n", &[], ""),
    (313, "private(def bar\n\n  baz\nend)\n", &[], ""),
    (324, "foo(:bar, [1,\n\n           2]\n)\n", &[], ""),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 22);
    for &(spec, source, expected, after) in PINNED {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "spec line {spec}"
        );
        expect(source, expected, after);
    }
}

#[test]
fn send_shapes_and_ranges() {
    for (source, expected, after) in [
        // Whitespace-only lines inside the run count; only the line before
        // the argument is reported, more need another pass.
        ("foo(\n  \n  bar)\n", vec![2], "foo(\n  bar)\n"),
        ("foo(\n\n\n  bar)\n", vec![3], "foo(\n\n  bar)\n"),
        // A comment ends the run.
        ("foo(\n  # c\n\n  bar)\n", vec![3], "foo(\n  # c\n  bar)\n"),
        ("foo(\n  # c\n  bar\n)\n", vec![], ""),
        // Commands without parentheses, block-pass arguments, splats,
        // keywords and forwarding.
        ("foo a,\n\n  b\n", vec![2], "foo a,\n  b\n"),
        ("foo(a,\n\n  &:b)\n", vec![2], "foo(a,\n  &:b)\n"),
        (
            "def f(...)\n  g(\n\n    ...\n  )\nend\n",
            vec![3],
            "def f(...)\n  g(\n    ...\n  )\nend\n",
        ),
        ("foo(*a,\n\n  **b)\n", vec![2], "foo(*a,\n  **b)\n"),
        // Operator and attribute/index assignment sends.
        ("a +\n\n  b\n", vec![2], "a +\n  b\n"),
        ("a.b =\n\n  c\n", vec![2], "a.b =\n  c\n"),
        ("a[1] =\n\n  2\n", vec![2], "a[1] =\n  2\n"),
        // Index sends: the arguments, never the closing bracket.
        ("a[\n\n  1\n]\n", vec![2], "a[\n  1\n]\n"),
        ("a[\n  1\n\n]\n", vec![], ""),
        ("a[\n\n  1\n] += 2\n", vec![2], "a[\n  1\n] += 2\n"),
        (
            "a[\n\n  1\n], b = 2, 3\n",
            vec![2],
            "a[\n  1\n], b = 2, 3\n",
        ),
        // A block does not make the send multi-line.
        ("foo(1) do\n\n  bar\nend\n", vec![], ""),
        (
            "foo(1,\n\n  2) do\nend\n",
            vec![2],
            "foo(1,\n  2) do\nend\n",
        ),
        // The receiver and the selector on different lines.
        ("a\n  .b(\n\n    1)\n", vec![], ""),
        ("a.\n  b(\n\n    1)\n", vec![], ""),
        // A multi-line receiver ending on the selector's line is checked.
        (
            "a.b(\n\n  1).c(\n\n  2)\n",
            vec![2, 4],
            "a.b(\n  1).c(\n  2)\n",
        ),
        // `super`, `yield` and `not` are not checked sends.
        ("def f\n  super(\n\n    1)\nend\n", vec![], ""),
        ("def f\n  yield(\n\n    1)\nend\n", vec![], ""),
        ("not(\n\n  x)\n", vec![], ""),
        // Nested calls report their own runs.
        (
            "foo(\n\n  bar(\n\n    1\n\n  )\n)\n",
            vec![2, 4, 6],
            "foo(\n  bar(\n    1\n  )\n)\n",
        ),
        // After a heredoc body.
        (
            "foo(<<~H,\n  text\nH\n\n  bar)\n",
            vec![4],
            "foo(<<~H,\n  text\nH\n  bar)\n",
        ),
    ] {
        expect(source, &expected, after);
    }
}
