//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_around_block_parameters_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 51
//! pinned examples (the shared `common behavior` examples of both contexts
//! included) run at the fixed defaults (`EnforcedStyleInsidePipes:
//! no_space`, `Layout/SpaceInsideParens` enabled), whatever style or
//! `SpaceInsideParens` setting the example declares; findings, messages and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceAroundBlockParameters";
const BEFORE: &str = "Space before first block parameter detected.";
const AFTER: &str = "Space after last block parameter detected.";
const CLOSING: &str = "Space after closing `|` missing.";
const EXTRA: &str = "Extra space before block parameter detected.";

fn findings_with(cop: &SpaceAroundBlockParameters, source: &str) -> Vec<(u32, u32, String)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[cop], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            (o.line, o.col, o.message)
        })
        .collect();
    found.sort_unstable();
    found
}

fn findings(source: &str) -> Vec<(u32, u32, String)> {
    findings_with(&SpaceAroundBlockParameters::default(), source)
}

fn expected(rows: &[(u32, u32, &str)]) -> Vec<(u32, u32, String)> {
    let mut rows: Vec<_> = rows
        .iter()
        .map(|&(line, col, message)| (line, col, message.to_owned()))
        .collect();
    rows.sort_unstable();
    rows
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceAroundBlockParameters::default(), source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings. Shared `common behavior`
/// examples carry the shared definition's lines (6, 10, 14).
type Pinned = (
    u32,
    &'static str,
    &'static [(u32, u32, &'static str)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (25, "{}.each { |x, y| puts x }", &[], ""),
    (29, "{}.each { |x, y| }", &[], ""),
    (34, "{}.each { |x,y| puts x }", &[], ""),
    (
        39,
        "some_result = lambda do |\n  so_many,\n  parameters,\n  it_will,\n  be_too_long,\n  for_one_line\n|\n  do_something\nend\n",
        &[],
        "",
    ),
    (53, "->(x, y) { puts x }", &[], ""),
    (
        57,
        "{}.each { | x| puts x }\n",
        &[(1, 12, BEFORE)],
        "{}.each { |x| puts x }\n",
    ),
    (
        68,
        "{}.each { |x, y  | puts x }\n",
        &[(1, 16, AFTER)],
        "{}.each { |x, y| puts x }\n",
    ),
    (
        79,
        "{}.each { |x, y|puts x }\n",
        &[(1, 16, CLOSING)],
        "{}.each { |x, y| puts x }\n",
    ),
    (90, "->( x, y) { puts x }\n", &[], ""),
    (101, "->(x, y  ) { puts x }\n", &[], ""),
    (112, "{}.each do |x, y|\n  puts x\nend\n", &[], ""),
    (
        120,
        "{}.each { |x,   y| puts x }\n",
        &[(1, 14, EXTRA)],
        "{}.each { |x, y| puts x }\n",
    ),
    (
        131,
        "{}.each { |a,  (x,  y),  z| puts x }\n",
        &[(1, 14, EXTRA), (1, 19, EXTRA), (1, 24, EXTRA)],
        "{}.each { |a, (x, y), z| puts x }\n",
    ),
    (
        168,
        "{}.each { |  x=5,  (y,*z) |puts x }\n",
        &[
            (1, 12, BEFORE),
            (1, 12, EXTRA),
            (1, 18, EXTRA),
            (1, 26, AFTER),
            (1, 27, CLOSING),
        ],
        "{}.each { |x=5, (y,*z)| puts x }\n",
    ),
    (
        183,
        "->(  a,  b, c) { puts a }\n",
        &[(1, 4, EXTRA), (1, 8, EXTRA)],
        "->( a, b, c) { puts a }\n",
    ),
    (6, "{}.each {}", &[], ""),
    (10, "->() { puts \"a\" }", &[], ""),
    (14, "->a { puts a }", &[], ""),
    (
        145,
        "{}.each { |x, | puts x }\n",
        &[(1, 14, AFTER)],
        "{}.each { |x,| puts x }\n",
    ),
    (
        156,
        "{}.each { |x , | puts x }\n",
        &[(1, 15, AFTER)],
        "{}.each { |x ,| puts x }\n",
    ),
    (
        203,
        "{}.each { | x, y | puts x }",
        &[(1, 12, BEFORE), (1, 17, AFTER)],
        "{}.each { |x, y| puts x }",
    ),
    (
        207,
        "{}.each { | x, y | }",
        &[(1, 12, BEFORE), (1, 17, AFTER)],
        "{}.each { |x, y| }",
    ),
    (
        212,
        "{}.each { | x,y | puts x }",
        &[(1, 12, BEFORE), (1, 16, AFTER)],
        "{}.each { |x,y| puts x }",
    ),
    (216, "->( x, y ) { puts x }", &[], ""),
    (
        220,
        "{}.each { |x | puts x }\n",
        &[(1, 13, AFTER)],
        "{}.each { |x| puts x }\n",
    ),
    (
        231,
        "{}.each { | x, y| puts x }\n",
        &[(1, 12, BEFORE)],
        "{}.each { |x, y| puts x }\n",
    ),
    (
        242,
        "{}.each { |  x | puts x }\n",
        &[(1, 12, BEFORE), (1, 12, EXTRA), (1, 15, AFTER)],
        "{}.each { |x| puts x }\n",
    ),
    (
        253,
        "{}.each { | x, y   | puts x }\n",
        &[(1, 12, BEFORE), (1, 17, AFTER)],
        "{}.each { |x, y| puts x }\n",
    ),
    (
        264,
        "{}.each { | x, y |puts x }\n",
        &[(1, 12, BEFORE), (1, 17, AFTER), (1, 18, CLOSING)],
        "{}.each { |x, y| puts x }\n",
    ),
    (275, "->(x ) { puts x }\n", &[], ""),
    (286, "->( x, y) { puts x }\n", &[], ""),
    (
        297,
        "->(  x ) { puts x }\n",
        &[(1, 4, EXTRA)],
        "->( x ) { puts x }\n",
    ),
    (308, "->( x, y   ) { puts x }\n", &[], ""),
    (
        319,
        "{}.each do | x, y |\n  puts x\nend\n",
        &[(1, 13, BEFORE), (1, 18, AFTER)],
        "{}.each do |x, y|\n  puts x\nend\n",
    ),
    (
        327,
        "{}.each { | x,   y | puts x }\n",
        &[(1, 12, BEFORE), (1, 15, EXTRA), (1, 19, AFTER)],
        "{}.each { |x, y| puts x }\n",
    ),
    (
        338,
        "{}.each { |(x,  y),  z| puts x }\n",
        &[(1, 15, EXTRA), (1, 20, EXTRA)],
        "{}.each { |(x, y), z| puts x }\n",
    ),
    (
        373,
        "{}.each { |  x=5,  (y,*z)|puts x }\n",
        &[
            (1, 12, BEFORE),
            (1, 12, EXTRA),
            (1, 18, EXTRA),
            (1, 26, CLOSING),
        ],
        "{}.each { |x=5, (y,*z)| puts x }\n",
    ),
    (388, "{}.each { |x, z| puts x }\n", &[], ""),
    (
        400,
        "->(  x,  y) { puts x }\n",
        &[(1, 4, EXTRA), (1, 8, EXTRA)],
        "->( x, y) { puts x }\n",
    ),
    (6, "{}.each {}", &[], ""),
    (10, "->() { puts \"a\" }", &[], ""),
    (14, "->a { puts a }", &[], ""),
    (
        353,
        "{}.each { | x, | puts x }",
        &[(1, 12, BEFORE), (1, 15, AFTER)],
        "{}.each { |x,| puts x }",
    ),
    (
        357,
        "{}.each { | x , | puts x }",
        &[(1, 12, BEFORE), (1, 16, AFTER)],
        "{}.each { |x ,| puts x }",
    ),
    (
        361,
        "{}.each { | x,| puts x }\n",
        &[(1, 12, BEFORE)],
        "{}.each { |x,| puts x }\n",
    ),
    (423, "->(x, y) { puts x }", &[], ""),
    (433, "->(x, y) { puts x }", &[], ""),
    (443, "->(x, y) { puts x }", &[], ""),
    (447, "{}.each { |x, y| puts x }\n", &[], ""),
    (469, "->( x, y ) { puts x }", &[], ""),
    (479, "->( x, y ) { puts x }", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 51);
    for &(line, source, rows, corrected) in PINNED {
        assert_eq!(
            findings(source),
            expected(rows),
            "spec line {line} {source:?}"
        );
        if rows.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceAroundBlockParameters::default(), source).is_empty());
        } else {
            assert_correction(&SpaceAroundBlockParameters::default(), source, corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceAroundBlockParameters::default();
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn every_parameter_kind_and_destructuring() {
    for (source, rows, after) in [
        (
            "f { |a,  b = 1,  *c,  d,  e:,  f: 1,  **g,  &h| }\n",
            &[
                (1, 8, EXTRA),
                (1, 16, EXTRA),
                (1, 21, EXTRA),
                (1, 25, EXTRA),
                (1, 30, EXTRA),
                (1, 37, EXTRA),
                (1, 43, EXTRA),
            ][..],
            "f { |a, b = 1, *c, d, e:, f: 1, **g, &h| }\n",
        ),
        (
            "f { |a,  **nil| }\n",
            &[(1, 8, EXTRA)],
            "f { |a, **nil| }\n",
        ),
        (
            "f { |*,  **,  &| }\n",
            &[(1, 8, EXTRA), (1, 13, EXTRA)],
            "f { |*, **, &| }\n",
        ),
        (
            "f { |(a,  (b,  *c)),  d| }\n",
            &[(1, 9, EXTRA), (1, 14, EXTRA), (1, 21, EXTRA)],
            "f { |(a, (b, *c)), d| }\n",
        ),
        (
            "f { |(a,  *),  b| }\n",
            &[(1, 9, EXTRA), (1, 14, EXTRA)],
            "f { |(a, *), b| }\n",
        ),
        // Block-local variables are parameters too.
        (
            "f { |a;  b,  c | }\n",
            &[(1, 8, EXTRA), (1, 12, EXTRA), (1, 15, AFTER)],
            "f { |a; b, c| }\n",
        ),
        (
            "f { | (a, b),  c| }\n",
            &[(1, 6, BEFORE), (1, 14, EXTRA)],
            "f { |(a, b), c| }\n",
        ),
        ("f { |a  , | }\n", &[(1, 10, AFTER)], "f { |a  ,| }\n"),
        (
            "f { |\ta\t| a }\n",
            &[(1, 6, BEFORE), (1, 8, AFTER)],
            "f { |a| a }\n",
        ),
        ("f do |a|a end\n", &[(1, 8, CLOSING)], "f do |a| a end\n"),
        (
            "f do |a|rescue; end\n",
            &[(1, 8, CLOSING)],
            "f do |a| rescue; end\n",
        ),
        (
            "f do |a|ensure; end\n",
            &[(1, 8, CLOSING)],
            "f do |a| ensure; end\n",
        ),
        (
            "f do |a|a\nrescue\nensure\nend\n",
            &[(1, 8, CLOSING)],
            "f do |a| a\nrescue\nensure\nend\n",
        ),
        (
            "lambda do | a,  b |a end\n",
            &[
                (1, 12, BEFORE),
                (1, 15, EXTRA),
                (1, 18, AFTER),
                (1, 19, CLOSING),
            ],
            "lambda do |a, b| a end\n",
        ),
        ("super { | a| }\n", &[(1, 10, BEFORE)], "super { |a| }\n"),
        (
            "a.b&.c(1) { | x| }\n",
            &[(1, 14, BEFORE)],
            "a.b&.c(1) { |x| }\n",
        ),
        (
            "f { |a|\n  g { | b|b }\n}\n",
            &[(2, 8, BEFORE), (2, 10, CLOSING)],
            "f { |a|\n  g { |b| b }\n}\n",
        ),
        (
            "->(a,  b) do a end\n",
            &[(1, 6, EXTRA)],
            "->(a, b) do a end\n",
        ),
        (
            "x = é.map { | y| y }\n",
            &[(1, 14, BEFORE)],
            "x = é.map { |y| y }\n",
        ),
        (
            "\u{feff}f { | a| }\n",
            &[(1, 6, BEFORE)],
            "\u{feff}f { |a| }\n",
        ),
        (
            "f { | a|a }\r\n",
            &[(1, 6, BEFORE), (1, 8, CLOSING)],
            "f { |a| a }\r\n",
        ),
    ] {
        assert_eq!(findings(source), expected(rows), "{source:?}");
        assert_correction(&SpaceAroundBlockParameters::default(), source, after);
    }
}

#[test]
fn lambda_parentheses_follow_configured_space_inside_parens() {
    let off = SpaceAroundBlockParameters {
        space_inside_parens: false,
    };
    for (source, rows, after) in [
        (
            "->( x, y) { puts x }\n",
            &[(1, 4, BEFORE)][..],
            "->(x, y) { puts x }\n",
        ),
        (
            "->(x, y  ) { puts x }\n",
            &[(1, 8, AFTER)],
            "->(x, y) { puts x }\n",
        ),
        (
            "->(  a,  b, c) { puts a }\n",
            &[(1, 4, BEFORE), (1, 4, EXTRA), (1, 8, EXTRA)],
            "->(a, b, c) { puts a }\n",
        ),
        (
            "->( a; b ) {}\n",
            &[(1, 4, BEFORE), (1, 9, AFTER)],
            "->(a; b) {}\n",
        ),
        // Pipes are checked either way.
        (
            "{}.each { | x| puts x }\n",
            &[(1, 12, BEFORE)],
            "{}.each { |x| puts x }\n",
        ),
    ] {
        assert_eq!(findings_with(&off, source), expected(rows), "{source:?}");
        let offenses = correction_offenses(&off, source);
        let applied = crate::corrections::apply(source.as_bytes(), &offenses).unwrap();
        assert_eq!(String::from_utf8(applied.bytes).unwrap(), after);
    }
    for source in [
        "->( x, y ) { puts x }\n",
        "->a { a }\n",
        "-> a, b { a }\n",
        "->( ) { }\n",
    ] {
        let rows: &[(u32, u32, &str)] = if source.starts_with("->( x") {
            &[(1, 4, BEFORE), (1, 9, AFTER)]
        } else {
            &[]
        };
        assert_eq!(findings_with(&off, source), expected(rows), "{source:?}");
        assert!(findings(source).is_empty(), "{source:?}");
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "f { |a| a }\n",
        "f { |a, b| }\n",
        "f { || a }\n",
        "f { | | a }\n",
        "f {}\n",
        "f { |a,b| a }\n",
        "f do |a|\n  a\nend\n",
        "f do |a| a end\n",
        "f { |a|;a }\n",
        "f do |a| rescue; end\n",
        "f do |a|\nensure\nend\n",
        "f do |a|; rescue; end\n",
        // Numbered parameters and `it` have no parameter list.
        "f { _1 + _2 }\n",
        "f { it }\n",
        "-> { _1 }\n",
        // Line breaks inside the pipes are Layout/MultilineBlockLayout's.
        "f { |\n  a,\n  b\n| a }\n",
        "f { |a,\n     b| a }\n",
        "f { |a, # c\n  b| a }\n",
        // A lambda's parentheses belong to Layout/SpaceInsideParens.
        "->( a ) { a }\n",
        "-> ( a ) { a }\n",
        "->a { a }\n",
        "->( ) { }\n",
        // Not block parameters.
        "def f( a,  b) = a\n",
        "f( a,  b)\n",
        "x = a | b |c\n",
        "for a,  b in [] do end\n",
        "case 1\nin [a,  b] then a\nend\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn a_shadow_local_semicolon_is_reported_without_an_edit() {
    // RuboCop reports the `;` before the only block-local variable as a
    // space and would delete it, turning the local into a parameter.
    let source = "f { |;a| a = 1 }\n";
    assert_eq!(findings(source), expected(&[(1, 6, BEFORE)]));
    held(source);
    let source = "f { |; a| }\n";
    assert_eq!(findings(source), expected(&[(1, 6, BEFORE)]));
    held(source);
}
