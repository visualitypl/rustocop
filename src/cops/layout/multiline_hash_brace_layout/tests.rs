//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_hash_brace_layout_spec.rb` and the shared
//! `spec/support/multiline_literal_brace_layout_examples.rb`,
//! `multiline_literal_brace_layout_method_argument_examples.rb` and
//! `multiline_literal_brace_layout_trailing_comma_examples.rb` it includes,
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 37
//! pinned examples run at the fixed `EnforcedStyle: symmetrical`; the
//! `new_line` and `same_line` contexts are replayed at it, so their findings
//! follow the symmetrical rule. Findings, messages and single-pass corrected
//! bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected SHA-256
//! was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/MultilineHashBraceLayout";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineHashBraceLayout], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert!(
                o.message == MESSAGES.same_line || o.message == MESSAGES.new_line,
                "{}",
                o.message
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

/// The finding messages, in position order.
fn messages(source: &str) -> Vec<String> {
    lint_str_with(&[&MultilineHashBraceLayout], source)
        .into_iter()
        .map(|o| o.message)
        .collect()
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&MultilineHashBraceLayout, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&MultilineHashBraceLayout, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

const MAIN: &str = "main";
const LITERAL: &str = "multiline literal brace layout";
const ARGUMENT: &str = "multiline literal brace layout method argument";
const TRAILING_COMMA: &str = "multiline literal brace layout trailing comma";

/// `(group, spec line, source, findings, single-pass correction)`; an empty
/// correction means no edit is applied.
type Pinned = (
    &'static str,
    u32,
    &'static str,
    &'static [(u32, u32)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (MAIN, 7, "foo(a: 1,\nb: 2)\n", &[], ""),
    (MAIN, 14, "{a: 1, b: 2}", &[], ""),
    (MAIN, 18, "{}", &[], ""),
    (LITERAL, 50, "{a: 1,\nb: <<-EOM\nbaz\nEOM\n}\n", &[], ""),
    (
        LITERAL,
        54,
        "{a: 1,\nb: {\na: <<-EOM\nbaz\nEOM\n}\n}\n\n",
        &[(7, 1)],
        "{a: 1,\nb: {\na: <<-EOM\nbaz\nEOM\n}}\n\n",
    ),
    (
        LITERAL,
        74,
        "{\na: 1, b: <<~EOM, c: 2}\n  baz\nEOM\n\n",
        &[],
        "",
    ),
    (
        LITERAL,
        74,
        "{\na: 1, b: <<~EOM, c: 2}\n  baz\nEOM\n\n",
        &[],
        "",
    ),
    (LITERAL, 91, "{a: 1,\nb: 2}\n", &[], ""),
    (LITERAL, 95, "{a: 1,\nb: [\n1\n]}\n", &[], ""),
    (
        LITERAL,
        99,
        "{a: 1,\nb: 2\n}\n\n",
        &[(3, 1)],
        "{a: 1,\nb: 2}\n\n",
    ),
    (
        LITERAL,
        109,
        "{a: 1, # a\nb: 2 # b\n}\n\n",
        &[(3, 1)],
        "{a: 1, # a\nb: 2} # b\n\n",
    ),
    (
        LITERAL,
        132,
        "{a: 1,\nb: 2 # comment b\n}\n.any?\n",
        &[(3, 1)],
        "",
    ),
    (
        LITERAL,
        147,
        "{a: 1,\nb: 2\n}\n.any?\n",
        &[(3, 1)],
        "{a: 1,\nb: 2}\n.any?\n",
    ),
    (LITERAL, 168, "{\na: 1,\nb: 2\n}\n", &[], ""),
    (LITERAL, 172, "{\na: 1,\nb: [\n1\n]\n}\n", &[], ""),
    (
        LITERAL,
        176,
        "{\na: 1,\nb: 2}\n",
        &[(3, 5)],
        "{\na: 1,\nb: 2\n}\n",
    ),
    (
        LITERAL,
        194,
        "{a: 1,\nb: 2\n}\n",
        &[(3, 1)],
        "{a: 1,\nb: 2}\n",
    ),
    (
        LITERAL,
        198,
        "{a: 1,\nb: [\n1\n]\n}\n",
        &[(5, 1)],
        "{a: 1,\nb: [\n1\n]}\n",
    ),
    (LITERAL, 202, "{a: 1,\nb: [\n1\n]}\n\n", &[], ""),
    (LITERAL, 211, "{a: 1, # a\nb: 2} # b\n\n", &[], ""),
    (LITERAL, 229, "{\na: 1,\nb: 2\n}\n", &[], ""),
    (LITERAL, 233, "{\na: 1,\nb: [\n1\n]\n}\n", &[], ""),
    (
        LITERAL,
        237,
        "{\na: 1,\nb: 2}\n\n",
        &[(3, 5)],
        "{\na: 1,\nb: 2\n}\n\n",
    ),
    (LITERAL, 257, "{a: 1,\nb: 2}\n", &[], ""),
    (LITERAL, 261, "{a: 1,\nb: [\n1\n]}\n", &[], ""),
    (
        LITERAL,
        265,
        "{a: 1,\nb: [\n1\n]\n}\n\n",
        &[(5, 1)],
        "{a: 1,\nb: [\n1\n]}\n\n",
    ),
    (
        LITERAL,
        275,
        "{a: 1, # a\nb: 2 # b\n}\n\n",
        &[(3, 1)],
        "{a: 1, # a\nb: 2} # b\n\n",
    ),
    (
        LITERAL,
        298,
        "{a: 1,\nb: 2 # comment b\n}\n.any?\n",
        &[(3, 1)],
        "",
    ),
    (
        LITERAL,
        313,
        "{a: 1,\nb: 2\n}\n.any?\n",
        &[(3, 1)],
        "{a: 1,\nb: 2}\n.any?\n",
    ),
    (
        LITERAL,
        334,
        "{\na: 1,\nb: 2}\n",
        &[(3, 5)],
        "{\na: 1,\nb: 2\n}\n",
    ),
    (
        LITERAL,
        338,
        "{\na: 1,\nb: [\n1\n]}\n",
        &[(5, 2)],
        "{\na: 1,\nb: [\n1\n]\n}\n",
    ),
    (LITERAL, 342, "{\na: 1,\nb: 2\n}\n\n", &[], ""),
    (
        ARGUMENT,
        13,
        "bar({a: 1,\nb: 2 # comment b\n}\n)\n",
        &[(3, 1)],
        "",
    ),
    (
        ARGUMENT,
        27,
        "bar({a: 1,\nb: 2\n}\n)\n",
        &[(3, 1)],
        "bar({a: 1,\nb: 2}\n)\n",
    ),
    (
        TRAILING_COMMA,
        14,
        "{a: 1, # a\nb: 2, # b\n}\n",
        &[(3, 1)],
        "{a: 1, # a\nb: 2,} # b\n",
    ),
    (
        TRAILING_COMMA,
        38,
        "{a: 1, # a\nb: 2, # b\n}\n",
        &[(3, 1)],
        "{a: 1, # a\nb: 2,} # b\n",
    ),
    (
        MAIN,
        63,
        "foo = {\n  bar: {\n    baz: 1 # comment\n  }\n}\n",
        &[],
        "",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 37);
    for &(group, line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "{group} line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineHashBraceLayout, source).is_empty(),
                "{group} line {line}"
            );
        } else if after.is_empty() {
            held(source);
        } else {
            assert_correction(&MultilineHashBraceLayout, source, after);
        }
    }
}

#[test]
fn messages_follow_the_opening_brace() {
    let (same, new) = PINNED_MESSAGES;
    assert_eq!(messages(same), [MESSAGES.same_line]);
    assert_eq!(messages(new), [MESSAGES.new_line]);
}

#[test]
fn metadata_is_fixed_and_default_on() {
    assert_eq!(MultilineHashBraceLayout.name(), COP);
    assert!(MultilineHashBraceLayout.default_enabled());
    assert!(MultilineHashBraceLayout.department_selection_enables());
    assert!(MultilineHashBraceLayout.safe_fixes());
    assert!(
        MultilineHashBraceLayout
            .autocorrect_incompatible_with()
            .is_empty()
    );
    let (same, _) = PINNED_MESSAGES;
    assert!(crate::pipeline::lint_str(same).iter().any(|o| o.cop == COP));
}

const PINNED_MESSAGES: (&str, &str) = ("{a: 1,\nb: 2\n}\n", "{\na: 1,\nb: 2}\n");

#[test]
fn every_braced_hash_is_checked() {
    for (source, expected) in [
        ("x = {a: 1,\n  b: 2\n}\n", vec![(3, 1)]),
        ("x = {\n  a: 1,\n  b: 2}\n", vec![(3, 7)]),
        ("x = {\"a\" => 1,\n  b => 2\n}\n", vec![(3, 1)]),
        ("x = {a:,\n  b:\n}\n", vec![(3, 1)]),
        ("x = {a: 1,\n  **b\n}\n", vec![(3, 1)]),
        ("x = {**a,\n  b: 1\n}\n", vec![(3, 1)]),
        ("foo({a: 1,\n  b: 2\n})\n", vec![(3, 1)]),
        ("foo(a: {x: 1,\n  y: 2\n})\n", vec![(3, 1)]),
        ("foo(**{x: 1,\n  y: 2\n})\n", vec![(3, 1)]),
        ("[{a: 1,\n  b: 2\n}]\n", vec![(3, 1)]),
        ("foo { {a: 1,\n  b: 2\n} }\n", vec![(3, 1)]),
        // A multi-line last value ends where its closing bracket is.
        ("{a: 1, b: [\n  1\n]\n}\n", vec![(4, 1)]),
        // Nested hashes are checked independently.
        ("{a: {b: 1,\n  c: 2\n}\n}\n", vec![(3, 1), (4, 1)]),
        // Character columns, not bytes.
        ("{\n  \"é\": 1, b: 2}\n", vec![(2, 15)]),
        // Symmetrical layouts.
        ("{a: 1,\n  b: 2}\n", vec![]),
        ("{\n  a: 1,\n  b: 2\n}\n", vec![]),
        ("{a: 1, b: {\n  c: 1\n}}\n", vec![]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn keyword_empty_single_line_and_pattern_lookalikes_are_ignored() {
    for source in [
        // Brace-less keyword hashes are the parser's implicit hashes.
        "foo(a: 1,\n  b: 2\n)\n",
        "foo a: 1,\n  b: 2\n",
        "[a: 1,\n  b: 2\n]\n",
        "{}\n",
        "{\n}\n",
        "{a: 1, b: 2}\n",
        // Patterns, blocks and arrays are not hash literals.
        "case x\nin {a: 1,\n  b: 2\n}\nend\n",
        "x => {a:,\n  b:\n}\n",
        "foo { |a,\n  b|\n}\n",
        "[a,\n  b\n]\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
}

#[test]
fn heredocs_reaching_the_last_line_or_opened_on_it_skip_the_literal() {
    for source in [
        "{a: 1,\n  b: <<~EOM\n  baz\nEOM\n}\n",
        "{a: 1,\n  b: foo(<<~EOM)\n  baz\nEOM\n}\n",
        "{a: 1,\n  **foo(<<~EOM)\n  baz\nEOM\n}\n",
        "{\n  a: 1, b: <<~EOM}\n  baz\nEOM\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
    for (source, expected) in [
        (
            "{a: 1,\n  b: {c: <<~EOM\n  baz\nEOM\n  }\n}\n",
            vec![(6, 1)],
        ),
        ("{a: 1,\n  b: <<~EOM, c: 2\n  baz\nEOM\n}\n", vec![(5, 1)]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn canonical_edits() {
    for (source, after) in [
        ("{a: 1,\n  b: 2\n}\n", "{a: 1,\n  b: 2}\n"),
        ("{a: 1,\n  b: 2,\n}\n", "{a: 1,\n  b: 2,}\n"),
        ("{a: 1,\n  **b\n}\n", "{a: 1,\n  **b}\n"),
        ("{a: 1,\n  b:\n}\n", "{a: 1,\n  b:}\n"),
        ("{a: 1,\n  b: 2\n  # c\n}\n", "{a: 1,\n  b: 2}\n  # c\n"),
        ("{a: 1, # a\n  b: 2 # b\n}\n", "{a: 1, # a\n  b: 2} # b\n"),
        (
            "x = [{a: 1, # a\n  b: 2 # b\n}, 3]\n",
            "x = [{a: 1, # a\n  b: 2}, 3] # b\n",
        ),
        ("{a: 1,\n  b: 2\n}.size\n", "{a: 1,\n  b: 2}.size\n"),
        (
            "foo(a: {x: 1,\n  y: 2 # y\n})\n",
            "foo(a: {x: 1,\n  y: 2}) # y\n",
        ),
        ("{\n  a: 1,\n  b: 2}\n", "{\n  a: 1,\n  b: 2\n}\n"),
        ("{a: 1,\n  b: :café\n}\n", "{a: 1,\n  b: :café}\n"),
        ("{a: 1,\r\n  b: 2\r\n}\r\n", "{a: 1,\r\n  b: 2}\r\n"),
        (
            "{\r\n  a: 1,\r\n  b: 2}\r\n",
            "{\r\n  a: 1,\r\n  b: 2\n}\r\n",
        ),
        (
            "{a: 1,\n  b: {c: <<~EOM\n  baz\nEOM\n  }\n}\n",
            "{a: 1,\n  b: {c: <<~EOM\n  baz\nEOM\n  }}\n",
        ),
    ] {
        assert_correction(&MultilineHashBraceLayout, source, after);
    }
    // A nested hash and its parent correct one per pass.
    let once = fixed("{a: {b: 1,\n  c: 2 # c\n}\n}\n");
    assert_eq!(once, "{a: {b: 1,\n  c: 2} # c\n}\n");
    assert_eq!(fixed(&once), "{a: {b: 1,\n  c: 2}} # c\n");
}

#[test]
fn holds_keep_the_finding_without_an_edit() {
    for source in [
        // Upstream: a comment after the last element of a receiver or a
        // `send` argument.
        "{a: 1,\n  b: 2 # b\n}.size\n",
        "foo({a: 1,\n  b: 2 # b\n})\n",
        // Pinned `same_line` context: a nested hash's bracket after a
        // comment line, replayed at symmetrical with an offending outer hash.
        "foo = {a: 1, bar: {\n    baz: 1 # comment\n  }\n}\n",
        // Unsafe moves.
        "{a: 1,\n  b: 2 \\\n}\n",
        "{a: 1,\n  b: <<~EOM, c: 2\n  baz\nEOM\n}\n",
        "{a: 1,\n  b: 2\n  # c\n}.size\n",
        "{a: 1, # a\n  b: 2 # b\n} # c\n",
        "x = [{a: 1, # a\n  b: 2 # b\n}, \"t\nu\"]\n",
        "x = {a: 1,\n  b: 2\n}\np __FILE__\n",
        "x = {\n  a: 1,\n  b: 2}\np $0\n",
    ] {
        held(source);
    }
}
