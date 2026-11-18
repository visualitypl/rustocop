//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_before_block_braces_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 18 pinned
//! examples run at the fixed `EnforcedStyle: space` and
//! `EnforcedStyleForEmptyBraces: space`, the `no_space` contexts, the
//! `Style/BlockDelimiters: line_count_based` context and the invalid-style
//! example included; findings, messages and single-pass corrected bytes
//! equal the fixed replay recorded in the pinned RuboCop reference run
//! (each source and corrected SHA-256 was checked against it when the rows
//! were ported). The `Ruby 2.7` contexts' numbered parameters and the
//! `Ruby 3.4` context's `it` are valid at the fixed Ruby 3.4.
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceBeforeBlockBraces";
const MISSING: &str = MSG;

fn findings(source: &str) -> Vec<(u32, u32, &'static str)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceBeforeBlockBraces], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            (o.line, o.col, MSG)
        })
        .collect();
    found.sort_unstable();
    found
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (
    u32,
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (8, "each { puts }", &[], ""),
    (12, "each{ puts }\n", &[(1, 5, MISSING)], "each { puts }\n"),
    (
        24,
        "each{ puts }\neach { puts }\n",
        &[(1, 5, MISSING)],
        "each { puts }\neach { puts }\n",
    ),
    (
        39,
        "foo.map{ |a|\n  a.bar.to_s\n}\n",
        &[(1, 8, MISSING)],
        "foo.map { |a|\n  a.bar.to_s\n}\n",
    ),
    (
        55,
        "each{ _1 }\neach { _1 }\n",
        &[(1, 5, MISSING)],
        "each { _1 }\neach { _1 }\n",
    ),
    (
        70,
        "foo.map{\n  _1.bar.to_s\n}\n",
        &[(1, 8, MISSING)],
        "foo.map {\n  _1.bar.to_s\n}\n",
    ),
    (
        87,
        "each{ it }\neach { it }\n",
        &[(1, 5, MISSING)],
        "each { it }\neach { it }\n",
    ),
    (
        102,
        "foo.map{\n  it.bar.to_s\n}\n",
        &[(1, 8, MISSING)],
        "foo.map {\n  it.bar.to_s\n}\n",
    ),
    (122, "each { puts }\n", &[], ""),
    (
        134,
        "each{ puts }\neach { puts }\n",
        &[(1, 5, MISSING)],
        "each { puts }\neach { puts }\n",
    ),
    (148, "each{ puts }", &[(1, 5, MISSING)], "each { puts }"),
    (
        153,
        "each{ _1 }\neach { _1 }\n",
        &[(1, 5, MISSING)],
        "each { _1 }\neach { _1 }\n",
    ),
    (
        180,
        "let(:foo){{foo: 1, bar: 2}}\n",
        &[(1, 10, MISSING)],
        "let(:foo) {{foo: 1, bar: 2}}\n",
    ),
    (196, "->{}", &[(1, 3, MISSING)], "-> {}"),
    (200, "-> {}\n", &[], ""),
    (221, "-> {}", &[], ""),
    (225, "->{}\n", &[(1, 3, MISSING)], "-> {}\n"),
    (241, "each {}", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 18);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceBeforeBlockBraces, source).is_empty());
        } else {
            assert_correction(&SpaceBeforeBlockBraces, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceBeforeBlockBraces;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert_eq!(
        cop.autocorrect_deferred_to(),
        ["Style/BlockDelimiters", "Style/SymbolProc"]
    );
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn every_brace_block_and_lambda() {
    for (source, expected, after) in [
        ("a.b{ 1 }\n", &[(1, 4, MISSING)][..], "a.b { 1 }\n"),
        ("a&.b{ 1 }\n", &[(1, 5, MISSING)], "a&.b { 1 }\n"),
        ("a.b(1){}\n", &[(1, 7, MISSING)], "a.b(1) {}\n"),
        ("a = 1\na{}\n", &[(2, 2, MISSING)], "a = 1\na {}\n"),
        ("foo a{}\n", &[(1, 6, MISSING)], "foo a {}\n"),
        ("A{}\n", &[(1, 2, MISSING)], "A {}\n"),
        ("A::B{}\n", &[(1, 5, MISSING)], "A::B {}\n"),
        ("f!{}\n", &[(1, 3, MISSING)], "f! {}\n"),
        ("f?{}\n", &[(1, 3, MISSING)], "f? {}\n"),
        (
            "def m = super{}\n",
            &[(1, 14, MISSING)],
            "def m = super {}\n",
        ),
        (
            "def m = super(1){}\n",
            &[(1, 17, MISSING)],
            "def m = super(1) {}\n",
        ),
        ("a.(){}\n", &[(1, 5, MISSING)], "a.() {}\n"),
        ("->(a){ a }\n", &[(1, 6, MISSING)], "->(a) { a }\n"),
        ("-> a{ a }\n", &[(1, 5, MISSING)], "-> a { a }\n"),
        (
            "each{ _1 + _2 }\n",
            &[(1, 5, MISSING)],
            "each { _1 + _2 }\n",
        ),
        (
            "a{ b{} }\n",
            &[(1, 2, MISSING), (1, 5, MISSING)],
            "a { b {} }\n",
        ),
        ("a do b{} end\n", &[(1, 7, MISSING)], "a do b {} end\n"),
        (
            "x = [1].map{_1}.sum\n",
            &[(1, 12, MISSING)],
            "x = [1].map {_1}.sum\n",
        ),
        ("é.map{ 1 }\n", &[(1, 6, MISSING)], "é.map { 1 }\n"),
        ("\u{feff}a{}\n", &[(1, 2, MISSING)], "\u{feff}a {}\n"),
        ("a{}\r\n", &[(1, 2, MISSING)], "a {}\r\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        assert_correction(&SpaceBeforeBlockBraces, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "a { 1 }\n",
        "a\t{ 1 }\n",
        "a {}\n",
        "-> {}\n",
        "->(a) {}\n",
        "a do end\n",
        "-> do end\n",
        "a.b do |x| x end\n",
        "a(&b)\n",
        "x = {a: 1}\n",
        "x = a({})\n",
        "x = \"#{a}\"\n",
        "x = %w{a}\n",
        "x = /a{2}/\n",
        "# a{}\n",
        "a \\\n{ 1 }\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

/// This cop's edit with the given companion also running.
fn brace_edit(companion: &dyn crate::cops::Cop, source: &str) -> bool {
    let found =
        crate::pipeline::lint_str_with_corrections(&[&SpaceBeforeBlockBraces, companion], source);
    let own: Vec<_> = found.iter().filter(|o| o.cop == COP).collect();
    assert_eq!(own.len(), 1, "{source:?}");
    let companion_edits = found
        .iter()
        .any(|o| o.cop == companion.name() && o.correction.is_some());
    assert!(!(companion_edits && own[0].correction.is_some()));
    own[0].correction.is_some()
}

#[test]
fn upstream_incompatible_companions_take_precedence_only_when_they_edit() {
    use crate::cops::style::{BlockDelimiters, SymbolProc};
    // Style/BlockDelimiters turns the multi-line braces into `do ... end`:
    // the blank before `{` waits for the next pass, as upstream's Team.
    assert!(!brace_edit(&BlockDelimiters, "a.map{ |x|\n  x\n}\n"));
    // A merely active companion, or one whose edit a directive disables,
    // defers nothing.
    assert!(brace_edit(&BlockDelimiters, "a.map{ |x| x }\n"));
    assert!(brace_edit(
        &BlockDelimiters,
        "# rubocop:disable Style/BlockDelimiters\na.map{ |x|\n  x\n}\n"
    ));
    // SymbolProc's edits are unsafe: a safe pass (this seam) does not apply
    // them, so they defer nothing.
    let found = crate::pipeline::lint_str_with_corrections(
        &[&SpaceBeforeBlockBraces, &SymbolProc],
        "a.map{ |x| x.to_s }\n",
    );
    assert!(found.iter().any(|o| o.cop == COP && o.correction.is_some()));
}
