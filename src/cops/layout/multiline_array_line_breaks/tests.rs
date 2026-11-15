//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_array_line_breaks_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop is opt-in;
//! all 7 pinned examples run at the fixed `AllowMultilineFinalElement:
//! false`. The `ignore last element` context configures `true`; it is
//! replayed at the fixed `false`, so its multi-line final hash is reported
//! too. Findings, messages and single-pass corrected bytes equal the fixed
//! replay recorded in the pinned RuboCop reference run (each source
//! and corrected SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/MultilineArrayLineBreaks";

/// `(line, col)` of each finding, checking message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineArrayLineBreaks], source)
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
    let offenses = correction_offenses(&MultilineArrayLineBreaks, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source is accepted.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (6, "[1,2,3]\n", &[], ""),
    (14, "[\n  1,2,3,\n]\n", &[], ""),
    (24, "[1,\n  2, 4]\n", &[(2, 6)], "[1,\n  2, \n4]\n"),
    (40, "[1,\n  <<~TEXT, 2]\n    text\n  TEXT\n", &[], ""),
    (
        51,
        "[1,\n  [2, 3], 4]\n",
        &[(2, 11)],
        "[1,\n  [2, 3], \n4]\n",
    ),
    // `AllowMultilineFinalElement: true`, replayed at the fixed `false`.
    (
        69,
        "[1, 2, 3, {\n  a: 1\n}]\n",
        &[(1, 5), (1, 8), (1, 11)],
        "[1, \n2, \n3, \n{\n  a: 1\n}]\n",
    ),
    (
        77,
        "[1, 2, 3, {\n  a: 1\n}, 4]\n",
        &[(1, 5), (1, 8), (1, 11)],
        "[1, \n2, \n3, \n{\n  a: 1\n}, 4]\n",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 7);
    for &(line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineArrayLineBreaks, source).is_empty(),
                "spec line {line}"
            );
        } else {
            assert_correction(&MultilineArrayLineBreaks, source, after);
        }
    }
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(MultilineArrayLineBreaks.name(), COP);
    assert!(!MultilineArrayLineBreaks.default_enabled());
    assert!(!MultilineArrayLineBreaks.department_selection_enables());
    assert!(MultilineArrayLineBreaks.safe_fixes());
    assert!(
        MultilineArrayLineBreaks
            .autocorrect_incompatible_with()
            .is_empty()
    );
    assert!(
        crate::pipeline::lint_str("[1,\n  2, 4]\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_array_literal_kind_is_checked() {
    for (source, expected) in [
        ("[1,\n 2, 3]\n", vec![(2, 5)]),
        ("%w[a\n b c]\n", vec![(2, 4)]),
        ("%W[a#{b}\n c d]\n", vec![(2, 4)]),
        ("%i[a\n b c]\n", vec![(2, 4)]),
        ("%I[a\n b c]\n", vec![(2, 4)]),
        ("[*a,\n *b, c]\n", vec![(2, 6)]),
        // A brace-less hash is one element.
        ("[a,\n b: 1, c: 2]\n", vec![]),
        ("[a,\n b, c: 1]\n", vec![(2, 5)]),
        // Implicit arrays of assignments.
        ("x = 1, 2,\n  3\n", vec![(1, 8)]),
        ("a, b = 1, 2,\n  3\n", vec![(1, 11)]),
        ("x = *a, b,\n  c\n", vec![(1, 9)]),
        ("a.b = 1, 2,\n  3\n", vec![(1, 10)]),
        ("a[0] = 1, 2,\n  3\n", vec![(1, 11)]),
        // Every item after the first on an occupied line.
        ("[1, 2, 3,\n 4]\n", vec![(1, 5), (1, 8)]),
        // A multi-line item: the next one on its last line.
        ("[foo(\n  1\n), 2]\n", vec![(3, 4)]),
        ("[1, foo(\n  2\n)]\n", vec![(1, 5)]),
        // Nested in calls, blocks and interpolation.
        ("foo [1,\n 2, 3]\n", vec![(2, 5)]),
        ("foo { [1,\n 2, 3] }\n", vec![(2, 5)]),
        ("\"#{[1,\n 2, 3]}\"\n", vec![(2, 5)]),
        // Character columns, not bytes.
        ("[\"é\",\n \"é\", 3]\n", vec![(2, 7)]),
    ] {
        assert_eq!(positions(source), expected, "{source}");
    }
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "[]\n",
        "[\n]\n",
        "[1, 2]\n",
        "[1, 2,\n]\n",
        "[\n  1,\n  2\n]\n",
        "[\n  1, 2\n]\n",
        "[1,\n 2,\n 3]\n",
        "%w[a b]\n",
        "%w[\n  a\n  b\n]\n",
        "x = 1,\n  2\n",
        "[ # comment\n  1, 2]\n",
        // One item, however many lines it takes.
        "[foo(\n  1, 2\n)]\n",
        // Argument lists, `when`/`rescue` lists and returns are not arrays.
        "foo 1, 2,\n  3\n",
        "foo(1, 2,\n  3)\n",
        "return 1, 2,\n  3\n",
        "case a\nwhen 1, 2,\n  3\nend\n",
        "begin\nrescue A, B,\n  C\nend\n",
        "a, b,\n  c = d\n",
        // Array patterns are not arrays.
        "case a\nin [1, 2,\n  3]\nend\n",
        // Braced hashes are another cop's.
        "{ a: 1, b: 2,\n  c: 3 }\n",
        // The element after a heredoc opened on its line.
        "[1,\n  <<~A, 2]\n  t\nA\n",
        "[1,\n  foo(<<~A), 2]\n  t\nA\n",
        "[1,\n  \"#{<<~A}\", 2]\n  t\nA\n",
        "[1,\n  <<~`A`, 2]\n  t\nA\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn heredocs_opened_on_other_lines_or_later_do_not_exempt() {
    // The heredoc opens on an earlier line than the offending item.
    assert_eq!(positions("[<<~A,\n  t\nA\n  1, 2]\n"), [(4, 6)]);
    // The heredoc is the offending item itself.
    assert_eq!(positions("[1,\n  2, <<~A]\n  t\nA\n"), [(2, 6)]);
    // A heredoc in a later item does not exempt an earlier one.
    assert_eq!(positions("[1,\n  2, 3, <<~A]\n  t\nA\n"), [(2, 6), (2, 9)]);
    // A `<<` string lookalike is no heredoc.
    assert_eq!(positions("[1,\n  \"<<~A\", 2]\n"), [(2, 11)]);
}

#[test]
fn nested_findings_are_independent() {
    assert_eq!(positions("[[1,\n  2, 3],\n 4]\n"), [(2, 6)]);
    assert_eq!(positions("[1, [2,\n  3]]\n"), [(1, 5)]);
    assert_eq!(positions("[1, [2,\n  3, 4]]\n"), [(1, 5), (2, 6)]);
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        ("x = [1,\n  2, 3 # c\n]\n", "x = [1,\n  2, \n3 # c\n]\n"),
        // Upstream inserts `\n`, also in CRLF files.
        ("x = [1,\r\n  2, 3]\r\n", "x = [1,\r\n  2, \n3]\r\n"),
        ("\u{feff}x = [1,\n  2, 3]\n", "\u{feff}x = [1,\n  2, \n3]\n"),
        ("%w[a\n  b c]\n", "%w[a\n  b \nc]\n"),
        ("%i(a\n  b c)\n", "%i(a\n  b \nc)\n"),
        ("x = 1, 2,\n  3\n", "x = 1, \n2,\n  3\n"),
        ("[\"a\",\n  'b', :c]\n", "[\"a\",\n  'b', \n:c]\n"),
        // Nested arrays: both insertions in one pass.
        ("[1, [2,\n  3, 4]]\n", "[1, \n[2,\n  3, \n4]]\n"),
        // A heredoc opener elsewhere on the item's line, after it.
        (
            "[1,\n  2, foo(<<~A)]\n  t\nA\n",
            "[1,\n  2, \nfoo(<<~A)]\n  t\nA\n",
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
fn findings_carry_the_item_extent() {
    let offenses = lint_str_with(&[&MultilineArrayLineBreaks], "[1,\n  2, foo(\n  3\n)]\n");
    assert_eq!(offenses.len(), 1);
    assert_eq!((offenses[0].line, offenses[0].col), (2, 6));
    assert_eq!(offenses[0].end_line, 4);
    assert!(offenses[0].correction.is_none());
}
