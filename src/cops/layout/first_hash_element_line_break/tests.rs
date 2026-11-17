//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/first_hash_element_line_break_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop is
//! opt-in; all 11 pinned examples run at the fixed
//! `AllowMultilineFinalElement: false`, so the `AllowMultilineFinalElement:
//! true` context is replayed at that default and its sources are reported.
//! Findings, messages and single-pass corrected bytes equal the fixed replay
//! recorded in the pinned RuboCop reference run (each source and
//! corrected SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{cops::testing::correction_offenses, pipeline::lint_str_with};

const COP: &str = "Layout/FirstHashElementLineBreak";

/// `(line, col)` of each finding, checking message, severity and end line.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&FirstHashElementLineBreak], source)
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
    let offenses = correction_offenses(&FirstHashElementLineBreak, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(FirstHashElementLineBreak.name(), COP);
    assert!(!FirstHashElementLineBreak.default_enabled());
    assert!(!FirstHashElementLineBreak.department_selection_enables());
    assert!(FirstHashElementLineBreak.safe_fixes());
    assert!(
        FirstHashElementLineBreak
            .autocorrect_incompatible_with()
            .is_empty()
    );
    assert!(
        crate::pipeline::lint_str("a = { a: 1,\n      b: 2 }\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_braced_hash_is_checked() {
    for (source, expected) in [
        ("{a: 1,\n b: 2}\n", (1, 2)),
        ("{ \"a\" => 1,\n  b => 2 }\n", (1, 3)),
        ("{ **a,\n  b: 1 }\n", (1, 3)),
        ("{ a:,\n  b: }\n", (1, 3)),
        ("{ a: foo(\n  1\n) }\n", (1, 3)),
        ("foo({ a: 1,\n  b: 2 })\n", (1, 7)),
        ("foo { { a: 1,\n  b: 2 } }\n", (1, 9)),
        ("[{ a: 1,\n  b: 2 }]\n", (1, 4)),
        ("x = { é: 1,\n  b: 2 }\n", (1, 7)),
    ] {
        assert_eq!(positions(source), [expected], "{source}");
    }
}

#[test]
fn accepted_lookalikes() {
    for source in [
        "{}\n",
        "{\n}\n",
        "{ a: 1, b: 2 }\n",
        "{\n  a: 1,\n  b: 2\n}\n",
        "{ # comment\n  a: 1, b: 2 }\n",
        // Keyword arguments and implicit hashes are not braced.
        "foo(a: 1,\n  b: 2)\n",
        "foo a: 1,\n  b: 2\n",
        "[a: 1,\n  b: 2]\n",
        "foo[a: 1,\n  b: 2]\n",
        // Hash patterns are not hashes.
        "case a\nin { b: 1,\n  c: 2 }\nend\n",
        // Blocks are not hashes.
        "foo { |a|\n  a }\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn nested_findings_are_independent() {
    assert_eq!(
        positions("{ a: { b: 1,\n  c: 2 },\n  d: 3 }\n"),
        [(1, 3), (1, 8)]
    );
    assert_eq!(positions("{\n  a: { b: 1,\n    c: 2 } }\n"), [(2, 8)]);
}

#[test]
fn corrections_insert_one_line_break_and_keep_every_other_byte() {
    for (source, after) in [
        (
            "x = { a: 1, # c\n  b: 2 }\n",
            "x = { \na: 1, # c\n  b: 2 }\n",
        ),
        // Upstream inserts `\n`, also in CRLF files.
        (
            "x = { a: 1,\r\n  b: 2 }\r\n",
            "x = { \na: 1,\r\n  b: 2 }\r\n",
        ),
        (
            "\u{feff}x = { a: 1,\n  b: 2 }\n",
            "\u{feff}x = { \na: 1,\n  b: 2 }\n",
        ),
        ("x = { **a,\n  b: 1 }\n", "x = { \n**a,\n  b: 1 }\n"),
        ("x = { a:,\n  b: }\n", "x = { \na:,\n  b: }\n"),
        (
            "x = { \"a\": 1,\n  b: 2 }\n",
            "x = { \n\"a\": 1,\n  b: 2 }\n",
        ),
        // Nested hashes: both insertions in one pass.
        (
            "{ a: { b: 1,\n  c: 2 },\n  d: 3 }\n",
            "{ \na: { \nb: 1,\n  c: 2 },\n  d: 3 }\n",
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
fn findings_only_without_requested_corrections() {
    let offenses = lint_str_with(&[&FirstHashElementLineBreak], "x = { a: 1,\n  b: 2 }\n");
    assert_eq!(offenses.len(), 1);
    assert!(offenses[0].correction.is_none());
    let offenses = lint_str_with(&[&FirstHashElementLineBreak], "{ a: foo(\n  1\n) }\n");
    assert_eq!((offenses[0].line, offenses[0].end_line), (1, 3));
}
