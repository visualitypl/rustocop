//! Examples ported from RuboCop's `spec/rubocop/cop/layout/closing_heredoc_indentation_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All
//! examples run at the fixed defaults; findings, messages and single-pass
//! corrected bytes match the fixed replay recorded in
//! the pinned RuboCop reference run.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    offense::Severity,
    pipeline::lint_str,
};

const COP: &str = "Layout/ClosingHeredocIndentation";

fn positions(source: &str) -> Vec<(u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source}"
    );
    lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| (o.line, o.col, o.message))
        .collect()
}

/// Every pinned example without an offense at the fixed defaults.
const ACCEPTED: &[&str] = &[
    // 8 accepts correctly indented closing heredoc
    "class Test\n  def foo\n    <<-SQL\n      bar\n    SQL\n  end\nend\n",
    // 20 accepts correctly indented closing heredoc when heredoc contents is after closing heredoc
    "include_examples :offense,\n                 <<-EOS\n                   foo\n                     bar\n                 EOS\n",
    // 30 accepts correctly indented closing heredoc when heredoc contents with blank line
    "def_node_matcher :eval_without_location?, <<~PATTERN\n  {\n    (send $(send _ $:sort ...) ${:[] :at :slice} {(int 0) (int -1)})\n\n    (send $(send _ $:sort_by _) ${:last :first})\n  }\nPATTERN\n",
    // 43 accepts correctly indented closing heredoc when aligned at the beginning of method definit
    "include_examples :offense,\n                 <<-EOS\n  bar\nEOS\n",
    // 53 accepts correctly indented closing heredoc when aligned at the beginning of method definit
    "include_examples :offense,\n                 <<-EOS.strip_indent\n  bar\nEOS\n",
    // 63 accepts correctly indented closing heredoc when aligned at the beginning of method definit
    "let(:source) { <<~EOS }\nEOS\n",
    // 70 accepts correctly indented closing heredoc when heredoc contents is before closing heredoc
    "include_examples :offense,\n                 <<-EOS\n                   foo\n  bar\n                   baz\n                 EOS\n",
    // 123 does not register an offense for correctly indented empty heredocs
    "def foo\n  <<-NIL\n\n  NIL\nend\n",
    // 133 does not register an offense for a << heredoc
    "def foo\n  <<NIL\n\nNIL\nend\n",
];

type Reported = (
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

/// Every pinned example with offenses, and its single-pass correction.
const REPORTED: &[Reported] = &[
    // 81 registers an offense for bad indentation of a closing heredoc
    (
        "class Test\n  def foo\n    <<-SQL\n      bar\n  SQL\n  end\nend\n",
        &[(5, 1, "`SQL` is not aligned with `<<-SQL`.")],
        "class Test\n  def foo\n    <<-SQL\n      bar\n    SQL\n  end\nend\n",
    ),
    // 104 registers an offense for incorrectly indented empty heredocs
    (
        "def foo\n  <<-NIL\n\n    NIL\nend\n",
        &[(4, 1, "`NIL` is not aligned with `<<-NIL`.")],
        "def foo\n  <<-NIL\n\n  NIL\nend\n",
    ),
];

#[test]
fn pinned_accepted_examples() {
    for source in ACCEPTED {
        assert_eq!(positions(source), vec![], "{source}");
    }
}

#[test]
fn pinned_reported_examples_and_single_pass_corrections() {
    for (source, expected, after) in REPORTED {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(l, c, m)| (l, c, m.to_owned()))
            .collect();
        assert_eq!(positions(source), expected, "{source}");
        assert_correction(&ClosingHeredocIndentation, source, after);
    }
}

#[test]
fn metadata_is_fixed() {
    let cop = ClosingHeredocIndentation;
    assert!(cop.default_enabled() && cop.safe_fixes());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.max_setting().is_none() && cop.autocorrect_incompatible_with().is_empty());
}

const MSG_ARG: &str = "`A` is not aligned with `<<~A` or beginning of method definition.";

#[test]
fn arguments_and_receivers_may_align_with_the_outermost_send() {
    for source in [
        // Argument of an outer send, through nested sends and receivers.
        "foo(bar,\n    baz(<<~A))\n  x\nA\n",
        "foo bar,\n    <<~A.strip\n  x\nA\n",
        "a.b = c(\n      <<~A)\n  x\nA\n",
        "x << <<~A\n  y\nA\n",
        "!<<~A\n  y\nA\n"
            .replace('!', "foo(!")
            .replace("A\n  y", "A)\n  y")
            .as_str(),
        // A literal block wraps its call; the call is the outermost send.
        "foo(\n    <<~A) do\n  x\nA\nend\n",
        // A safe-navigation receiver is `chained?`; its call climbs to `foo`.
        "foo(\n  <<~A&.strip, 1)\n  x\nA\n",
    ] {
        assert_eq!(positions(source), vec![], "{source}");
    }
    // Not `argument?` nor `chained?`: index, `[]=`, super, yield, a pair, a
    // splat, a regexp match, a safe-navigation call's argument.
    for (source, line, message) in [
        (
            "foo[\n  <<~A]\n  x\nA\n",
            4,
            "`A` is not aligned with `<<~A]`.",
        ),
        (
            "super(\n  <<~A)\n  x\nA\n",
            4,
            "`A` is not aligned with `<<~A)`.",
        ),
        (
            "foo(k:\n  <<~A)\n  x\nA\n",
            4,
            "`A` is not aligned with `<<~A)`.",
        ),
        (
            "foo(*\n  <<~A)\n  x\nA\n",
            4,
            "`A` is not aligned with `<<~A)`.",
        ),
        (
            "/x/ =~\n  <<~A\n  x\nA\n",
            4,
            "`A` is not aligned with `<<~A`.",
        ),
        (
            "a&.b(\n  <<~A)\n  x\nA\n",
            4,
            "`A` is not aligned with `<<~A)`.",
        ),
    ] {
        assert_eq!(
            positions(source),
            vec![(line, 1, message.into())],
            "{source}"
        );
    }
    // `argument?` (MSG_ARG) needs a `send`; a misaligned closing reports it.
    assert_eq!(
        positions("foo(\n  <<~A)\n  x\n A\n"),
        vec![(4, 1, MSG_ARG.replace("<<~A", "<<~A)"))]
    );
    assert_eq!(
        positions("  foo(<<~A)\n    x\n A\n"),
        vec![(
            3,
            1,
            "`A` is not aligned with `foo(<<~A)` or beginning of method definition.".into()
        )]
    );
}

#[test]
fn lookalikes_and_measures() {
    for source in [
        "x = <<A\n  y\nA\n",
        "x = \"\n  A\n\"\n",
        "  x = <<~A\n    y\n  A\n",
        "  x = <<-`A`\n    ls\n  A\n",
        // Leading spaces only: tabs are not counted on either line.
        "\tx = <<~A\n  y\n\tA\n",
    ] {
        assert_eq!(positions(source), vec![], "{source:?}");
    }
    // A BOM is not a space: the opening line of a BOM file has no indentation.
    assert_eq!(
        positions("\u{feff}x = <<~A\n  y\n  A\n"),
        vec![(3, 1, "`A` is not aligned with `\u{feff}x = <<~A`.".into())]
    );
}

#[test]
fn canonical_edits() {
    for (before, after) in [
        ("  x = <<~A\n    y\nA\n", "  x = <<~A\n    y\n  A\n"),
        ("x = <<-A\n  y\n    A\n", "x = <<-A\n  y\nA\n"),
        (
            "  x = <<~A\r\n    y\r\n      A\r\n",
            "  x = <<~A\r\n    y\r\n  A\r\n",
        ),
        ("x = <<~A\n  y\n \tA\n", "x = <<~A\n  y\n\tA\n"),
        (
            "def f\n  <<~A\n    #{<<~B}\n      b\n    B\n      A\nend\n",
            "def f\n  <<~A\n    #{<<~B}\n      b\n    B\n  A\nend\n",
        ),
    ] {
        assert_correction(&ClosingHeredocIndentation, before, after);
    }
}

#[test]
fn listed_source_observers_hold_the_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p caller",
        "p SCRIPT_LINES__",
    ] {
        let source = format!("{observer}\n  x = <<~A\n    y\nA\n");
        let found = correction_offenses(&ClosingHeredocIndentation, &source);
        assert_eq!(found.len(), 1, "{source}");
        assert!(found[0].correction.is_none(), "{source}");
    }
}
