//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_array_brace_layout_spec.rb` and the shared
//! `spec/support/multiline_literal_brace_layout_examples.rb`,
//! `multiline_literal_brace_layout_method_argument_examples.rb` and
//! `multiline_literal_brace_layout_trailing_comma_examples.rb` it includes,
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 38
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

const COP: &str = "Layout/MultilineArrayBraceLayout";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineArrayBraceLayout], source)
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
    lint_str_with(&[&MultilineArrayBraceLayout], source)
        .into_iter()
        .map(|o| o.message)
        .collect()
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&MultilineArrayBraceLayout, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&MultilineArrayBraceLayout, source);
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
    (MAIN, 7, "foo = a,\nb\n", &[], ""),
    (MAIN, 14, "[a, b, c]", &[], ""),
    (MAIN, 18, "[]", &[], ""),
    (LITERAL, 50, "[a,\n<<-EOM\nbaz\nEOM\n]\n", &[], ""),
    (
        LITERAL,
        54,
        "[a,\n{\na: <<-EOM\nbaz\nEOM\n}\n]\n\n",
        &[(7, 1)],
        "[a,\n{\na: <<-EOM\nbaz\nEOM\n}]\n\n",
    ),
    (LITERAL, 74, "[\na, <<~EOM, b]\n  baz\nEOM\n\n", &[], ""),
    (LITERAL, 74, "[\na, <<~EOM, b]\n  baz\nEOM\n\n", &[], ""),
    (LITERAL, 91, "[a,\nb]\n", &[], ""),
    (LITERAL, 95, "[a,\n{\nfoo: bar\n}]\n", &[], ""),
    (LITERAL, 99, "[a,\nb\n]\n\n", &[(3, 1)], "[a,\nb]\n\n"),
    (
        LITERAL,
        109,
        "[a, # a\nb # b\n]\n\n",
        &[(3, 1)],
        "[a, # a\nb] # b\n\n",
    ),
    (
        LITERAL,
        132,
        "[a,\nb # comment b\n]\n.any?\n",
        &[(3, 1)],
        "",
    ),
    (
        LITERAL,
        147,
        "[a,\nb\n]\n.any?\n",
        &[(3, 1)],
        "[a,\nb]\n.any?\n",
    ),
    (LITERAL, 168, "[\na,\nb\n]\n", &[], ""),
    (LITERAL, 172, "[\na,\n{\nfoo: bar\n}\n]\n", &[], ""),
    (LITERAL, 176, "[\na,\nb]\n", &[(3, 2)], "[\na,\nb\n]\n"),
    (LITERAL, 194, "[a,\nb\n]\n", &[(3, 1)], "[a,\nb]\n"),
    (
        LITERAL,
        198,
        "[a,\n{\nfoo: bar\n}\n]\n",
        &[(5, 1)],
        "[a,\n{\nfoo: bar\n}]\n",
    ),
    (LITERAL, 202, "[a,\n{\nfoo: bar\n}]\n\n", &[], ""),
    (LITERAL, 211, "[a, # a\nb] # b\n\n", &[], ""),
    (LITERAL, 229, "[\na,\nb\n]\n", &[], ""),
    (LITERAL, 233, "[\na,\n{\nfoo: bar\n}\n]\n", &[], ""),
    (LITERAL, 237, "[\na,\nb]\n\n", &[(3, 2)], "[\na,\nb\n]\n\n"),
    (LITERAL, 257, "[a,\nb]\n", &[], ""),
    (LITERAL, 261, "[a,\n{\nfoo: bar\n}]\n", &[], ""),
    (
        LITERAL,
        265,
        "[a,\n{\nfoo: bar\n}\n]\n\n",
        &[(5, 1)],
        "[a,\n{\nfoo: bar\n}]\n\n",
    ),
    (
        LITERAL,
        275,
        "[a, # a\nb # b\n]\n\n",
        &[(3, 1)],
        "[a, # a\nb] # b\n\n",
    ),
    (
        LITERAL,
        298,
        "[a,\nb # comment b\n]\n.any?\n",
        &[(3, 1)],
        "",
    ),
    (
        LITERAL,
        313,
        "[a,\nb\n]\n.any?\n",
        &[(3, 1)],
        "[a,\nb]\n.any?\n",
    ),
    (LITERAL, 334, "[\na,\nb]\n", &[(3, 2)], "[\na,\nb\n]\n"),
    (
        LITERAL,
        338,
        "[\na,\n{\nfoo: bar\n}]\n",
        &[(5, 2)],
        "[\na,\n{\nfoo: bar\n}\n]\n",
    ),
    (LITERAL, 342, "[\na,\nb\n]\n\n", &[], ""),
    (
        ARGUMENT,
        13,
        "bar([a: 1,\nb: 2 # comment b\n]\n)\n",
        &[(3, 1)],
        "",
    ),
    (
        ARGUMENT,
        27,
        "bar([a: 1,\nb: 2\n]\n)\n",
        &[(3, 1)],
        "bar([a: 1,\nb: 2]\n)\n",
    ),
    (
        TRAILING_COMMA,
        14,
        "[a, # a\nb, # b\n]\n",
        &[(3, 1)],
        "[a, # a\nb,] # b\n",
    ),
    (
        TRAILING_COMMA,
        38,
        "[a, # a\nb, # b\n]\n",
        &[(3, 1)],
        "[a, # a\nb,] # b\n",
    ),
    (
        MAIN,
        49,
        "{\n  key1: [a, # comment 1\n        b # comment 2\n  ],\n  key2: 'foo'\n}\n",
        &[(4, 3)],
        "{\n  key1: [a, # comment 1\n        b], # comment 2\n  key2: 'foo'\n}\n",
    ),
    (
        MAIN,
        73,
        "foo = [\n  [\n    1,\n    # comment\n  ]\n]\n",
        &[],
        "",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 38);
    for &(group, line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "{group} line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineArrayBraceLayout, source).is_empty(),
                "{group} line {line}"
            );
        } else if after.is_empty() {
            held(source);
        } else {
            assert_correction(&MultilineArrayBraceLayout, source, after);
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
    assert_eq!(MultilineArrayBraceLayout.name(), COP);
    assert!(MultilineArrayBraceLayout.default_enabled());
    assert!(MultilineArrayBraceLayout.department_selection_enables());
    assert!(MultilineArrayBraceLayout.safe_fixes());
    assert!(
        MultilineArrayBraceLayout
            .autocorrect_incompatible_with()
            .is_empty()
    );
    let (same, _) = PINNED_MESSAGES;
    assert!(crate::pipeline::lint_str(same).iter().any(|o| o.cop == COP));
}

const PINNED_MESSAGES: (&str, &str) = ("[a,\nb\n]\n", "[\na,\nb]\n");

#[test]
fn every_bracketed_array_is_checked() {
    for (source, expected) in [
        ("x = [1,\n  2\n]\n", vec![(3, 1)]),
        ("x = [\n  1,\n  2]\n", vec![(3, 4)]),
        ("%w[a\n  b\n]\n", vec![(3, 1)]),
        ("%i(a\n  b\n)\n", vec![(3, 1)]),
        ("%W[#{a}\n  b\n]\n", vec![(3, 1)]),
        ("%I[\n  a\n  b]\n", vec![(3, 4)]),
        ("[*a,\n  *b\n]\n", vec![(3, 1)]),
        // A brace-less hash is one element.
        ("[a,\n  b: 1\n]\n", vec![(3, 1)]),
        ("a, b = [1,\n  2\n]\n", vec![(3, 1)]),
        ("foo([1,\n  2\n])\n", vec![(3, 1)]),
        ("foo { [1,\n  2\n] }\n", vec![(3, 1)]),
        ("\"#{[1,\n  2\n]}\"\n", vec![(3, 1)]),
        // A multi-line last element ends where its closing bracket is.
        ("[a, foo(\n  1\n)\n]\n", vec![(4, 1)]),
        ("[\n  a, foo(\n  1\n)]\n", vec![(4, 2)]),
        // Nested arrays are checked independently.
        ("[[1,\n  2\n]\n]\n", vec![(3, 1), (4, 1)]),
        // Character columns, not bytes.
        ("[\n  \"é\", 1]\n", vec![(2, 9)]),
        // Symmetrical layouts.
        ("[a,\n  b]\n", vec![]),
        ("[\n  a,\n  b\n]\n", vec![]),
        ("[a, [\n  1\n]]\n", vec![]),
        ("[\n  a, [\n    1\n  ]\n]\n", vec![]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn implicit_empty_single_line_and_non_array_lookalikes_are_ignored() {
    for source in [
        "x = 1,\n  2\n",
        "a, b = 1,\n  2\n",
        "begin\nrescue A,\n  B\nend\n",
        "[]\n",
        "[\n]\n",
        "%w[\n]\n",
        "[a, b]\n",
        "%w[a b]\n",
        // Index arguments, patterns and parameters are not array literals.
        "foo[1,\n  2\n]\n",
        "foo[1,\n  2\n] = 3\n",
        "case x\nin [1,\n  2\n]\nend\n",
        "x => [a,\n  b\n]\n",
        "def foo(a,\n  b\n)\nend\n",
        "foo(a,\n  b\n)\n",
        "{a: 1,\n  b: 2\n}\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
}

#[test]
fn heredocs_reaching_the_last_line_or_opened_on_it_skip_the_literal() {
    for source in [
        // The last element's heredoc ends after its (opener) line.
        "[a,\n  <<~EOM\n  baz\nEOM\n]\n",
        "[a,\n  foo(<<~EOM)\n  baz\nEOM\n]\n",
        "[a,\n  <<~EOM.strip\n  baz\nEOM\n]\n",
        "[a,\n  `#{<<~EOM}`\n  baz\nEOM\n]\n",
        "[a,\n  <<~`EOM`\n  ls\nEOM\n]\n",
        // A heredoc opened on the closing bracket's line.
        "[\n  a, <<~EOM]\n  baz\nEOM\n",
        "[\n  a, foo(<<~EOM), b]\n  baz\nEOM\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
    // A heredoc in an earlier element, or one ending before the last line.
    for (source, expected) in [
        ("[a,\n  <<~EOM, b\n  baz\nEOM\n]\n", vec![(5, 1)]),
        ("[a,\n  [<<~EOM\n  baz\nEOM\n  ]\n]\n", vec![(6, 1)]),
        ("[a, <<~EOM,\n  baz\nEOM\n  b\n]\n", vec![(5, 1)]),
        // `<<` that is not a heredoc.
        ("[a,\n  b << c\n]\n", vec![(3, 1)]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn canonical_edits() {
    for (source, after) in [
        // The closing bracket moves to the last element, blanks dropped.
        ("[a,\n  b\n]\n", "[a,\n  b]\n"),
        ("[a,\n  b  \n\n\n  ]\n", "[a,\n  b]  \n"),
        ("[a,\n  b\n  \n]\n", "[a,\n  b]\n  \n"),
        ("x = [a,\n  b\n].size\n", "x = [a,\n  b].size\n"),
        ("[a,\n  b,\n]\n", "[a,\n  b,]\n"),
        // An own-line comment before the bracket stays where it is.
        ("[a,\n  b\n  # c\n]\n", "[a,\n  b]\n  # c\n"),
        (
            "[a,\n  b\n=begin\nc\n=end\n]\n",
            "[a,\n  b]\n=begin\nc\n=end\n",
        ),
        // A comment after the last element: the rest of the bracket's line
        // moves before it.
        ("[a, # a\n  b # b\n]\n", "[a, # a\n  b] # b\n"),
        (
            "x = { k: [a, # a\n  b # b\n], j: 1 }\n",
            "x = { k: [a, # a\n  b], j: 1 } # b\n",
        ),
        ("[a,\n  b, # b\n  # c\n]\n", "[a,\n  b,] # b\n  # c\n"),
        // Receivers and arguments without a comment.
        ("foo([a,\n  b\n])\n", "foo([a,\n  b])\n"),
        ("[a,\n  b\n]&.size\n", "[a,\n  b]&.size\n"),
        // Not `send` arguments: `csend`, `super`, `yield`, index arguments.
        ("foo&.bar([a,\n  b # b\n])\n", "foo&.bar([a,\n  b]) # b\n"),
        ("super([a,\n  b # b\n])\n", "super([a,\n  b]) # b\n"),
        ("x[[a,\n  b # b\n]]\n", "x[[a,\n  b]] # b\n"),
        ("[a,\n  b # b\n][0]\n", "[a,\n  b][0] # b\n"),
        // A line break inserted before a bracket on the last element's line.
        ("[\n  a,\n  b]\n", "[\n  a,\n  b\n]\n"),
        ("[\n  a,\n  b] # c\n", "[\n  a,\n  b\n] # c\n"),
        ("%w[\n  a\n  b]\n", "%w[\n  a\n  b\n]\n"),
        // Multibyte last elements and percent words.
        ("[a,\n  :café\n]\n", "[a,\n  :café]\n"),
        ("%w[a\n  é\n]\n", "%w[a\n  é]\n"),
        // CRLF: `\r\n` is one break; the inserted break is upstream's `\n`.
        ("[a,\r\n  b\r\n]\r\n", "[a,\r\n  b]\r\n"),
        ("[a, # a\r\n  b # b\r\n]\r\n", "[a, # a\r\n  b] # b\r\n"),
        ("[\r\n  a,\r\n  b]\r\n", "[\r\n  a,\r\n  b\n]\r\n"),
        // A heredoc inside the last element that ends before its last line.
        (
            "[a,\n  [<<~EOM\n  baz\nEOM\n  ]\n]\n",
            "[a,\n  [<<~EOM\n  baz\nEOM\n  ]]\n",
        ),
    ] {
        assert_correction(&MultilineArrayBraceLayout, source, after);
    }
}

#[test]
fn nested_brackets_correct_one_per_pass() {
    // The outer edit starts at the inner bracket, which the inner edit
    // replaces: one pass applies the inner one, the next run the outer.
    let source = "[[1,\n  2\n]\n]\n";
    let once = fixed(source);
    assert_eq!(once, "[[1,\n  2]\n]\n");
    assert_eq!(fixed(&once), "[[1,\n  2]]\n");
    let source = "[a, [b,\n  c # c\n]\n]\n";
    let once = fixed(source);
    assert_eq!(once, "[a, [b,\n  c] # c\n]\n");
    assert_eq!(fixed(&once), "[a, [b,\n  c]] # c\n");
}

#[test]
fn upstream_holds_keep_the_finding_without_an_edit() {
    for source in [
        // A comment after the last element of a receiver or `send` argument.
        "[a,\n  b # b\n].size\n",
        "foo([a,\n  b # b\n])\n",
        "foo [a,\n  b # b\n]\n",
        "x + [a,\n  b # b\n]\n",
        "a.b = [a,\n  b, # b\n]\n",
    ] {
        held(source);
    }
    // A nested bracket first on the last line after a comment line holds
    // the outer edit; the inner array is corrected on its own.
    for (source, line) in [("[a, [b,\n  # c\n]\n]\n", 4), ("[a, [b, # c\n]\n]\n", 3)] {
        let found = correction_offenses(&MultilineArrayBraceLayout, source);
        let outer = found.iter().find(|o| o.line == line).unwrap();
        assert!(outer.correction.is_none(), "{source:?}");
    }
    // The nested bracket hangs from a `send`: only an own-line comment holds.
    assert_correction(
        &MultilineArrayBraceLayout,
        "[a, foo(1, [b, # c\n])\n]\n",
        "[a, foo(1, [b, # c\n])]\n",
    );
}

#[test]
fn unsafe_moves_keep_the_finding_without_an_edit() {
    for source in [
        // A line continuation or a heredoc body between element and bracket.
        "[a,\n  b \\\n]\n",
        "[a,\n  <<~EOM, b\n  baz\nEOM\n]\n",
        // A comment before the bracket would swallow the rest of its line.
        "[a,\n  b\n  # c\n].size\n",
        "x = [a,\n  b\n  # c\n], 1\n",
        // The moved rest of the line holds a comment or a heredoc opener, or
        // ends inside a string.
        "[a, # a\n  b # b\n] # c\n",
        "x = [a, # a\n  b # b\n], <<~EOM\n  baz\nEOM\n",
        "x = { k: [a, # a\n  b # b\n], s: \"t\nu\" }\n",
        // Listed source observers.
        "x = [a,\n  b\n]\np __FILE__\n",
        "x = [\n  a,\n  b]\np __dir__\n",
        "x = [a,\n  b\n]\np caller\n",
    ] {
        held(source);
    }
}
