//! Examples ported from RuboCop's `spec/rubocop/cop/layout/condition_position_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! example of each `if`/`unless`/`while`/`until` iteration is replayed;
//! findings, messages and single-pass corrected bytes match the fixed replay
//! recorded in the pinned RuboCop reference run (source and corrected
//! SHA-256 checked against it when the examples were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str,
};

const COP: &str = "Layout/ConditionPosition";

type Reported = (u32, &'static str, &'static [(u32, u32)], &'static str);

fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    let mut found: Vec<_> = lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| (o.line, o.col, o.end_line, o.message))
        .collect();
    found.sort();
    found
}

fn positions(source: &str) -> Vec<(u32, u32)> {
    findings(source)
        .into_iter()
        .map(|(line, col, _, message)| {
            assert!(
                message.starts_with("Place the condition on the same line as `"),
                "{message}"
            );
            (line, col)
        })
        .collect()
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&ConditionPosition, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// The finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&ConditionPosition, source);
    assert_eq!(found.len(), 1, "{source}");
    assert!(found[0].correction.is_none(), "{source}");
}

/// Every pinned example without an offense at the fixed defaults.
const ACCEPTED: &[(u32, &str)] = &[
    (35, "if x == 10\n bala\nend\n"),
    (43, "do_something if\n  something && something_else\n"),
    (35, "unless x == 10\n bala\nend\n"),
    (43, "do_something unless\n  something && something_else\n"),
    (35, "while x == 10\n bala\nend\n"),
    (43, "do_something while\n  something && something_else\n"),
    (35, "until x == 10\n bala\nend\n"),
    (43, "do_something until\n  something && something_else\n"),
    (71, "x ? a : b"),
];

/// Every pinned example with offenses: (spec line, source, findings, single-pass correction).
const REPORTED: &[Reported] = &[
    (6, "if\nx == 10\nend\n", &[(2, 1)], "if x == 10\nend\n"),
    (
        20,
        "if\n  x == 10; do_something\nend\n",
        &[(2, 3)],
        "if x == 10\n  do_something\nend\n",
    ),
    (
        6,
        "unless\nx == 10\nend\n",
        &[(2, 1)],
        "unless x == 10\nend\n",
    ),
    (
        20,
        "unless\n  x == 10; do_something\nend\n",
        &[(2, 3)],
        "unless x == 10\n  do_something\nend\n",
    ),
    (
        6,
        "while\nx == 10\nend\n",
        &[(2, 1)],
        "while x == 10\nend\n",
    ),
    (
        20,
        "while\n  x == 10; do_something\nend\n",
        &[(2, 3)],
        "while x == 10\n  do_something\nend\n",
    ),
    (
        6,
        "until\nx == 10\nend\n",
        &[(2, 1)],
        "until x == 10\nend\n",
    ),
    (
        20,
        "until\n  x == 10; do_something\nend\n",
        &[(2, 3)],
        "until x == 10\n  do_something\nend\n",
    ),
    (
        51,
        "if something\n  test\nelsif\n  something\n  test\nend\n",
        &[(4, 3)],
        "if something\n  test\nelsif something\n  test\nend\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for (line, source) in ACCEPTED {
        assert_eq!(positions(source), [], "spec line {line}");
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (line, source, expected, after) in REPORTED {
        assert_eq!(positions(source), *expected, "spec line {line}");
        assert_correction(&ConditionPosition, source, after);
    }
}

#[test]
fn messages_name_the_keyword_and_span_the_condition() {
    for (source, keyword) in [
        ("if\n  a\nend\n", "if"),
        ("unless\n  a\nend\n", "unless"),
        ("if b\nelsif\n  a\nend\n", "elsif"),
        ("while\n  a\nend\n", "while"),
        ("until\n  a\nend\n", "until"),
    ] {
        let found = findings(source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!(
            found[0].3,
            format!("Place the condition on the same line as `{keyword}`.")
        );
    }
    assert_eq!(
        findings("if\n  a &&\n    b\n  c\nend\n"),
        [(
            2,
            3,
            3,
            "Place the condition on the same line as `if`.".to_owned()
        )]
    );
}

#[test]
fn metadata_is_fixed() {
    assert_eq!(ConditionPosition.name(), COP);
    assert!(ConditionPosition.default_enabled());
    assert!(ConditionPosition.safe_fixes());
    assert!(ConditionPosition.autocorrect_incompatible_with().is_empty());
}

#[test]
fn modifiers_ternaries_guards_and_same_line_conditions_are_accepted() {
    for source in [
        "a if\n  b\n",
        "a unless\n  b\n",
        "a while\n  b\n",
        "a until\n  b\n",
        "begin\n  a\nend while\n  b\n",
        "begin\n  a\nend until\n  b\n",
        "x ?\n  a :\n  b\n",
        "case x\nin y if\n  z\nend\n",
        "case x\nin y unless\n  z\nend\n",
        "if (\n  a)\n  b\nend\n",
        "if a &&\n  b\n  c\nend\n",
        "while a\nend\n",
        "if a then b end\n",
        "if a\nelsif b\nend\n",
        "for x in\n  y\nend\n",
        "case\n  x\nwhen 1\nend\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
}

#[test]
fn corrections_keep_separators_comments_and_branches() {
    for (source, after) in [
        // Multi-line conditions move whole.
        (
            "if\n  a &&\n    b\n  c\nend\n",
            "if a &&\n    b\n  c\nend\n",
        ),
        // `then`, `do` and `;` after the condition are dropped with it.
        ("if\n  a then\n  b\nend\n", "if a\n  b\nend\n"),
        ("while\n  a do\n  b\nend\n", "while a\n  b\nend\n"),
        ("until\n  a;\n  b\nend\n", "until a\n  b\nend\n"),
        // A body on the condition line stays, behind its separator.
        ("if\n  a then b end\n", "if a\n  b end\n"),
        ("while\n  a do b end\n", "while a\n  b end\n"),
        ("if\n  a then b else c end\n", "if a\n  b else c end\n"),
        // Comments and blank lines between the keyword and the condition
        // stay where they are.
        ("if # note\n  a\n  b\nend\n", "if a # note\n  b\nend\n"),
        (
            "if\n  # note\n\n  a\n  b\nend\n",
            "if a\n  # note\n\n  b\nend\n",
        ),
        // `unless` bodies and `else` branches are kept.
        (
            "unless\n  a; b\nelse\n  c\nend\n",
            "unless a\n  b\nelse\n  c\nend\n",
        ),
        (
            "if x\n  y\nelsif\n  a; b\nelse\n  c\nend\n",
            "if x\n  y\nelsif a\n  b\nelse\n  c\nend\n",
        ),
        // CRLF and a nested conditional in one pass.
        ("if\r\n  a\r\n  b\r\nend\r\n", "if a\r\n  b\r\nend\r\n"),
        (
            "if\n  a\n  while\n    b\n    c\n  end\nend\n",
            "if a\n  while b\n    c\n  end\nend\n",
        ),
    ] {
        assert!(!positions(source).is_empty(), "{source:?}");
        assert_eq!(fixed(source), after, "{source:?}");
    }
}

#[test]
fn edits_that_would_drop_or_join_source_are_held() {
    for source in [
        // A comment after the condition would be deleted with its line.
        "if\n  a # note\n  b\nend\n",
        // Another token after the condition would be deleted.
        "if\n  a then else b end\n",
        "if\n  a; else b end\n",
        // A continuation would join the moved condition with the next line.
        "if \\\n  a\n  b\nend\n",
        // A heredoc opener would leave its body behind.
        "if\n  a == <<~A\n    x\n  A\n  b\nend\n",
    ] {
        held(source);
    }
}

#[test]
fn columns_count_characters_and_skip_the_bom() {
    assert_eq!(positions("\u{feff}if\n  a\nend\n"), [(2, 3)]);
    assert_eq!(positions("if\n  ｒ\nend\n"), [(2, 3)]);
    assert_eq!(fixed("\u{feff}if\n  a\nend\n"), "\u{feff}if a\nend\n");
}

#[test]
fn findings_only_without_requested_corrections() {
    let offenses: Vec<_> = lint_str("if\n  a\nend\n")
        .into_iter()
        .filter(|o| o.cop == COP)
        .collect();
    assert_eq!(offenses.len(), 1);
    assert!(offenses[0].correction.is_none());
}
