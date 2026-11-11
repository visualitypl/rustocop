//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/first_method_argument_line_break_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop is
//! opt-in; all 14 pinned examples run at the fixed defaults
//! (`AllowMultilineFinalElement: false`, `AllowedMethods: []`), so the
//! `AllowMultilineFinalElement: true` and `AllowedMethods` contexts are
//! replayed at those defaults and their sources are reported. Findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected SHA-256
//! was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/FirstMethodArgumentLineBreak";

/// `(line, col)` of each finding, checking message and severity.
fn positions(source: &str) -> Vec<(u32, u32)> {
    findings(source)
        .into_iter()
        .map(|(line, col, _)| (line, col))
        .collect()
}

/// `(line, col, end line)` of each finding.
fn findings(source: &str) -> Vec<(u32, u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&FirstMethodArgumentLineBreak], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col, o.end_line)
        })
        .collect();
    found.sort();
    found
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&FirstMethodArgumentLineBreak, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&FirstMethodArgumentLineBreak, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source is accepted.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (6, "foo(bar,\n  baz)\n", &[(1, 5)], "foo(\nbar,\n  baz)\n"),
    (
        20,
        "super(bar,\n  baz)\n",
        &[(1, 7)],
        "super(\nbar,\n  baz)\n",
    ),
    (
        34,
        "receiver&.foo(bar,\n  baz)\n",
        &[(1, 15)],
        "receiver&.foo(\nbar,\n  baz)\n",
    ),
    (
        49,
        "something(3, bar: 1,\nbaz: 2)\n",
        &[(1, 11)],
        "something(\n3, bar: 1,\nbaz: 2)\n",
    ),
    (
        63,
        "something(bar: 1,\nbaz: 2)\n",
        &[(1, 11)],
        "something(\nbar: 1,\nbaz: 2)\n",
    ),
    (77, "foo(bar, baz, bing)", &[], ""),
    (81, "super(foo: 1, bar: 2)", &[], ""),
    (85, "foo bar,\n  baz\n", &[], ""),
    (92, "foo", &[], ""),
    // `AllowMultilineFinalElement: true`, replayed at the fixed `false`.
    (
        99,
        "foo(bar, {\n  a: b\n})\n",
        &[(1, 5)],
        "foo(\nbar, {\n  a: b\n})\n",
    ),
    (
        107,
        "foo({\n  a: b\n})\n",
        &[(1, 5)],
        "foo(\n{\n  a: b\n})\n",
    ),
    (
        115,
        "foo(bar, {\n  a: b\n}, baz)\n",
        &[(1, 5)],
        "foo(\nbar, {\n  a: b\n}, baz)\n",
    ),
    (131, "foo(bar,\nbaz)\n", &[(1, 5)], "foo(\nbar,\nbaz)\n"),
    // `AllowedMethods: [something]`, replayed at the fixed `[]`.
    (
        149,
        "something(3, bar: 1,\nbaz: 2)\n",
        &[(1, 11)],
        "something(\n3, bar: 1,\nbaz: 2)\n",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 14);
    for &(line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&FirstMethodArgumentLineBreak, source).is_empty(),
                "spec line {line}"
            );
        } else {
            assert_correction(&FirstMethodArgumentLineBreak, source, after);
        }
    }
    // Upstream's ranges: the flattened first pair, not the whole hash.
    assert_eq!(
        findings("something(bar: 1,\nbaz: 2)\n"),
        [(1, 11, 1)],
        "spec line 63"
    );
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(FirstMethodArgumentLineBreak.name(), COP);
    assert!(!FirstMethodArgumentLineBreak.default_enabled());
    assert!(!FirstMethodArgumentLineBreak.department_selection_enables());
    assert!(FirstMethodArgumentLineBreak.safe_fixes());
    assert_eq!(
        FirstMethodArgumentLineBreak.autocorrect_incompatible_with(),
        ["Style/MethodCallWithArgsParentheses"]
    );
    assert!(
        crate::pipeline::lint_str("foo(bar,\n  baz)\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_parenthesized_send_csend_and_super_is_checked() {
    for (source, expected) in [
        ("a.b(1,\n  2)\n", (1, 5, 1)),
        ("a&.b(1,\n  2)\n", (1, 6, 1)),
        ("a.(1,\n  2)\n", (1, 4, 1)),
        ("A::b(1,\n  2)\n", (1, 6, 1)),
        ("x = foo(1,\n  2)\n", (1, 9, 1)),
        ("foo( 1,\n  2)\n", (1, 6, 1)),
        ("foo(*a,\n  2)\n", (1, 5, 1)),
        ("foo(1,\n  *a)\n", (1, 5, 1)),
        // A block pass is an argument.
        ("foo(1,\n  &b)\n", (1, 5, 1)),
        ("super(1,\n  &b)\n", (1, 7, 1)),
        // A single multi-line argument.
        ("foo(bar(\n  1\n))\n", (1, 5, 3)),
        ("foo([1,\n  2])\n", (1, 5, 2)),
        // Keyword arguments are flattened into their pairs ...
        ("foo(a: 1,\n  b: 2)\n", (1, 5, 1)),
        ("foo(**a,\n  b: 2)\n", (1, 5, 1)),
        ("foo(a: bar(\n  1\n))\n", (1, 5, 3)),
        // ... unless a block pass follows them, as upstream's parser args.
        ("foo(a: 1,\n  b: 2, &c)\n", (1, 5, 2)),
        // Braced hashes are not flattened.
        ("foo({ a: 1 },\n  2)\n", (1, 5, 1)),
        ("foo({ a: 1,\n  b: 2 })\n", (1, 5, 2)),
        // Command calls as arguments.
        ("foo(bar 1,\n  2)\n", (1, 5, 2)),
        // Character columns, not bytes.
        ("é(1,\n  2)\n", (1, 3, 1)),
    ] {
        assert_eq!(findings(source), [expected], "{source}");
    }
    // Inner calls are checked on their own.
    assert_eq!(positions("foo(bar(1,\n  2))\n"), [(1, 5), (1, 9)]);
    assert_eq!(positions("foo(1, bar(2,\n  3))\n"), [(1, 5), (1, 12)]);
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "foo\n",
        "foo()\n",
        "foo(1, 2)\n",
        "foo(\n  1,\n  2)\n",
        "foo(\n  1, 2\n)\n",
        "foo( # c\n  1, 2)\n",
        // Without parentheses.
        "foo 1,\n  2\n",
        "foo (1),\n  2\n",
        "a.b 1,\n  2\n",
        "super 1,\n  2\n",
        // Index calls and operators.
        "foo[1,\n  2]\n",
        "a +\n  b\n",
        "a.b = 1,\n  2\n",
        // Upstream reads the node's first line: a receiver starting on an
        // earlier line keeps the arguments off it.
        "foo\n  .bar(1,\n    2)\n",
        "[1,\n  2].bar(3,\n  4)\n",
        // A block is not an argument.
        "foo(1) do\nend\n",
        "foo(1) {\n}\n",
        // `yield`, `super` without arguments and other keywords.
        "def f\n  yield(1,\n    2)\nend\n",
        "def f\n  super\nend\n",
        "def f(...)\n  g(...)\nend\n",
        // Parameters and arrays are other cops'.
        "def f(a,\n  b)\nend\n",
        "[1,\n  2]\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        ("foo(1, # c\n  2)\n", "foo(\n1, # c\n  2)\n"),
        // Upstream inserts `\n`, also in CRLF files.
        ("foo(1,\r\n  2)\r\n", "foo(\n1,\r\n  2)\r\n"),
        ("\u{feff}foo(1,\n  2)\n", "\u{feff}foo(\n1,\n  2)\n"),
        ("foo( 1,\n  2)\n", "foo( \n1,\n  2)\n"),
        ("foo(*a,\n  &:b)\n", "foo(\n*a,\n  &:b)\n"),
        ("foo(**a,\n  b: 2)\n", "foo(\n**a,\n  b: 2)\n"),
        ("foo(bar 1,\n  2)\n", "foo(\nbar 1,\n  2)\n"),
        ("a&.b(1,\n  2)\n", "a&.b(\n1,\n  2)\n"),
        // Nested calls: both insertions in one pass.
        ("foo(bar(1,\n  2))\n", "foo(\nbar(\n1,\n  2))\n"),
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
        held(&format!("y = {observer}\nfoo(1,\n  2)\n"));
    }
    held("foo(1,\n  2)\nDATA.read\n__END__\nz\n");
    held("foo(1,\n  2)\np __LINE__\n");
    held("foo(<<~A, bar(1,\n  body\nA\n  2))\n");
    held("foo(<<~A, 1,\n  body\nA\n  2)\n");
    // The shared code-object barrier: a block pass of an unknown object can
    // acquire a method object (`&b` calls `b.to_proc`).
    held("foo(1,\n  &b)\n");
    held("foo(1,\n  2)\nbar(&b)\n");
    assert_eq!(fixed("y = 1\nfoo(1,\n  2)\n"), "y = 1\nfoo(\n1,\n  2)\n");
}

#[test]
fn findings_only_without_requested_corrections() {
    let offenses = lint_str_with(&[&FirstMethodArgumentLineBreak], "foo(1,\n  2)\n");
    assert_eq!(offenses.len(), 1);
    assert!(offenses[0].correction.is_none());
}
