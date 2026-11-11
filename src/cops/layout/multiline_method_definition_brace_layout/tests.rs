//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_method_definition_brace_layout_spec.rb`
//! and the shared `spec/support/multiline_literal_brace_layout_examples.rb` it
//! includes (with `prefix` `def foo`, `suffix` `end`, `multi_prefix` `b: ` and
//! `after_heredoc` `c: 1`), at 97f30c4 (MIT License, Copyright (c) 2012-26
//! Bozhidar Batsov). All 28 pinned examples run at the fixed `EnforcedStyle:
//! symmetrical` (the shared chained-call contexts are excluded upstream for
//! this cop); the `new_line` and `same_line` contexts are replayed at it, so
//! their findings follow the symmetrical rule. Findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected SHA-256
//! was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::{lint_str_with, lint_str_with_corrections},
};

const COP: &str = "Layout/MultilineMethodDefinitionBraceLayout";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineMethodDefinitionBraceLayout], source)
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
    lint_str_with(&[&MultilineMethodDefinitionBraceLayout], source)
        .into_iter()
        .map(|o| o.message)
        .collect()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&MultilineMethodDefinitionBraceLayout, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

const MAIN: &str = "main";
const LITERAL: &str = "multiline literal brace layout";

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
    (MAIN, 7, "def foo a: 1,\nb: 2\nend\n", &[], ""),
    (MAIN, 15, "def foo(a,b)\nend\n", &[], ""),
    (MAIN, 22, "def foo\nend\n", &[], ""),
    (
        LITERAL,
        50,
        "def foo(a,\nb: <<-EOM\nbaz\nEOM\n)\nend",
        &[],
        "",
    ),
    (
        LITERAL,
        54,
        "def foo(a,\nb: {\na: <<-EOM\nbaz\nEOM\n}\n)\nend\n",
        &[(7, 1)],
        "def foo(a,\nb: {\na: <<-EOM\nbaz\nEOM\n})\nend\n",
    ),
    (
        LITERAL,
        74,
        "def foo(\na, b: <<~EOM, c: 1)\n  baz\nEOM\nend\n",
        &[],
        "",
    ),
    (
        LITERAL,
        74,
        "def foo(\na, b: <<~EOM, c: 1)\n  baz\nEOM\nend\n",
        &[],
        "",
    ),
    (LITERAL, 91, "def foo(a,\nb)\nend", &[], ""),
    (LITERAL, 95, "def foo(a,\nb: {\nfoo: bar\n})\nend", &[], ""),
    (
        LITERAL,
        99,
        "def foo(a,\nb\n)\nend\n",
        &[(3, 1)],
        "def foo(a,\nb)\nend\n",
    ),
    (
        LITERAL,
        109,
        "def foo(a, # a\nb # b\n)\nend\n",
        &[(3, 1)],
        "def foo(a, # a\nb) # b\nend\n",
    ),
    (LITERAL, 168, "def foo(\na,\nb\n)\nend", &[], ""),
    (
        LITERAL,
        172,
        "def foo(\na,\nb: {\nfoo: bar\n}\n)\nend",
        &[],
        "",
    ),
    (
        LITERAL,
        176,
        "def foo(\na,\nb)\nend",
        &[(3, 2)],
        "def foo(\na,\nb\n)\nend",
    ),
    (
        LITERAL,
        194,
        "def foo(a,\nb\n)\nend",
        &[(3, 1)],
        "def foo(a,\nb)\nend",
    ),
    (
        LITERAL,
        198,
        "def foo(a,\nb: {\nfoo: bar\n}\n)\nend",
        &[(5, 1)],
        "def foo(a,\nb: {\nfoo: bar\n})\nend",
    ),
    (
        LITERAL,
        202,
        "def foo(a,\nb: {\nfoo: bar\n})\nend\n",
        &[],
        "",
    ),
    (LITERAL, 211, "def foo(a, # a\nb) # b\nend\n", &[], ""),
    (LITERAL, 229, "def foo(\na,\nb\n)\nend", &[], ""),
    (
        LITERAL,
        233,
        "def foo(\na,\nb: {\nfoo: bar\n}\n)\nend",
        &[],
        "",
    ),
    (
        LITERAL,
        237,
        "def foo(\na,\nb)\nend\n",
        &[(3, 2)],
        "def foo(\na,\nb\n)\nend\n",
    ),
    (LITERAL, 257, "def foo(a,\nb)\nend", &[], ""),
    (LITERAL, 261, "def foo(a,\nb: {\nfoo: bar\n})\nend", &[], ""),
    (
        LITERAL,
        265,
        "def foo(a,\nb: {\nfoo: bar\n}\n)\nend\n",
        &[(5, 1)],
        "def foo(a,\nb: {\nfoo: bar\n})\nend\n",
    ),
    (
        LITERAL,
        275,
        "def foo(a, # a\nb # b\n)\nend\n",
        &[(3, 1)],
        "def foo(a, # a\nb) # b\nend\n",
    ),
    (
        LITERAL,
        334,
        "def foo(\na,\nb)\nend",
        &[(3, 2)],
        "def foo(\na,\nb\n)\nend",
    ),
    (
        LITERAL,
        338,
        "def foo(\na,\nb: {\nfoo: bar\n})\nend",
        &[(5, 2)],
        "def foo(\na,\nb: {\nfoo: bar\n}\n)\nend",
    ),
    (LITERAL, 342, "def foo(\na,\nb\n)\nend\n", &[], ""),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 28);
    for &(group, line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "{group} line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineMethodDefinitionBraceLayout, source).is_empty(),
                "{group} line {line}"
            );
        } else if after.is_empty() {
            held(source);
        } else {
            assert_correction(&MultilineMethodDefinitionBraceLayout, source, after);
        }
    }
}

const PINNED_MESSAGES: (&str, &str) = ("def foo(a,\nb\n)\nend\n", "def foo(\na,\nb)\nend\n");

#[test]
fn messages_follow_the_opening_parenthesis() {
    let (same, new) = PINNED_MESSAGES;
    assert_eq!(messages(same), [MESSAGES.same_line]);
    assert_eq!(messages(new), [MESSAGES.new_line]);
}

#[test]
fn metadata_is_fixed_and_default_on() {
    assert_eq!(MultilineMethodDefinitionBraceLayout.name(), COP);
    assert!(MultilineMethodDefinitionBraceLayout.default_enabled());
    assert!(MultilineMethodDefinitionBraceLayout.department_selection_enables());
    assert!(MultilineMethodDefinitionBraceLayout.safe_fixes());
    assert_eq!(
        MultilineMethodDefinitionBraceLayout.autocorrect_incompatible_with(),
        ["Style/MultilineMethodSignature"]
    );
    let (same, _) = PINNED_MESSAGES;
    assert!(crate::pipeline::lint_str(same).iter().any(|o| o.cop == COP));
}

#[test]
fn every_parenthesized_definition_is_checked() {
    for (source, expected) in [
        ("def foo(a,\n  b\n)\nend\n", vec![(3, 1)]),
        ("def foo(\n  a,\n  b)\nend\n", vec![(3, 4)]),
        ("def self.foo(a,\n  b\n)\nend\n", vec![(3, 1)]),
        ("def x.foo(\n  a,\n  b)\nend\n", vec![(3, 4)]),
        ("def foo(a,\n  b\n) = a\n", vec![(3, 1)]),
        ("def ==(\n  other)\nend\n", vec![(2, 8)]),
        // Every parameter kind, in source order.
        (
            "def foo(a, b = 1,\n  *c, d, e:, f: 2,\n  **g, &h\n)\nend\n",
            vec![(4, 1)],
        ),
        ("def foo(a,\n  *\n)\nend\n", vec![(3, 1)]),
        ("def foo(a,\n  **nil\n)\nend\n", vec![(3, 1)]),
        ("def foo(a,\n  ...\n)\nend\n", vec![(3, 1)]),
        ("def foo(a,\n  &\n)\nend\n", vec![(3, 1)]),
        ("def foo((a, b),\n  c\n)\nend\n", vec![(3, 1)]),
        ("def foo(\n  (a,\n  b))\nend\n", vec![(3, 5)]),
        // A multi-line last parameter ends where its default value ends.
        ("def foo(a, b = [\n  1\n]\n)\nend\n", vec![(4, 1)]),
        ("def foo(a, b = [\n  1\n])\nend\n", vec![]),
        // Inside classes, blocks and as a call argument.
        (
            "class A\n  def foo(a,\n    b\n  )\n  end\nend\n",
            vec![(4, 3)],
        ),
        ("private def foo(a,\n  b\n)\nend\n", vec![(3, 1)]),
        ("x.each { def foo(\n  a,\n  b); end }\n", vec![(3, 4)]),
        // Character columns, not bytes.
        ("def foo(\n  é = \"é\", b)\nend\n", vec![(2, 13)]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn implicit_empty_single_line_and_non_definition_lookalikes_are_ignored() {
    for source in [
        "def foo a,\n  b\nend\n",
        "def foo\nend\n",
        "def foo()\nend\n",
        "def foo(\n)\nend\n",
        "def foo(a, b)\n  1\nend\n",
        "def foo(a, b = [\n  1\n]) = 1\n",
        // Calls, lambdas and blocks are not definitions.
        "foo(a,\n  b\n)\n".trim_start_matches('f'),
        "->(a,\n  b\n) {}\n",
        "x.each { |a,\n  b\n| }\n",
        "x = proc do |a,\n  b\n|\nend\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
}

#[test]
fn heredocs_reaching_the_last_line_or_opened_on_it_skip_the_definition() {
    for source in [
        "def foo(a,\n  b = <<~EOM\n  baz\nEOM\n)\nend\n",
        "def foo(a,\n  b: bar(<<~EOM)\n  baz\nEOM\n)\nend\n",
        "def foo(\n  a, b = <<~EOM)\n  baz\nEOM\nend\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
    // The body is not a descendant of the `args` node.
    for (source, expected) in [
        ("def foo(\n  a,\n  b) = <<~EOM\n  baz\nEOM\n", vec![(3, 4)]),
        (
            "def foo(\n  a,\n  b); <<~EOM\n  baz\nEOM\nend\n",
            vec![(3, 4)],
        ),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn canonical_edits() {
    for (source, after) in [
        ("def foo(a,\n  b\n)\nend\n", "def foo(a,\n  b)\nend\n"),
        ("def foo(a,\n  b\n) = a\n", "def foo(a,\n  b) = a\n"),
        (
            "def self.foo(a,\n  b\n)\nend\n",
            "def self.foo(a,\n  b)\nend\n",
        ),
        ("def foo(a,\n  &\n)\nend\n", "def foo(a,\n  &)\nend\n"),
        ("def foo(a,\n  ...\n)\nend\n", "def foo(a,\n  ...)\nend\n"),
        (
            "def foo(a,\n  b\n  # c\n)\nend\n",
            "def foo(a,\n  b)\n  # c\nend\n",
        ),
        // A comment after the last parameter: the rest of the line moves.
        (
            "def foo(a, # a\n  b # b\n)\nend\n",
            "def foo(a, # a\n  b) # b\nend\n",
        ),
        (
            "def foo(a, # a\n  b # b\n) = a\n",
            "def foo(a, # a\n  b) = a # b\n",
        ),
        // A definition is never `chained?` or `argument?`.
        (
            "private def foo(a,\n  b # b\n)\nend\n",
            "private def foo(a,\n  b) # b\nend\n",
        ),
        // A line break inserted before a parenthesis on the last line.
        (
            "def foo(\n  a,\n  b)\nend\n",
            "def foo(\n  a,\n  b\n)\nend\n",
        ),
        ("def foo(\n  a,\n  b) = a\n", "def foo(\n  a,\n  b\n) = a\n"),
        // CRLF: `\r\n` is one break; the inserted break is upstream's `\n`.
        (
            "def foo(a,\r\n  b\r\n)\r\nend\r\n",
            "def foo(a,\r\n  b)\r\nend\r\n",
        ),
        (
            "def foo(\r\n  a,\r\n  b)\r\nend\r\n",
            "def foo(\r\n  a,\r\n  b\n)\r\nend\r\n",
        ),
        // A default value's heredoc body between parameter and parenthesis.
        (
            "def foo(a = <<~EOS, b = 1\n  text\nEOS\n)\nend\n",
            "def foo(a = <<~EOS, b = 1)\n  text\nEOS\nend\n",
        ),
        // A nested lambda or hash bracket that is not an `args` node.
        (
            "def foo(a, b = { # c\n  k: 1 }\n)\nend\n",
            "def foo(a, b = { # c\n  k: 1 })\nend\n",
        ),
    ] {
        assert_correction(&MultilineMethodDefinitionBraceLayout, source, after);
    }
}

#[test]
fn holds_keep_the_finding_without_an_edit() {
    for source in [
        // `nested_closing_bracket_moved_before_comment?`: a nested `args`
        // node's `)` first on the last line after a comment line.
        "def foo(a, b = ->(x # c\n) { x }\n)\nend\n",
        "def foo(a, b = ->(x\n  # c\n) { x }\n)\nend\n",
        // A line continuation between parameter and parenthesis.
        "def foo(a,\n  b \\\n)\nend\n",
        // The moved rest of the line holds a comment, or would follow one.
        "def foo(a, # a\n  b # b\n) # c\nend\n",
        "def foo(a,\n  b\n  # c\n) = 1\n",
        // Whatever follows the parenthesis would be glued to the terminator.
        "def foo(a = <<~EOS, b = 1\n  text\nEOS\n) = 1\n",
        // Listed source observers.
        "def foo(a,\n  b\n)\nend\np __FILE__\n",
        "def foo(\n  a,\n  b)\nend\np __dir__\n",
        "def foo(a,\n  b\n) = caller\n",
    ] {
        held(source);
    }
    // A nested definition's parameters are the same kind: the outer edit is
    // held while the inner one is corrected.
    let source = "def foo(a, b = def bar(x # c\n); end\n)\nend\n";
    let found = correction_offenses(&MultilineMethodDefinitionBraceLayout, source);
    assert_eq!(
        found
            .iter()
            .map(|o| (o.line, o.correction.is_some()))
            .collect::<Vec<_>>(),
        [(2, true), (3, false)]
    );
}

#[test]
fn the_incompatible_style_peer_holds_the_edits() {
    let peer = crate::cops::style::MultilineMethodSignature::default();
    for source in ["def foo(a,\n  b\n)\nend\n", "def foo(\n  a,\n  b)\nend\n"] {
        let alone = correction_offenses(&MultilineMethodDefinitionBraceLayout, source);
        assert!(alone.iter().all(|o| o.correction.is_some()), "{source:?}");
        for selected in [
            [&MultilineMethodDefinitionBraceLayout as &dyn Cop, &peer],
            [&peer, &MultilineMethodDefinitionBraceLayout],
        ] {
            let both = lint_str_with_corrections(&selected, source);
            let own: Vec<_> = both.iter().filter(|o| o.cop == COP).collect();
            assert_eq!(own.len(), 1, "{source:?}");
            assert!(own[0].correction.is_none(), "{source:?}");
        }
    }
}
