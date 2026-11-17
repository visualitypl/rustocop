//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/multiline_method_call_brace_layout_spec.rb` and the
//! shared `spec/support/multiline_literal_brace_layout_examples.rb` and
//! `multiline_literal_brace_layout_trailing_comma_examples.rb` it includes, at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 47 pinned
//! examples run at the fixed `EnforcedStyle: symmetrical`; the `new_line` and
//! `same_line` contexts are replayed at it, so their findings follow the
//! symmetrical rule. Findings, messages and single-pass corrected bytes equal
//! the fixed replay recorded in the pinned RuboCop reference run (each
//! source and corrected SHA-256 was checked against it when the rows were
//! ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::{lint_str_with, lint_str_with_corrections},
};

const COP: &str = "Layout/MultilineMethodCallBraceLayout";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&MultilineMethodCallBraceLayout], source)
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
    lint_str_with(&[&MultilineMethodCallBraceLayout], source)
        .into_iter()
        .map(|o| o.message)
        .collect()
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&MultilineMethodCallBraceLayout, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&MultilineMethodCallBraceLayout, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

const MAIN: &str = "main";
const LITERAL: &str = "multiline literal brace layout";
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
    (MAIN, 8, "foo 1,\n2\n", &[], ""),
    (MAIN, 15, "foo(1,2)", &[], ""),
    (MAIN, 19, "puts", &[], ""),
    (MAIN, 23, "puts()", &[], ""),
    (MAIN, 27, "puts(\n)\n", &[], ""),
    (
        MAIN,
        34,
        "foo(<<~EOS, arg\n  text\nEOS\n).do_something\n",
        &[(4, 1)],
        "foo(<<~EOS, arg).do_something\n  text\nEOS\n",
    ),
    (
        MAIN,
        50,
        "foo(<<~EOS, arg\n  text\nEOS\n)&.do_something\n",
        &[(4, 1)],
        "foo(<<~EOS, arg)&.do_something\n  text\nEOS\n",
    ),
    (LITERAL, 50, "foo(a,\n<<-EOM\nbaz\nEOM\n)\n", &[], ""),
    (
        LITERAL,
        54,
        "foo(a,\n{\na: <<-EOM\nbaz\nEOM\n}\n)\n\n",
        &[(7, 1)],
        "foo(a,\n{\na: <<-EOM\nbaz\nEOM\n})\n\n",
    ),
    (LITERAL, 74, "foo(\na, <<~EOM, b)\n  baz\nEOM\n\n", &[], ""),
    (LITERAL, 74, "foo(\na, <<~EOM, b)\n  baz\nEOM\n\n", &[], ""),
    (LITERAL, 91, "foo(a,\nb)\n", &[], ""),
    (LITERAL, 95, "foo(a,\n{\nfoo: bar\n})\n", &[], ""),
    (LITERAL, 99, "foo(a,\nb\n)\n\n", &[(3, 1)], "foo(a,\nb)\n\n"),
    (
        LITERAL,
        109,
        "foo(a, # a\nb # b\n)\n\n",
        &[(3, 1)],
        "foo(a, # a\nb) # b\n\n",
    ),
    (
        LITERAL,
        132,
        "foo(a,\nb # comment b\n)\n.any?\n",
        &[(3, 1)],
        "",
    ),
    (
        LITERAL,
        147,
        "foo(a,\nb\n)\n.any?\n",
        &[(3, 1)],
        "foo(a,\nb)\n.any?\n",
    ),
    (LITERAL, 168, "foo(\na,\nb\n)\n", &[], ""),
    (LITERAL, 172, "foo(\na,\n{\nfoo: bar\n}\n)\n", &[], ""),
    (
        LITERAL,
        176,
        "foo(\na,\nb)\n",
        &[(3, 2)],
        "foo(\na,\nb\n)\n",
    ),
    (LITERAL, 194, "foo(a,\nb\n)\n", &[(3, 1)], "foo(a,\nb)\n"),
    (
        LITERAL,
        198,
        "foo(a,\n{\nfoo: bar\n}\n)\n",
        &[(5, 1)],
        "foo(a,\n{\nfoo: bar\n})\n",
    ),
    (LITERAL, 202, "foo(a,\n{\nfoo: bar\n})\n\n", &[], ""),
    (LITERAL, 211, "foo(a, # a\nb) # b\n\n", &[], ""),
    (LITERAL, 229, "foo(\na,\nb\n)\n", &[], ""),
    (LITERAL, 233, "foo(\na,\n{\nfoo: bar\n}\n)\n", &[], ""),
    (
        LITERAL,
        237,
        "foo(\na,\nb)\n\n",
        &[(3, 2)],
        "foo(\na,\nb\n)\n\n",
    ),
    (LITERAL, 257, "foo(a,\nb)\n", &[], ""),
    (LITERAL, 261, "foo(a,\n{\nfoo: bar\n})\n", &[], ""),
    (
        LITERAL,
        265,
        "foo(a,\n{\nfoo: bar\n}\n)\n\n",
        &[(5, 1)],
        "foo(a,\n{\nfoo: bar\n})\n\n",
    ),
    (
        LITERAL,
        275,
        "foo(a, # a\nb # b\n)\n\n",
        &[(3, 1)],
        "foo(a, # a\nb) # b\n\n",
    ),
    (
        LITERAL,
        298,
        "foo(a,\nb # comment b\n)\n.any?\n",
        &[(3, 1)],
        "",
    ),
    (
        LITERAL,
        313,
        "foo(a,\nb\n)\n.any?\n",
        &[(3, 1)],
        "foo(a,\nb)\n.any?\n",
    ),
    (
        LITERAL,
        334,
        "foo(\na,\nb)\n",
        &[(3, 2)],
        "foo(\na,\nb\n)\n",
    ),
    (
        LITERAL,
        338,
        "foo(\na,\n{\nfoo: bar\n})\n",
        &[(5, 2)],
        "foo(\na,\n{\nfoo: bar\n}\n)\n",
    ),
    (LITERAL, 342, "foo(\na,\nb\n)\n\n", &[], ""),
    (
        TRAILING_COMMA,
        14,
        "foo(a, # a\nb, # b\n)\n",
        &[(3, 1)],
        "foo(a, # a\nb,) # b\n",
    ),
    (
        TRAILING_COMMA,
        38,
        "foo(a, # a\nb, # b\n)\n",
        &[(3, 1)],
        "foo(a, # a\nb,) # b\n",
    ),
    (MAIN, 87, "puts(\"Hello world!\")", &[], ""),
    (MAIN, 91, "[\n].join(\" \")\n", &[], ""),
    (MAIN, 98, "[\n]\n.join(\" \")\n", &[], ""),
    (
        MAIN,
        108,
        "super(bar(baz,\n  ham # comment\n))\n",
        &[(3, 1)],
        "super(bar(baz,\n  ham)) # comment\n",
    ),
    (
        MAIN,
        125,
        "foo(\n  {\n    bar: 1 # comment\n  }\n)\n",
        &[],
        "",
    ),
    (MAIN, 146, "foo&.bar(1,2)", &[], ""),
    (
        MAIN,
        153,
        "foo&.bar(\n  baz)\n",
        &[(2, 6)],
        "foo&.bar(\n  baz\n)\n",
    ),
    (
        MAIN,
        171,
        "foo&.bar(\n  baz)\n",
        &[(2, 6)],
        "foo&.bar(\n  baz\n)\n",
    ),
    (MAIN, 189, "foo&.bar(\n  baz\n)\n", &[], ""),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    assert_eq!(PINNED.len(), 47);
    for &(group, line, source, expected, after) in PINNED {
        assert_eq!(positions(source), expected, "{group} line {line}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&MultilineMethodCallBraceLayout, source).is_empty(),
                "{group} line {line}"
            );
        } else if after.is_empty() {
            held(source);
        } else {
            assert_correction(&MultilineMethodCallBraceLayout, source, after);
        }
    }
}

const PINNED_MESSAGES: (&str, &str) = ("foo(a,\nb\n)\n", "foo(\na,\nb)\n");

#[test]
fn messages_follow_the_opening_parenthesis() {
    let (same, new) = PINNED_MESSAGES;
    assert_eq!(messages(same), [MESSAGES.same_line]);
    assert_eq!(messages(new), [MESSAGES.new_line]);
}

#[test]
fn metadata_is_fixed_and_default_on() {
    assert_eq!(MultilineMethodCallBraceLayout.name(), COP);
    assert!(MultilineMethodCallBraceLayout.default_enabled());
    assert!(MultilineMethodCallBraceLayout.department_selection_enables());
    assert!(MultilineMethodCallBraceLayout.safe_fixes());
    assert_eq!(
        MultilineMethodCallBraceLayout.autocorrect_incompatible_with(),
        ["Style/MethodCallWithArgsParentheses"]
    );
    let (same, _) = PINNED_MESSAGES;
    assert!(crate::pipeline::lint_str(same).iter().any(|o| o.cop == COP));
}

#[test]
fn every_parenthesized_send_and_csend_is_checked() {
    for (source, expected) in [
        ("x = foo(1,\n  2\n)\n", vec![(3, 1)]),
        ("x = foo(\n  1,\n  2)\n", vec![(3, 4)]),
        ("a.b(1,\n  2\n)\n", vec![(3, 1)]),
        ("a&.b(1,\n  2\n)\n", vec![(3, 1)]),
        ("a.(1,\n  2\n)\n", vec![(3, 1)]),
        ("A::b(1,\n  2\n)\n", vec![(3, 1)]),
        ("a.+(1,\n  2\n)\n", vec![(3, 1)]),
        ("a.[](1,\n  2\n)\n", vec![(3, 1)]),
        ("foo(*a,\n  **b\n)\n", vec![(3, 1)]),
        // A brace-less keyword hash is one argument.
        ("foo(a,\n  b: 1, c: 2\n)\n", vec![(3, 1)]),
        ("foo(\n  a: 1,\n  b: 2)\n", vec![(3, 7)]),
        // A block pass is the last argument.
        ("foo(a,\n  &b\n)\n", vec![(3, 1)]),
        ("foo(\n  a,\n  &b)\n", vec![(3, 5)]),
        ("def f(...)\n  g(1,\n    ...\n  )\nend\n", vec![(4, 3)]),
        // A literal block does not belong to the arguments.
        ("foo(a,\n  b\n) do\nend\n", vec![(3, 1)]),
        ("foo(a,\n  b) do\n  1\nend\n", vec![]),
        ("foo(a,\n  b\n) { |x|\n  x }\n", vec![(3, 1)]),
        // A multi-line last argument ends where its closing bracket is.
        ("foo(a, [\n  1\n]\n)\n", vec![(4, 1)]),
        ("foo(a, [\n  1\n])\n", vec![]),
        // Nested calls are checked independently.
        ("foo(bar(1,\n  2\n)\n)\n", vec![(3, 1), (4, 1)]),
        ("foo(a, bar(\n  1\n))\n", vec![]),
        // Inside interpolation, blocks and arguments of other calls.
        ("\"#{foo(1,\n  2\n)}\"\n", vec![(3, 1)]),
        ("x.each { foo(1,\n  2\n) }\n", vec![(3, 1)]),
        ("super(foo(1,\n  2\n))\n", vec![(3, 1)]),
        // Character columns, not bytes.
        ("foo(\n  \"é\", 1)\n", vec![(2, 9)]),
        // An endless method body.
        ("def f = foo(1,\n  2\n)\n", vec![(3, 1)]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn implicit_empty_single_line_and_non_send_lookalikes_are_ignored() {
    for source in [
        "foo 1,\n  2\n",
        "a.b 1,\n  2\n",
        "foo()\n",
        "foo(\n)\n",
        "foo(1, 2)\n",
        // A multi-line receiver: `single_line_ignoring_receiver?`.
        "[\n  1\n].join(\", \")\n",
        "[\n  1\n]\n  .join(\", \")\n",
        "foo(1,\n  2).bar(3, 4)\n",
        // Index calls, `super`, `yield`, keywords and definitions.
        "foo[1,\n  2\n]\n",
        "foo[1,\n  2\n] = 3\n",
        "def f\n  super(1,\n    2\n  )\nend\n",
        "def f\n  yield(1,\n    2\n  )\nend\n",
        "defined?(foo(1, 2)\n)\n",
        "def foo(a,\n  b\n)\nend\n",
        "->(a,\n  b\n) {}\n",
        "not(foo\n)\n",
        "x = (1 +\n  2\n)\n",
        "[1,\n  2\n]\n",
        "{a: 1,\n  b: 2\n}\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
}

#[test]
fn heredocs_reaching_the_last_line_or_opened_on_it_skip_the_call() {
    for source in [
        "foo(a,\n  <<~EOM\n  baz\nEOM\n)\n",
        "foo(a,\n  bar(<<~EOM)\n  baz\nEOM\n)\n",
        "foo(a,\n  <<~EOM.strip\n  baz\nEOM\n)\n",
        "foo(a,\n  &<<~EOM\n  baz\nEOM\n)\n",
        // A heredoc opened on the closing parenthesis's line, in the
        // arguments or the receiver.
        "foo(\n  a, <<~EOM)\n  baz\nEOM\n",
        "foo(\n  a, bar(<<~EOM), b)\n  baz\nEOM\n",
    ] {
        assert_eq!(positions(source), [], "{source:?}");
    }
    for (source, expected) in [
        // The literal block is not a `send` descendant.
        ("foo(\n  a,\n  b) { <<~EOM }\n  baz\nEOM\n", vec![(3, 4)]),
        // A heredoc in the chain after the parenthesis.
        ("foo(\n  a,\n  b).bar(<<~EOM)\n  baz\nEOM\n", vec![(3, 4)]),
        ("foo(a,\n  <<~EOM, b\n  baz\nEOM\n)\n", vec![(5, 1)]),
        ("foo(a,\n  b << c\n)\n", vec![(3, 1)]),
    ] {
        assert_eq!(positions(source), expected, "{source:?}");
    }
}

#[test]
fn canonical_edits() {
    for (source, after) in [
        ("foo(a,\n  b\n)\n", "foo(a,\n  b)\n"),
        ("foo(a,\n  b  \n\n\n  )\n", "foo(a,\n  b)  \n"),
        ("x = foo(a,\n  b\n).size\n", "x = foo(a,\n  b).size\n"),
        ("foo(a,\n  b,\n)\n", "foo(a,\n  b,)\n"),
        ("foo(a,\n  &:b\n)\n", "foo(a,\n  &:b)\n"),
        ("foo(a,\n  b: 1\n)\n", "foo(a,\n  b: 1)\n"),
        // An own-line comment before the parenthesis stays where it is.
        ("foo(a,\n  b\n  # c\n)\n", "foo(a,\n  b)\n  # c\n"),
        // A comment after the last argument: the rest of the line moves.
        ("foo(a, # a\n  b # b\n)\n", "foo(a, # a\n  b) # b\n"),
        (
            "foo(a, # a\n  b # b\n) do\nend\n",
            "foo(a, # a\n  b) do # b\nend\n",
        ),
        // A call wrapped by a literal block is not chained.
        (
            "foo(a,\n  b # b\n) { 1 }.bar\n",
            "foo(a,\n  b) { 1 }.bar # b\n",
        ),
        // Not `send` arguments: `csend`, `super`.
        ("x&.y(foo(a,\n  b # b\n))\n", "x&.y(foo(a,\n  b)) # b\n"),
        // The shared seam treats index calls as upstream's `index` nodes
        //; RuboCop's AST has a `send` and holds these edits
        // (a known deviation).
        ("x[foo(a,\n  b # b\n)]\n", "x[foo(a,\n  b)] # b\n"),
        ("foo(a,\n  b # b\n)[0]\n", "foo(a,\n  b)[0] # b\n"),
        // A line break inserted before a parenthesis on the last line.
        ("foo(\n  a,\n  b)\n", "foo(\n  a,\n  b\n)\n"),
        ("foo(\n  a,\n  b) # c\n", "foo(\n  a,\n  b\n) # c\n"),
        ("foo(\n  a,\n  b).bar\n", "foo(\n  a,\n  b\n).bar\n"),
        // Multibyte last arguments.
        ("foo(a,\n  :café\n)\n", "foo(a,\n  :café)\n"),
        // CRLF: `\r\n` is one break; the inserted break is upstream's `\n`.
        ("foo(a,\r\n  b\r\n)\r\n", "foo(a,\r\n  b)\r\n"),
        ("foo(\r\n  a,\r\n  b)\r\n", "foo(\r\n  a,\r\n  b\n)\r\n"),
        // Heredoc bodies between the last argument and the parenthesis.
        (
            "foo(<<~EOS, arg\n  text\nEOS\n)\n",
            "foo(<<~EOS, arg)\n  text\nEOS\n",
        ),
        (
            "foo(<<~A, <<~B, c\n  a\nA\n  b\nB\n)\n",
            "foo(<<~A, <<~B, c)\n  a\nA\n  b\nB\n",
        ),
        (
            "foo(<<~EOS, arg # c\n  text\nEOS\n) if x\n",
            "foo(<<~EOS, arg) if x # c\n  text\nEOS\n",
        ),
        // `correct_heredoc_argument_method_chain`.
        (
            "x = foo(<<~EOS, arg\n  text\nEOS\n).bar(1)\n",
            "x = foo(<<~EOS, arg).bar(1)\n  text\nEOS\n",
        ),
        (
            "foo(<<-EOS, arg,\n  text\n  EOS\n)&.bar\n",
            "foo(<<-EOS, arg,)&.bar\n  text\n  EOS\n",
        ),
    ] {
        assert_correction(&MultilineMethodCallBraceLayout, source, after);
    }
}

#[test]
fn nested_calls_correct_one_per_pass() {
    // The outer edit starts at the inner parenthesis, which the inner edit
    // replaces: one pass applies the inner one, the next run the outer.
    let source = "foo(bar(1,\n  2\n)\n)\n";
    let once = fixed(source);
    assert_eq!(once, "foo(bar(1,\n  2)\n)\n");
    assert_eq!(fixed(&once), "foo(bar(1,\n  2))\n");
}

#[test]
fn upstream_holds_keep_the_finding_without_an_edit() {
    for source in [
        // A comment after the last argument of a receiver or `send` argument.
        "foo(a,\n  b # b\n).size\n",
        "foo(a,\n  b # b\n)&.size\n",
        "p(foo(a,\n  b # b\n))\n",
        "p foo(a,\n  b # b\n)\n",
        "x + foo(a,\n  b # b\n)\n",
        "x.y = foo(a,\n  b, # b\n)\n",
    ] {
        held(source);
    }
    // A nested call's parenthesis first on the last line after a comment
    // line holds the outer edit.
    for (source, line) in [
        ("foo(a, bar(b,\n  # c\n)\n)\n", 4),
        // Not a receiver or argument: a trailing comment holds too.
        ("foo(a, x = bar(b, # c\n)\n)\n", 3),
    ] {
        let found = correction_offenses(&MultilineMethodCallBraceLayout, source);
        let outer = found.iter().find(|o| o.line == line);
        assert!(
            outer.is_some_and(|o| o.correction.is_none()),
            "{source:?} {found:?}"
        );
    }
    // The nested call is an argument: only an own-line comment holds. The
    // last argument of a `send` is an argument itself, that of a `csend` not.
    for (source, after) in [
        (
            "foo(a, bar(1, baz(b, # c\n))\n)\n",
            "foo(a, bar(1, baz(b, # c\n)))\n",
        ),
        ("foo(a, bar(b, # c\n)\n)\n", "foo(a, bar(b, # c\n))\n"),
    ] {
        let found = correction_offenses(&MultilineMethodCallBraceLayout, source);
        let held: Vec<_> = found
            .iter()
            .map(|o| (o.line, o.correction.is_none()))
            .collect();
        assert_eq!(held.len(), 2, "{source:?}");
        assert!(
            held.contains(&(2, true)) && held.contains(&(3, false)),
            "{held:?}"
        );
        assert_eq!(fixed(source), after);
    }
    let source = "x&.foo(a, bar(b, # c\n)\n)\n";
    let found = correction_offenses(&MultilineMethodCallBraceLayout, source);
    assert!(
        found.iter().any(|o| o.line == 3 && o.correction.is_none()),
        "{found:?}"
    );
    assert_eq!(fixed(source), "x&.foo(a, bar(b,) # c\n)\n");
}

#[test]
fn the_incompatible_style_peer_holds_the_edits() {
    let peer = crate::cops::style::MethodCallWithArgsParentheses;
    for source in ["foo(a,\n  b\n)\n", "foo(\n  a,\n  b)\n"] {
        let alone = correction_offenses(&MultilineMethodCallBraceLayout, source);
        assert!(alone.iter().all(|o| o.correction.is_some()), "{source:?}");
        for selected in [
            [&MultilineMethodCallBraceLayout as &dyn Cop, &peer],
            [&peer, &MultilineMethodCallBraceLayout],
        ] {
            let both = lint_str_with_corrections(&selected, source);
            let own: Vec<_> = both.iter().filter(|o| o.cop == COP).collect();
            assert_eq!(own.len(), 1, "{source:?}");
            assert!(own[0].correction.is_none(), "{source:?}");
        }
    }
}
