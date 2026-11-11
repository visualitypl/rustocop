//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_hash_key_line_breaks_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop is
//! opt-in; all 10 pinned examples run at the fixed
//! `AllowMultilineFinalElement: false`. The `ignore last element` context
//! configures `true`; it is replayed at the fixed `false`, so its multi-line
//! final values are reported too. Findings, messages and single-pass
//! corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected SHA-256
//! was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/MultilineHashKeyLineBreaks";

/// `(line, col)` of each finding, checking message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineHashKeyLineBreaks], source)
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
    let offenses = correction_offenses(&MultilineHashKeyLineBreaks, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&MultilineHashKeyLineBreaks, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source is accepted.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (7, "{\n  foo: 1, bar: \"2\"\n}\n", &[], ""),
    (17, "{\n  foo => 1, bar => \"2\"\n}\n", &[], ""),
    (
        26,
        "{\n  foo: 1,\n  baz: 3, bar: \"2\"}\n",
        &[(3, 11)],
        "{\n  foo: 1,\n  baz: 3, \nbar: \"2\"}\n",
    ),
    (
        43,
        "{\n  foo => 1,\n  baz => 3, bar: \"2\"}\n",
        &[(3, 13)],
        "{\n  foo => 1,\n  baz => 3, \nbar: \"2\"}\n",
    ),
    (63, "{foo: 1, bar: \"2\"}\n", &[], ""),
    (
        70,
        "{foo: 1,\n  baz: 3, bar: \"2\"}\n",
        &[(2, 11)],
        "{foo: 1,\n  baz: 3, \nbar: \"2\"}\n",
    ),
    (
        84,
        "{foo: 1,\n  baz: {\n    as: 12,\n  }, bar: \"2\"}\n",
        &[(4, 6)],
        "{foo: 1,\n  baz: {\n    as: 12,\n  }, \nbar: \"2\"}\n",
    ),
    // `AllowMultilineFinalElement: true`, replayed at the fixed `false`.
    (
        105,
        "{foo: 1, bar: {\n  a: 1\n}}\n",
        &[(1, 10)],
        "{foo: 1, \nbar: {\n  a: 1\n}}\n",
    ),
    (
        113,
        "{foo: 1, bar: {\n  a: 1,\n}, baz: 3}\n",
        &[(1, 10)],
        "{foo: 1, \nbar: {\n  a: 1,\n}, baz: 3}\n",
    ),
    (
        130,
        "{foo: 1, bar: 2,\n  baz: 3}\n",
        &[(1, 10)],
        "{foo: 1, \nbar: 2,\n  baz: 3}\n",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 10);
    for &(line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineHashKeyLineBreaks, source).is_empty(),
                "spec line {line}"
            );
        } else {
            assert_correction(&MultilineHashKeyLineBreaks, source, after);
        }
    }
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(MultilineHashKeyLineBreaks.name(), COP);
    assert!(!MultilineHashKeyLineBreaks.default_enabled());
    assert!(!MultilineHashKeyLineBreaks.department_selection_enables());
    assert!(MultilineHashKeyLineBreaks.safe_fixes());
    assert!(
        MultilineHashKeyLineBreaks
            .autocorrect_incompatible_with()
            .is_empty()
    );
    assert!(
        crate::pipeline::lint_str("{foo: 1,\n  baz: 3, bar: \"2\"}\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_braced_hash_child_kind_is_checked() {
    for (source, expected) in [
        ("{ a: 1,\n  b: 2, c: 3 }\n", vec![(2, 9)]),
        ("{ a => 1,\n  b => 2, c => 3 }\n", vec![(2, 11)]),
        ("{ \"a\": 1,\n  \"b\": 2, \"c\": 3 }\n", vec![(2, 11)]),
        ("{ **a,\n  **b, c: 1 }\n", vec![(2, 8)]),
        ("{ a:,\n  b:, c: }\n", vec![(2, 7)]),
        ("{ a: 1, b: 2, c: 3,\n  d: 4 }\n", vec![(1, 9), (1, 15)]),
        // A multi-line value: the next key on its last line.
        ("{ a: foo(\n  1\n), b: 2 }\n", vec![(3, 4)]),
        ("{ a: 1, b: foo(\n  2\n) }\n", vec![(1, 9)]),
        // Nested in calls, arrays and blocks.
        ("foo({ a: 1,\n  b: 2, c: 3 })\n", vec![(2, 9)]),
        ("[{ a: 1,\n  b: 2, c: 3 }]\n", vec![(2, 9)]),
        ("foo { { a: 1,\n  b: 2, c: 3 } }\n", vec![(2, 9)]),
        // Character columns, not bytes.
        ("{ é: 1,\n  é2: 2, c: 3 }\n", vec![(2, 10)]),
    ] {
        assert_eq!(positions(source), expected, "{source}");
    }
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "{}\n",
        "{\n}\n",
        "{ a: 1, b: 2 }\n",
        "{ a: 1, b: 2,\n}\n",
        "{\n  a: 1,\n  b: 2\n}\n",
        "{ a: 1,\n  b: 2,\n  c: 3 }\n",
        "{ # comment\n  a: 1, b: 2 }\n",
        // One key, however many lines its value takes.
        "{ a: foo(\n  1, 2\n) }\n",
        // Brace-less hashes are the argument cop's.
        "foo(a: 1, b: 2,\n  c: 3)\n",
        "foo a: 1, b: 2,\n  c: 3\n",
        "[a: 1, b: 2,\n  c: 3]\n",
        "x[a: 1, b: 2,\n  c: 3]\n",
        // Hash patterns are not hashes.
        "case a\nin {b: 1, c: 2,\n  d: 3}\nend\n",
        // Blocks are not hashes.
        "foo { |a, b|\n  a; b }\n",
        // The key after a heredoc opened on its line.
        "{ a: 1,\n  b: <<~A, c: 2 }\n  t\nA\n",
        "{ a: 1,\n  **foo(<<~A), c: 2 }\n  t\nA\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn nested_findings_are_independent() {
    assert_eq!(
        positions("{ a: { b: 1,\n  c: 2, d: 3 },\n e: 4 }\n"),
        [(2, 9)]
    );
    assert_eq!(
        positions("{ a: 1, b: { c: 2,\n  d: 3, e: 4 } }\n"),
        [(1, 9), (2, 9)]
    );
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        (
            "x = { a: 1,\n  b: 2, c: 3 # c\n}\n",
            "x = { a: 1,\n  b: 2, \nc: 3 # c\n}\n",
        ),
        // Upstream inserts `\n`, also in CRLF files.
        (
            "x = { a: 1,\r\n  b: 2, c: 3 }\r\n",
            "x = { a: 1,\r\n  b: 2, \nc: 3 }\r\n",
        ),
        (
            "\u{feff}x = { a: 1,\n  b: 2, c: 3 }\n",
            "\u{feff}x = { a: 1,\n  b: 2, \nc: 3 }\n",
        ),
        (
            "x = { \"a\": 1,\n  'b' => 2, **c }\n",
            "x = { \"a\": 1,\n  'b' => 2, \n**c }\n",
        ),
        // Nested hashes: both insertions in one pass.
        (
            "{ a: 1, b: { c: 2,\n  d: 3, e: 4 } }\n",
            "{ a: 1, \nb: { c: 2,\n  d: 3, \ne: 4 } }\n",
        ),
        // A heredoc value's opener after the moved key's start.
        (
            "{ a: 1,\n  b: 2, c: <<~A }\n  t\nA\n",
            "{ a: 1,\n  b: 2, \nc: <<~A }\n  t\nA\n",
        ),
        (
            "{ a: 1,\n  b: 2, **<<~A }\n  t\nA\n",
            "{ a: 1,\n  b: 2, \n**<<~A }\n  t\nA\n",
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
        held(&format!("y = {observer}\nx = {{ a: 1,\n  b: 2, c: 3 }}\n"));
    }
    held("x = { a: 1,\n  b: 2, c: 3 }\nDATA.read\n__END__\nz\n");
    held("x = { a: 1,\n  b: 2, c: 3 }\np __LINE__\n");
    held("foo(<<~A, { a: 1, b: 2,\n  body\nA\n  c: 3 })\n");
    assert_eq!(
        fixed("y = 1\nx = { a: 1,\n  b: 2, c: 3 }\n"),
        "y = 1\nx = { a: 1,\n  b: 2, \nc: 3 }\n"
    );
}
