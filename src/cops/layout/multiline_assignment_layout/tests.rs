//! Examples ported from RuboCop's `spec/rubocop/cop/layout/multiline_assignment_layout_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The cop
//! is opt-in; every example, including the `same_line` context and those
//! configuring `SupportedTypes`, is replayed at the fixed `new_line` style and
//! the default supported types (`block`, `case`, `class`, `if`, `kwbegin`,
//! `module`). Findings, messages and single-pass corrected bytes match the
//! fixed replay recorded in the pinned RuboCop reference run (source and
//! corrected SHA-256 checked against it when the examples were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/MultilineAssignmentLayout";

type Reported = (u32, &'static str, &'static [(u32, u32)], &'static str);

fn findings(source: &str) -> Vec<(u32, u32, u32, String)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineAssignmentLayout], source)
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
            assert_eq!(message, MSG);
            (line, col)
        })
        .collect()
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&MultilineAssignmentLayout, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every pinned example without an offense at the fixed defaults.
const ACCEPTED: &[(u32, &str)] = &[
    (54, "a, b = 4,\n5\n"),
    (64, "a, b = 4,\n5\n"),
    (79, "blarg=\nif true\nend\n"),
    (123, "lambda =\n  -> {\n    puts 'hello'\n  }\n"),
    (132, "default_scope -> {\n  where(foo: \"bar\")\n}\n"),
    (140, "foo << items.map do |item|\n  \"#{item}!\"\nend\n"),
    (162, "foo =\n  items.map { |item| item.do_something }\n"),
    (
        187,
        "foo =\n  items.map do |item|\n    item.do_something\n  end\n",
    ),
    (233, "blarg =\nif true\nend\n"),
    (247, "hash[:foo] =\nif true\nend\n"),
    (261, "foo.bar =\nif true\nend\n"),
    (275, "a, b =\n4,\n5\n"),
    (286, "a, b =\n4,\n5\n"),
    (308, "a,\nb =\nif foo\nend\n"),
    (336, "lambda =\n  -> {\n    puts 'hello'\n  }\n"),
    (352, "default_scope -> {\n  where(foo: \"bar\")\n}\n"),
    (360, "foo << items.map do |item|\n  \"#{item}!\"\nend\n"),
    (375, "foo =\n  items.map { |item| item.do_something }\n"),
    (
        396,
        "foo =\n  items.map do |item|\n    item.do_something\n  end\n",
    ),
    (412, "bars =\n  foo.map {\n    _1.bar\n  }\n"),
    (428, "bars =\n  foo.map {\n    it.bar\n  }\n"),
];

/// Every pinned example with offenses: (spec line, source, findings, single-pass correction).
const REPORTED: &[Reported] = &[
    (
        12,
        "blarg = if true\nend\n",
        &[(1, 1)],
        "blarg =\n if true\nend\n",
    ),
    (
        26,
        "hash[:foo] = if true\nend\n",
        &[(1, 1)],
        "hash[:foo] =\n if true\nend\n",
    ),
    (
        40,
        "foo.bar = if true\nend\n",
        &[(1, 1)],
        "foo.bar =\n if true\nend\n",
    ),
    (
        87,
        "a,\nb = if foo\nend\n",
        &[(1, 1)],
        "a,\nb =\n if foo\nend\n",
    ),
    (
        107,
        "lambda = -> {\n  puts 'hello'\n}\n",
        &[(1, 1)],
        "lambda =\n -> {\n  puts 'hello'\n}\n",
    ),
    (
        148,
        "foo = items\n  .map { |item| item.do_something }\n",
        &[(1, 1)],
        "foo =\n items\n  .map { |item| item.do_something }\n",
    ),
    (
        169,
        "foo = items\n  .map do |item|\n    item.do_something\n  end\n",
        &[(1, 1)],
        "foo =\n items\n  .map do |item|\n    item.do_something\n  end\n",
    ),
    (
        196,
        "bars = foo.map {\n  _1.bar\n}\n",
        &[(1, 1)],
        "bars =\n foo.map {\n  _1.bar\n}\n",
    ),
    (
        212,
        "bars = foo.map {\n  it.bar\n}\n",
        &[(1, 1)],
        "bars =\n foo.map {\n  it.bar\n}\n",
    ),
    (
        301,
        "blarg= if true\nend\n",
        &[(1, 1)],
        "blarg=\n if true\nend\n",
    ),
    (
        327,
        "lambda = -> {\n  puts 'hello'\n}\n",
        &[(1, 1)],
        "lambda =\n -> {\n  puts 'hello'\n}\n",
    ),
    (
        368,
        "foo = items\n  .map { |item| item.do_something }\n",
        &[(1, 1)],
        "foo =\n items\n  .map { |item| item.do_something }\n",
    ),
    (
        387,
        "foo = items\n  .map do |item|\n    item.do_something\n  end\n",
        &[(1, 1)],
        "foo =\n items\n  .map do |item|\n    item.do_something\n  end\n",
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
        assert_correction(&MultilineAssignmentLayout, source, after);
    }
}

#[test]
fn metadata_is_fixed_and_opt_in() {
    assert_eq!(MultilineAssignmentLayout.name(), COP);
    assert!(!MultilineAssignmentLayout.default_enabled());
    assert!(MultilineAssignmentLayout.safe_fixes());
    assert!(
        MultilineAssignmentLayout
            .autocorrect_incompatible_with()
            .is_empty()
    );
    // Not run by the default selection.
    assert!(
        crate::pipeline::lint_str("x = if a\n  b\nend\n")
            .iter()
            .all(|o| o.cop != COP)
    );
}

#[test]
fn every_parser_assignment_shape_is_checked() {
    let rhs = "if a\n  b\nend\n";
    for (lhs, operator) in [
        ("x", "="),
        ("@x", "="),
        ("@@x", "="),
        ("$x", "="),
        ("X", "="),
        ("A::X", "="),
        ("::X", "="),
        ("x, y", "="),
        ("*x", "="),
        ("x", "+="),
        ("@x", "-="),
        ("@@x", "*="),
        ("$x", "<<="),
        ("X", "|="),
        ("A::X", "&="),
        ("x.y", "+="),
        ("x[1]", "+="),
        ("x", "||="),
        ("@x", "||="),
        ("@@x", "||="),
        ("$x", "||="),
        ("X", "||="),
        ("A::X", "||="),
        ("x.y", "||="),
        ("x&.y", "||="),
        ("x[1]", "||="),
        ("x", "&&="),
        ("@x", "&&="),
        ("@@x", "&&="),
        ("$x", "&&="),
        ("X", "&&="),
        ("A::X", "&&="),
        ("x.y", "&&="),
        ("x[1]", "&&="),
        ("x.y", "="),
        ("x[1]", "="),
        ("x[1, 2]", "="),
        ("self.y", "="),
    ] {
        let source = format!("{lhs} {operator} {rhs}");
        assert_eq!(positions(&source), [(1, 1)], "{source}");
        assert_eq!(
            fixed(&source),
            format!("{lhs} {operator}\n {rhs}"),
            "{source}"
        );
    }
}

#[test]
fn unchecked_assignment_lookalikes() {
    for source in [
        // `csend` writes are not `send` nodes.
        "x&.y = if a\n  b\nend\n",
        // Calls whose last argument is a supported value but no `=`.
        "foo(if a\n  b\nend)\n",
        "x.y(if a\n  b\nend)\n",
        "x.send(:y=, if a\n  b\nend)\n",
        "x << if a\n  b\nend\n",
        "x == if a\n  b\nend\n",
        // Keyword arguments, defaults and hash values are not assignments.
        "foo(a: if b\n  c\nend)\n",
        "def f(a = if b\n  c\nend); end\n",
        "{ a: if b\n  c\nend }\n",
        // Modifier conditionals wrap the assignment.
        "x = a if b\n",
        // Nested assignments check only the one holding the value.
        "rescue_target = nil\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
    assert_eq!(positions("x = y = if a\n  b\nend\n"), [(1, 5)]);
}

#[test]
fn only_the_default_supported_types() {
    for rhs in [
        "if a\n  b\nend",
        "unless a\n  b\nend",
        "a ?\n  b : c",
        "case a\nwhen b\nend",
        "class Foo\nend",
        "module Foo\nend",
        "begin\n  a\nend",
        "begin\n  a\nrescue\n  b\nend",
        "foo do\nend",
        "foo {\n}",
        "foo.map { _1\n}",
        "foo.map { it\n}",
        "-> {\n}",
        "lambda do\nend",
        "super do\nend",
        "super(1) {\n}",
        "items\n  .map { |item| item }",
    ] {
        let source = format!("x = {rhs}\n");
        assert_eq!(positions(&source), [(1, 1)], "{source}");
    }
    for rhs in [
        "[1,\n  2]",
        "{ a: 1,\n  b: 2 }",
        "foo(1,\n  2)",
        "foo(&blk\n)",
        "case a\nin b\nend",
        "class << self\nend",
        "while a\nend",
        "until a\nend",
        "for a in b\nend",
        "(a\n)",
        "\"a\n\"",
        "<<~A\n  a\nA",
        "a &&\n  b",
        "def f\nend",
        "foo.map(&:bar\n)",
    ] {
        let source = format!("x = {rhs}\n");
        assert_eq!(positions(&source), [], "{source}");
    }
}

#[test]
fn single_line_values_and_block_openings() {
    for source in [
        "x = if a then b end\n",
        "x = foo { }\n",
        "x = begin; a; end\n",
        "x = case a when b then c end\n",
        "x = a ? b : c\n",
        // The block opens on the assignment's line.
        "x = foo.map { |a| a }\n",
        // The value starts on the operator's next line.
        "x =\n  if a\n    b\n  end\n",
        "x =\n  foo do\n  end\n",
    ] {
        assert_eq!(positions(source), [], "{source}");
    }
    // A single-line block opening on a later line than the assignment.
    assert_eq!(positions("x = foo\n  .map { |a| a }\n"), [(1, 1)]);
    assert_eq!(positions("x = foo.\n  map { |a| a }\n"), [(1, 1)]);
    // A multi-line left-hand side starts the finding.
    assert_eq!(findings("a,\nb = if c\nend\n"), [(1, 1, 3, MSG.to_owned())]);
}

#[test]
fn corrections_insert_one_newline_and_keep_every_byte() {
    for (source, after) in [
        ("x=if a\n  b\nend\n", "x=\nif a\n  b\nend\n"),
        ("x =  if a # c\n  b\nend\n", "x =\n  if a # c\n  b\nend\n"),
        ("x = if a\r\n  b\r\nend\r\n", "x =\n if a\r\n  b\r\nend\r\n"),
        (
            "\u{feff}x = if a\n  b\nend\n",
            "\u{feff}x =\n if a\n  b\nend\n",
        ),
        (
            "x = foo(<<~A) do\n  a\nA\nend\n",
            "x =\n foo(<<~A) do\n  a\nA\nend\n",
        ),
    ] {
        assert_eq!(fixed(source), after, "{source:?}");
    }
}

#[test]
fn findings_only_without_requested_corrections() {
    let offenses = lint_str_with(&[&MultilineAssignmentLayout], "x = if a\n  b\nend\n");
    assert_eq!(offenses.len(), 1);
    assert!(offenses[0].correction.is_none());
}
