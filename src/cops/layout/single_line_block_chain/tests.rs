//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/single_line_block_chain_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 11 pinned examples
//! (the `:ruby34` `it` example applies at the fixed Ruby 3.4) run with the
//! cop enabled, as the spec's config does. Findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SingleLineBlockChain";

/// `(line, col)` of each finding, checking the message, severity and that no
/// edit is offered without a correction request.
fn findings(source: &str) -> Vec<(u32, u32)> {
    let mut found: Vec<_> = lint_str_with(&[&SingleLineBlockChain], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect();
    found.sort();
    found
}

/// Apply this cop's corrections once, as one `--fix` pass would.
fn fixed(source: &str) -> String {
    let offenses = correction_offenses(&SingleLineBlockChain, source);
    let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
    String::from_utf8(applied.bytes).unwrap()
}

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&SingleLineBlockChain, source);
    assert!(!found.is_empty(), "{source}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (
        5,
        "example.select { |item| item.cond? }.join('-')\n",
        &[(1, 37)],
        "example.select { |item| item.cond? }\n.join('-')\n",
    ),
    (
        17,
        "example.select { _1.cond? }.join('-')\n",
        &[(1, 28)],
        "example.select { _1.cond? }\n.join('-')\n",
    ),
    (
        29,
        "example.select { it.cond? }.join('-')\n",
        &[(1, 28)],
        "example.select { it.cond? }\n.join('-')\n",
    ),
    (
        41,
        "example.select { |item| item.cond? }&.join('-')\n",
        &[(1, 37)],
        "example.select { |item| item.cond? }\n&.join('-')\n",
    ),
    (
        53,
        "example.select { |item| item.cond? }.(42)\n",
        &[(1, 37)],
        "example.select { |item| item.cond? }\n.(42)\n",
    ),
    (
        65,
        "example.select { |item| item.cond? }\n       .join('-')\n",
        &[],
        "",
    ),
    (
        72,
        "example.select { |item| item.cond? }.\n        join('-')\n",
        &[],
        "",
    ),
    (79, "example.select { |item| item.cond? } + 2\n", &[], ""),
    (
        85,
        "example.select do |item|\n  item.cond?\nend.join('-')\n",
        &[],
        "",
    ),
    (
        93,
        "<<~TEXT.each_line.map { |line| line.strip }.join\n  text\nTEXT\n",
        &[],
        "",
    ),
    (
        101,
        "example.map { |item| item.name }.join(<<~SEP)\n  ,\nSEP\n",
        &[(1, 33)],
        "example.map { |item| item.name }\n.join(<<~SEP)\n  ,\nSEP\n",
    ),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 11);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SingleLineBlockChain, source).is_empty());
        } else {
            assert_correction(&SingleLineBlockChain, source, corrected);
        }
    }
}

#[test]
fn opt_in_metadata() {
    assert!(!SingleLineBlockChain.default_enabled());
    assert!(!SingleLineBlockChain.department_selection_enables());
    assert!(SingleLineBlockChain.safe_fixes());
    // Upstream's pass-ordering peers are default-on and not declared.
    assert!(
        SingleLineBlockChain
            .autocorrect_incompatible_with()
            .is_empty()
    );
}

#[test]
fn every_dotted_send_on_a_single_line_block_is_reported() {
    for (source, expected, after) in [
        ("a { }.b\n", (1, 6), "a { }\n.b\n"),
        (
            "a.b(1) { |x| x }.c.d\n",
            (1, 17),
            "a.b(1) { |x| x }\n.c.d\n",
        ),
        ("a do |x| x end.b\n", (1, 15), "a do |x| x end\n.b\n"),
        ("super { }.b\n", (1, 10), "super { }\n.b\n"),
        ("super(1) { }.b\n", (1, 13), "super(1) { }\n.b\n"),
        ("->(x) { x }.(1)\n", (1, 12), "->(x) { x }\n.(1)\n"),
        ("-> { 1 }.call\n", (1, 9), "-> { 1 }\n.call\n"),
        ("a { }.b = 1\n", (1, 6), "a { }\n.b = 1\n"),
        ("a { }.b += 1\n", (1, 6), "a { }\n.b += 1\n"),
        ("a { }.b ||= 1\n", (1, 6), "a { }\n.b ||= 1\n"),
        ("a { }&.b &&= 1\n", (1, 6), "a { }\n&.b &&= 1\n"),
        ("a { }.b, c = 1, 2\n", (1, 6), "a { }\n.b, c = 1, 2\n"),
        // The block's own call may span lines; only `{` and `}` count.
        ("a(\n  1\n) { }.b\n", (3, 6), "a(\n  1\n) { }\n.b\n"),
        ("é { }.b\n", (1, 6), "é { }\n.b\n"),
        ("\u{feff}a { }.b\n", (1, 6), "\u{feff}a { }\n.b\n"),
        ("a { }.b\r\n", (1, 6), "a { }\r\n.b\r\n"),
        (
            "x = a { }.b(<<~S)\n  t\nS\n",
            (1, 10),
            "x = a { }\n.b(<<~S)\n  t\nS\n",
        ),
        ("p(a { }.b)\n", (1, 8), "p(a { }\n.b)\n"),
    ] {
        assert_eq!(findings(source), [expected], "{source}");
        assert_correction(&SingleLineBlockChain, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "a { }\n",
        "a { }[0]\n",
        "a { } + 1\n",
        "a(&b).c\n",
        "a.b.c\n",
        "a do\nend.b\n",
        "a {\n}.b\n",
        "a { }\n.b\n",
        "a { }.\n  b\n",
        "-> {\n}.call\n",
        "<<~T.lines.map { _1 }.join\n  t\nT\n",
        "a { <<~T }.b\n  t\nT\n",
        "a(<<~T) { }.b\n  t\nT\n",
    ] {
        assert_eq!(findings(source), [], "{source}");
    }
    // Upstream compares columns only: a selector on the next line further
    // right than the trailing dot is reported.
    assert_eq!(findings("a { }.\n          b\n"), [(1, 6)]);
}

#[test]
fn unsafe_insertions_are_withheld() {
    for source in [
        // A line starting with `::` is a new top-level constant reference.
        "a { }::b\n",
        // A heredoc opened before the dot outside the block.
        "p(<<~T, a { }.b)\n  t\nT\n",
        // A directive on the line.
        "a { }.b # rubocop:disable Lint/Debugger\n",
        "a { }.b # rustocop:disable Style\n",
        // the listed source observers.
        "a { __FILE__ }.b\n",
        "a { __dir__ }.b\n",
        "a { $0 }.b\n",
        "a { $PROGRAM_NAME }.b\n",
        "a { caller }.b\n",
        "a { caller_locations }.b\n",
        "a { m.source_location }.b\n",
        "a { SCRIPT_LINES__ }.b\n",
        "a { DATA.read }.b\n__END__\nx\n",
    ] {
        held(source);
    }
}

#[test]
fn line_values_are_held_by_the_shared_guard() {
    held("a { }.b\np __LINE__\n");
    assert_eq!(fixed("p __LINE__\na { }.b\n"), "p __LINE__\na { }\n.b\n");
}

#[test]
fn chains_insert_one_break_per_block() {
    assert_eq!(fixed("a { }.b { }.c\n"), "a { }\n.b { }\n.c\n");
}
