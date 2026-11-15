//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/first_array_element_line_break_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop is
//! opt-in; all 18 pinned examples run at the fixed defaults
//! (`AllowImplicitArrayLiterals: false`, `AllowMultilineFinalElement: false`,
//! `Layout/SpaceInsideArrayLiteralBrackets` fixed `no_space`). The contexts
//! configuring `AllowImplicitArrayLiterals: true`,
//! `AllowMultilineFinalElement: true` and the `compact` bracket style are
//! replayed at those defaults, so their sources are reported. Findings,
//! messages and single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected SHA-256
//! was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/FirstArrayElementLineBreak";

/// `(line, col)` of each finding, checking message, severity and end line.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&FirstArrayElementLineBreak], source)
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
    let offenses = correction_offenses(&FirstArrayElementLineBreak, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&FirstArrayElementLineBreak, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source is accepted.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (137, "b = [\n  :a,\n  :b]\n", &[], ""),
    (
        6,
        "a = [:a,\n     :b]\n",
        &[(1, 6)],
        "a = [\n:a,\n     :b]\n",
    ),
    (24, "%w(a b\n   c d)\n", &[(1, 4)], "%w(\na b\n   c d)\n"),
    // The shared seam holds this edit (see `HELD`).
    (
        40,
        "method([:foo,\n        :bar])\n",
        &[(1, 9)],
        "method([\n:foo,\n        :bar])\n",
    ),
    (94, "a, b,\nc =\n1, 2,\n3\n", &[], ""),
    (
        59,
        "a, b,\nc = 1,\n2, 3\n",
        &[(2, 5)],
        "a, b,\nc = \n1,\n2, 3\n",
    ),
    (77, "a\n.c = 1,\n2, 3\n", &[(2, 6)], "a\n.c = \n1,\n2, 3\n"),
    // `AllowImplicitArrayLiterals: true`, replayed at the fixed `false`.
    (127, "a, b,\nc =\n1, 2,\n3\n", &[], ""),
    (
        108,
        "a, b,\nc = 1,\n2, 3\n",
        &[(2, 5)],
        "a, b,\nc = \n1,\n2, 3\n",
    ),
    (118, "a\n.c = 1,\n2, 3\n", &[(2, 6)], "a\n.c = \n1,\n2, 3\n"),
    // `AllowMultilineFinalElement: true`, replayed at the fixed `false`.
    (
        148,
        "[a, b, {\n  c: d\n}]\n",
        &[(1, 2)],
        "[\na, b, {\n  c: d\n}]\n",
    ),
    (156, "[{\n  a: b\n}]\n", &[(1, 2)], "[\n{\n  a: b\n}]\n"),
    (
        164,
        "[a, {\n  b: c\n}, d]\n",
        &[(1, 2)],
        "[\na, {\n  b: c\n}, d]\n",
    ),
    (180, "[a, b,\nc]\n", &[(1, 2)], "[\na, b,\nc]\n"),
    // The `compact` bracket style, replayed at the fixed `no_space`.
    (
        204,
        "[[:a, :b],\n [:c, :d]]\n",
        &[(1, 2)],
        "[\n[:a, :b],\n [:c, :d]]\n",
    ),
    (211, "[:a,\n [:c, :d]]\n", &[(1, 2)], "[\n:a,\n [:c, :d]]\n"),
    (
        219,
        "x = [1, 2],\n  [3, 4]\n",
        &[(1, 5)],
        "x = \n[1, 2],\n  [3, 4]\n",
    ),
    (
        237,
        "[[:a, :b],\n [:c, :d]]\n",
        &[(1, 2)],
        "[\n[:a, :b],\n [:c, :d]]\n",
    ),
];

/// Pinned corrections withheld by the shared correction boundary, finding
/// kept: a receiverless `method(<dynamic>)` is a possible `source_location`
/// acquisition for the listed-observer and code-object barriers (as
/// `object.method(argument)`).
const HELD: &[u32] = &[40];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 18);
    for &(line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "spec line {line}");
        if HELD.contains(&line) {
            held(source);
            // Without the reflective selector the same edit is applied.
            let renamed = source.replace("method(", "meth(");
            assert_eq!(fixed(&renamed), after.replace("method(", "meth("));
        } else if expected.is_empty() {
            assert!(
                correction_offenses(&FirstArrayElementLineBreak, source).is_empty(),
                "spec line {line}"
            );
        } else {
            assert_correction(&FirstArrayElementLineBreak, source, after);
        }
    }
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(FirstArrayElementLineBreak.name(), COP);
    assert!(!FirstArrayElementLineBreak.default_enabled());
    assert!(!FirstArrayElementLineBreak.department_selection_enables());
    assert!(FirstArrayElementLineBreak.safe_fixes());
    assert!(
        FirstArrayElementLineBreak
            .autocorrect_incompatible_with()
            .is_empty()
    );
    assert!(
        crate::pipeline::lint_str("a = [:a,\n     :b]\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_array_literal_kind_is_checked() {
    for (source, expected) in [
        ("[1,\n 2]\n", (1, 2)),
        ("%w[a\n b]\n", (1, 4)),
        ("%W[a#{b}\n c]\n", (1, 4)),
        ("%i[a\n b]\n", (1, 4)),
        ("%I[a\n b]\n", (1, 4)),
        ("[*a,\n b]\n", (1, 2)),
        ("[a: 1,\n b: 2]\n", (1, 2)),
        ("[**h,\n a: 1]\n", (1, 2)),
        // A single multi-line element.
        ("[foo(\n  1\n)]\n", (1, 2)),
        // Nested in calls, blocks and interpolation.
        ("foo [1,\n 2]\n", (1, 6)),
        ("foo { [1,\n 2] }\n", (1, 8)),
        ("\"#{[1,\n 2]}\"\n", (1, 5)),
        // Implicit arrays after `=` on the same line.
        ("x = 1,\n  2\n", (1, 5)),
        ("x=1,\n  2\n", (1, 3)),
        ("x = *a,\n  b\n", (1, 5)),
        ("@x = 1,\n  2\n", (1, 6)),
        ("X = 1,\n  2\n", (1, 5)),
        ("a[1] = 2,\n  3\n", (1, 8)),
        ("a, b = 1,\n  2\n", (1, 8)),
        ("*a, b = 1,\n  2\n", (1, 9)),
        // Character columns, not bytes.
        ("é = [1,\n 2]\n", (1, 6)),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
    }
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "[]\n",
        "[\n]\n",
        "[1, 2]\n",
        "%w[a b]\n",
        "[\n  1,\n  2\n]\n",
        "[ # comment\n  1, 2]\n",
        "%w[\n  a b\n]\n",
        // The array starts after the operator's line.
        "x =\n  1,\n  2\n",
        "x = \\\n  1,\n  2\n",
        // Implicit lists that are not arrays.
        "foo 1,\n  2\n",
        "return 1,\n  2\n",
        "def f\n  yield 1,\n    2\nend\n",
        "foo(1,\n  2)\n",
        "foo[1,\n  2]\n",
        "case a\nwhen 1,\n  2\nend\n",
        "begin\nrescue A,\n  B\nend\n",
        "a, b = c\n",
        "a,\n  b = c\n",
        // Array patterns are not arrays.
        "case a\nin [1,\n  2]\nend\n",
        "a => [b,\n  c]\n",
        // Braced hashes are another cop's.
        "{ a: 1,\n  b: 2 }\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn nested_findings_are_independent() {
    assert_eq!(positions("[[1,\n  2],\n 3]\n"), [(1, 2), (1, 3)]);
    assert_eq!(positions("x = [1,\n  2],\n  3\n"), [(1, 5), (1, 6)]);
    // An inner array starting on a later line is checked on its own.
    assert_eq!(positions("[\n  [1,\n   2]]\n"), [(2, 4)]);
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        ("x = [1, # c\n  2]\n", "x = [\n1, # c\n  2]\n"),
        // Upstream inserts `\n`, also in CRLF files.
        ("x = [1,\r\n  2]\r\n", "x = [\n1,\r\n  2]\r\n"),
        ("\u{feff}x = [1,\n  2]\n", "\u{feff}x = [\n1,\n  2]\n"),
        ("%w[a b\n  c]\n", "%w[\na b\n  c]\n"),
        ("%i(a\n  b)\n", "%i(\na\n  b)\n"),
        ("x = *a,\n  b\n", "x = \n*a,\n  b\n"),
        ("[\"a\",\n  'b']\n", "[\n\"a\",\n  'b']\n"),
        // Nested arrays: both insertions in one pass.
        ("[[1,\n  2],\n 3]\n", "[\n[\n1,\n  2],\n 3]\n"),
        ("x = [1,\n  2],\n  3\n", "x = \n[\n1,\n  2],\n  3\n"),
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
fn findings_only_without_requested_corrections() {
    let offenses = lint_str_with(&[&FirstArrayElementLineBreak], "x = [1,\n  2]\n");
    assert_eq!(offenses.len(), 1);
    assert!(offenses[0].correction.is_none());
    assert_eq!(offenses[0].end_line, 1);
    let offenses = lint_str_with(&[&FirstArrayElementLineBreak], "[foo(\n  1\n), 2]\n");
    assert_eq!((offenses[0].line, offenses[0].end_line), (1, 3));
}
