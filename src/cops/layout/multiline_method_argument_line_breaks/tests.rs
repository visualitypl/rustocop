//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_method_argument_line_breaks_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop
//! is opt-in; all 20 pinned examples run at the fixed
//! `AllowMultilineFinalElement: false`. The contexts configuring `true` are
//! replayed at the fixed `false`, so their multi-line final arguments are
//! reported too. Findings, messages and single-pass corrected bytes equal the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source and corrected SHA-256 was checked against it when the rows were
//! ported); the one edit before a heredoc opener is held (see `HELD`).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/MultilineMethodArgumentLineBreaks";

/// `(line, col)` of each finding, checking message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineMethodArgumentLineBreaks], source)
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
    let offenses = correction_offenses(&MultilineMethodArgumentLineBreaks, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&MultilineMethodArgumentLineBreaks, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source is accepted.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (6, "taz(\"abc\")\n", &[], ""),
    (
        14,
        "class Thing\n  def call\n    bar['foo'] = ::Time.zone.at(\n                   huh['foo'],\n                 )\n  end\nend\n",
        &[],
        "",
    ),
    (28, "a['b',\n    'c', 'd'] = e\n", &[], ""),
    (37, "taz(\n  \"abc\", \"foo\"\n)\n", &[], ""),
    // The shared heredoc boundary holds this edit (see `HELD`).
    (
        47,
        "taz(\"abc\",\n\"foo\", <<~TEXT, \"bar\")\n  text\nTEXT\n",
        &[(2, 8)],
        "taz(\"abc\",\n\"foo\", \n<<~TEXT, \"bar\")\n  text\nTEXT\n",
    ),
    (
        65,
        "foo(bar(<<~TEXT,\n  text\nTEXT\n  1), baz)\n",
        &[(4, 7)],
        "foo(bar(<<~TEXT,\n  text\nTEXT\n  1), \nbaz)\n",
    ),
    (
        85,
        "taz(\"abc\",\n\"foo\", \"bar\",\n\"baz\"\n)\n",
        &[(2, 8)],
        "taz(\"abc\",\n\"foo\", \n\"bar\",\n\"baz\"\n)\n",
    ),
    (
        105,
        "taz(\"abc\",\n\"foo\", \"bar\", \"barz\",\n\"baz\"\n)\n",
        &[(2, 8), (2, 15)],
        "taz(\"abc\",\n\"foo\", \n\"bar\", \n\"barz\",\n\"baz\"\n)\n",
    ),
    (
        127,
        "taz(\"abc\",\n\"foo\", \"bar\", z: \"barz\",\nx: \"baz\"\n)\n",
        &[(2, 8), (2, 15)],
        "taz(\"abc\",\n\"foo\", \n\"bar\", \nz: \"barz\",\nx: \"baz\"\n)\n",
    ),
    (
        149,
        "taz(\"abc\", {\n  foo: \"edf\",\n})\n",
        &[(1, 12)],
        "taz(\"abc\", \n{\n  foo: \"edf\",\n})\n",
    ),
    (
        167,
        "taz({\n  foo: \"edf\",\n}, \"abc\")\n",
        &[(3, 4)],
        "taz({\n  foo: \"edf\",\n}, \n\"abc\")\n",
    ),
    (
        185,
        "do_something(foo, bar, baz,\n  quux)\n",
        &[(1, 19), (1, 24)],
        "do_something(foo, \nbar, \nbaz,\n  quux)\n",
    ),
    (204, "foo&.bar(baz)\n", &[], ""),
    (212, "foo&.bar(baz, quux)\n", &[], ""),
    (
        220,
        "foo&.bar(baz, quux,\n  corge)\n",
        &[(1, 15)],
        "foo&.bar(baz, \nquux,\n  corge)\n",
    ),
    (
        236,
        "foo&.bar['foo'] = ::Time.zone.at(\n                    huh['foo'],\n                  )\n",
        &[],
        "",
    ),
    // `AllowMultilineFinalElement: true`, replayed at the fixed `false`.
    (
        248,
        "foo&.bar(1, 2, 3, {\n  a: 1,\n})\n",
        &[(1, 13), (1, 16), (1, 19)],
        "foo&.bar(1, \n2, \n3, \n{\n  a: 1,\n})\n",
    ),
    (
        261,
        "foo(1, 2, 3, {\n  a: 1,\n})\n",
        &[(1, 8), (1, 11), (1, 14)],
        "foo(1, \n2, \n3, \n{\n  a: 1,\n})\n",
    ),
    (
        269,
        "foo(1, 2, 3, {\n  a: 1,\n}, 4)\n",
        &[(1, 8), (1, 11), (1, 14)],
        "foo(1, \n2, \n3, \n{\n  a: 1,\n}, 4)\n",
    ),
    (
        290,
        "foo(1, 2, 3,\n4)\n",
        &[(1, 8), (1, 11)],
        "foo(1, \n2, \n3,\n4)\n",
    ),
];

/// Pinned corrections withheld by the shared heredoc boundary, finding kept:
/// a line break inserted right before a heredoc opener (as for the
/// first-element seam).
const HELD: &[u32] = &[47];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 20);
    for &(line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "spec line {line}");
        if HELD.contains(&line) {
            held(source);
            // A plain string in the opener's place takes the same edit (and
            // no longer exempts the argument after it).
            assert_eq!(
                fixed("taz(\"abc\",\n\"foo\", \"text\", \"bar\")\n"),
                "taz(\"abc\",\n\"foo\", \n\"text\", \n\"bar\")\n"
            );
        } else if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineMethodArgumentLineBreaks, source).is_empty(),
                "spec line {line}"
            );
        } else {
            assert_correction(&MultilineMethodArgumentLineBreaks, source, after);
        }
    }
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(MultilineMethodArgumentLineBreaks.name(), COP);
    assert!(!MultilineMethodArgumentLineBreaks.default_enabled());
    assert!(!MultilineMethodArgumentLineBreaks.department_selection_enables());
    assert!(MultilineMethodArgumentLineBreaks.safe_fixes());
    assert!(
        MultilineMethodArgumentLineBreaks
            .autocorrect_incompatible_with()
            .is_empty()
    );
    assert!(
        crate::pipeline::lint_str("foo(1, 2,\n  3)\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_send_kind_is_checked() {
    for (source, expected) in [
        ("foo(1, 2,\n  3)\n", vec![(1, 8)]),
        ("foo 1, 2,\n  3\n", vec![(1, 8)]),
        ("a.foo(1, 2,\n  3)\n", vec![(1, 10)]),
        ("a&.foo 1, 2,\n  3\n", vec![(1, 11)]),
        ("a::foo(1, 2,\n  3)\n", vec![(1, 11)]),
        ("a.(1, 2,\n  3)\n", vec![(1, 7)]),
        // Index reads, and the `[]` read of an index operator assignment.
        ("a[1, 2,\n  3]\n", vec![(1, 6)]),
        ("a.[](1, 2,\n  3)\n", vec![(1, 9)]),
        ("a[1, 2,\n  3] += 4\n", vec![(1, 6)]),
        ("a[1, 2,\n  3] ||= 4\n", vec![(1, 6)]),
        ("a[1, 2,\n  3] &&= 4\n", vec![(1, 6)]),
        ("a&.[](1, 2,\n  3)\n", vec![(1, 10)]),
        // A trailing brace-less keyword hash is flattened into its pairs.
        ("foo(a: 1, b: 2,\n  c: 3)\n", vec![(1, 11)]),
        ("foo(x, a: 1, **b,\n  c: 3)\n", vec![(1, 8), (1, 14)]),
        ("foo x, \"a\" => 1,\n  c: 3\n", vec![(1, 8)]),
        // A block pass is an argument and keeps the hash before it whole.
        ("foo(a, b: 1,\n  c: 2, &d)\n", vec![(1, 8)]),
        ("foo(a, b,\n  c, &d)\n", vec![(1, 8), (2, 6)]),
        ("def f(&) = foo(a, b,\n  c, &)\n", vec![(1, 19), (2, 6)]),
        // Flattened pairs, the last one multi-line.
        ("foo(a: 1, b: {\n  c: 2 })\n", vec![(1, 11)]),
        // A braced hash is one argument.
        ("foo(a, { b: 1,\n  c: 2 })\n", vec![(1, 8)]),
        // Splats, forwarding and blocks.
        ("foo(*a, b,\n  **c)\n", vec![(1, 9)]),
        ("def f(...) = foo(a, b,\n  ...)\n", vec![(1, 21)]),
        ("foo(a, b,\n  c) { d }\n", vec![(1, 8)]),
        ("foo a, b,\n  c do\nend\n", vec![(1, 8)]),
        // A multi-line argument: the next one on its last line.
        ("foo(bar(\n  1\n), 2)\n", vec![(3, 4)]),
        ("foo(1, bar(\n  2\n))\n", vec![(1, 8)]),
        // Nested calls are independent.
        ("foo(bar(1, 2,\n  3), 4)\n", vec![(1, 12), (2, 7)]),
        // Character columns, not bytes.
        ("foo(\"é\", \"é\",\n  3)\n", vec![(1, 10)]),
    ] {
        assert_eq!(positions(source), expected, "{source}");
    }
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "foo\n",
        "foo()\n",
        "foo(1, 2)\n",
        "foo(\n  1, 2\n)\n",
        "foo(1,\n  2,\n  3)\n",
        "foo(1, 2,\n)\n",
        "foo( # comment\n  1, 2)\n",
        // One argument, however many lines it takes.
        "foo(bar(\n  1, 2\n))\n",
        // Index assignments and their targets are `[]=`.
        "a[1, 2,\n  3] = 4\n",
        "a&.[]=(1, 2,\n  3)\n",
        "a[1, 2,\n  3], b = 4, 5\n",
        // Keywords are not sends.
        "def f(*)\n  super(1, 2,\n    3)\nend\n",
        "def f\n  yield 1, 2,\n    3\nend\n",
        "def f\n  return 1, 2,\n    3\nend\n",
        "foo { next 1, 2,\n  3 }\n",
        // Operators have one argument.
        "a +\n  b\n",
        // Arrays and braced hashes are other cops'.
        "[1, 2,\n  3]\n",
        "{ a: 1, b: 2,\n  c: 3 }\n",
        // The argument after a heredoc opened on its line.
        "foo(1,\n  <<~A, 2)\n  t\nA\n",
        "foo(1,\n  bar(<<~A), 2)\n  t\nA\n",
        "foo(1,\n  a: <<~A, b: 2)\n  t\nA\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        ("foo(1,\n  2, 3 # c\n)\n", "foo(1,\n  2, \n3 # c\n)\n"),
        // Upstream inserts `\n`, also in CRLF files.
        ("foo(1,\r\n  2, 3)\r\n", "foo(1,\r\n  2, \n3)\r\n"),
        ("\u{feff}foo(1,\n  2, 3)\n", "\u{feff}foo(1,\n  2, \n3)\n"),
        // Command calls continue after the comma.
        ("foo 1, 2,\n  3\n", "foo 1, \n2,\n  3\n"),
        ("foo(a: 1, b: 2,\n  c: 3)\n", "foo(a: 1, \nb: 2,\n  c: 3)\n"),
        ("foo(a, b,\n  c, &:d)\n", "foo(a, \nb,\n  c, \n&:d)\n"),
        ("a[1, 2,\n  3] += 4\n", "a[1, \n2,\n  3] += 4\n"),
        // Nested calls: both insertions in one pass.
        ("foo(bar(1, 2,\n  3), 4)\n", "foo(bar(1, \n2,\n  3), \n4)\n"),
        // A heredoc opener after the moved argument's start.
        (
            "foo(1,\n  2, bar(<<~A))\n  t\nA\n",
            "foo(1,\n  2, \nbar(<<~A))\n  t\nA\n",
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
