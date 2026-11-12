//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_around_method_call_operator_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 51
//! pinned examples (the shared `offense` examples included) run at the
//! fixed defaults; findings, messages and single-pass corrected bytes equal
//! the fixed replay recorded in the pinned RuboCop reference run
//! (each source and corrected SHA-256 was checked against it when the rows
//! were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceAroundMethodCallOperator";

fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceAroundMethodCallOperator], source)
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
    found.sort_unstable();
    found
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceAroundMethodCallOperator, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings. Shared `offense` examples
/// carry the shared definition's line, 6.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (6, "foo. bar\n", &[(1, 5)], "foo.bar\n"),
    (6, "foo .bar\n", &[(1, 4)], "foo.bar\n"),
    (6, "foo  .bar\n", &[(1, 4)], "foo.bar\n"),
    (6, "foo.  bar\n", &[(1, 5)], "foo.bar\n"),
    (6, "foo . bar\n", &[(1, 4), (1, 6)], "foo.bar\n"),
    (6, "foo. ()\n", &[(1, 5)], "foo.()\n"),
    (6, "foo\n  . bar\n", &[(2, 4)], "foo\n  .bar\n"),
    (6, "foo .\n  bar\n", &[(1, 4)], "foo.\n  bar\n"),
    (76, "foo\n  .bar\n", &[], ""),
    (84, "'foo'.bar\n", &[], ""),
    (6, "foo.bar. buzz\n", &[(1, 9)], "foo.bar.buzz\n"),
    (6, "foo. bar.buzz\n", &[(1, 5)], "foo.bar.buzz\n"),
    (6, "foo .bar.buzz\n", &[(1, 4)], "foo.bar.buzz\n"),
    (6, "foo.bar .buzz\n", &[(1, 8)], "foo.bar.buzz\n"),
    (6, "foo.bar .buzz.bat\n", &[(1, 8)], "foo.bar.buzz.bat\n"),
    (
        6,
        "foo. bar. buzz.bat\n",
        &[(1, 5), (1, 10)],
        "foo.bar.buzz.bat\n",
    ),
    (133, "foo.bar.buzz\n", &[], ""),
    (140, "foo. # comment\n  bar.baz\n", &[], ""),
    (
        147,
        "foo . # comment\n  bar.baz\n",
        &[(1, 4)],
        "foo. # comment\n  bar.baz\n",
    ),
    (6, "foo&. bar\n", &[(1, 6)], "foo&.bar\n"),
    (6, "foo &.bar\n", &[(1, 4)], "foo&.bar\n"),
    (6, "foo  &.bar\n", &[(1, 4)], "foo&.bar\n"),
    (6, "foo&.  bar\n", &[(1, 6)], "foo&.bar\n"),
    (6, "foo &. bar\n", &[(1, 4), (1, 7)], "foo&.bar\n"),
    (6, "foo\n  &. bar\n", &[(2, 5)], "foo\n  &.bar\n"),
    (6, "foo &.\n  bar\n", &[(1, 4)], "foo&.\n  bar\n"),
    (218, "foo\n  &.bar\n", &[], ""),
    (226, "foo&.bar\n", &[], ""),
    (6, "foo&.bar&. buzz\n", &[(1, 11)], "foo&.bar&.buzz\n"),
    (6, "foo&. bar&.buzz\n", &[(1, 6)], "foo&.bar&.buzz\n"),
    (6, "foo &.bar&.buzz\n", &[(1, 4)], "foo&.bar&.buzz\n"),
    (6, "foo&.bar &.buzz\n", &[(1, 9)], "foo&.bar&.buzz\n"),
    (
        6,
        "foo&.bar &.buzz&.bat\n",
        &[(1, 9)],
        "foo&.bar&.buzz&.bat\n",
    ),
    (
        6,
        "foo&. bar&. buzz&.bat\n",
        &[(1, 6), (1, 12)],
        "foo&.bar&.buzz&.bat\n",
    ),
    (275, "foo&.bar&.buzz\n", &[], ""),
    (6, "RuboCop:: Cop\n", &[(1, 10)], "RuboCop::Cop\n"),
    (6, "RuboCop::  Cop\n", &[(1, 10)], "RuboCop::Cop\n"),
    (6, "RuboCop\n  :: Cop\n", &[(2, 5)], "RuboCop\n  ::Cop\n"),
    (307, "RuboCop\n  ::Cop\n", &[], ""),
    (315, "RuboCop::Cop\n", &[], ""),
    (6, "RuboCop::Cop:: Cop\n", &[(1, 15)], "RuboCop::Cop::Cop\n"),
    (
        6,
        "RuboCop::Cop:: Cop::Cop\n",
        &[(1, 15)],
        "RuboCop::Cop::Cop::Cop\n",
    ),
    (
        6,
        ":: RuboCop:: Cop:: Cop::Cop\n",
        &[(1, 3), (1, 13), (1, 19)],
        "::RuboCop::Cop::Cop::Cop\n",
    ),
    (
        6,
        "klass = :: RuboCop::Cop\n",
        &[(1, 11)],
        "klass = ::RuboCop::Cop\n",
    ),
    (351, "RuboCop::Cop::Cop\n", &[], ""),
    (357, "klass = ::RuboCop::Cop\n", &[], ""),
    (363, "class Test <  ::RuboCop::Cop\nend\n", &[], ""),
    (370, "::RuboCop::Cop || ::RuboCop\n", &[], ""),
    (
        6,
        ":: RuboCop:: Cop || :: RuboCop\n",
        &[(1, 3), (1, 13), (1, 23)],
        "::RuboCop::Cop || ::RuboCop\n",
    ),
    (386, "'foo' + 'bar'\n", &[], ""),
    (392, "__ENCODING__\n", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 51);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceAroundMethodCallOperator, source).is_empty());
        } else {
            assert_correction(&SpaceAroundMethodCallOperator, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceAroundMethodCallOperator;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
}

#[test]
fn every_dotted_call_and_constant_path() {
    for (source, expected, after) in [
        ("a .b += 1\n", &[(1, 2)][..], "a.b += 1\n"),
        ("a. b ||= 1\n", &[(1, 3)], "a.b ||= 1\n"),
        ("a &. b &&= 1\n", &[(1, 2), (1, 5)], "a&.b &&= 1\n"),
        ("a .b, c = 1, 2\n", &[(1, 2)], "a.b, c = 1, 2\n"),
        ("for a. b in [1]; end\n", &[(1, 7)], "for a.b in [1]; end\n"),
        ("a\t.b\n", &[(1, 2)], "a.b\n"),
        ("a. \tb\n", &[(1, 3)], "a.b\n"),
        ("a. b = 1\n", &[(1, 3)], "a.b = 1\n"),
        ("a .[](1)\n", &[(1, 2)], "a.[](1)\n"),
        ("a. (1)\n", &[(1, 3)], "a.(1)\n"),
        ("a do end .b\n", &[(1, 9)], "a do end.b\n"),
        ("<<~A .b\n  x\nA\n", &[(1, 5)], "<<~A.b\n  x\nA\n"),
        ("1 .succ\n", &[(1, 2)], "1.succ\n"),
        ("-1 .abs\n", &[(1, 3)], "-1.abs\n"),
        ("?a .b\n", &[(1, 3)], "?a.b\n"),
        ("a? .b\n", &[(1, 3)], "a?.b\n"),
        ("x = A:: B::\tC\n", &[(1, 8), (1, 12)], "x = A::B::C\n"),
        ("class A:: B; end\n", &[(1, 10)], "class A::B; end\n"),
        (
            "case x\nin A:: B\nend\n",
            &[(2, 7)],
            "case x\nin A::B\nend\n",
        ),
        ("A::B = C:: D\n", &[(1, 11)], "A::B = C::D\n"),
        ("A:: B::C = 1\n", &[(1, 4)], "A::B::C = 1\n"),
        // A `::` after a blank keeps that blank: it decides how `::` lexes.
        ("a = 1\na :: B\n", &[(2, 5)], "a = 1\na ::B\n"),
        ("é. b\n", &[(1, 3)], "é.b\n"),
        ("\u{feff}a .b\n", &[(1, 2)], "\u{feff}a.b\n"),
        ("a .b\r\n", &[(1, 2)], "a.b\r\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        assert_correction(&SpaceAroundMethodCallOperator, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "a.b\n",
        "a&.b\n",
        "a::b\n",
        "A::B\n",
        "::A\n",
        "a\n  .b\n",
        "a.\n  b\n",
        "a&.\n  b\n",
        "a. # c\n  b\n",
        "a \\\n.b\n",
        "a.\\\n  b\n",
        // `casgn` targets are not `const` nodes.
        "A:: B = 1\n",
        "A:: B ||= 1\n",
        "A:: B += 1\n",
        "A:: B, c = 1, 2\n",
        // Not method call operators.
        "a ? b : c\n",
        "1 .. 2\n",
        "x = :a\n",
        "x = \"a . b\"\n",
        "# a . b\n",
        "__ENCODING__\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn listed_observers_hold_every_edit() {
    for observer in [
        "p __FILE__",
        "p __dir__",
        "p $0",
        "p $PROGRAM_NAME",
        "p caller",
        "p caller_locations",
        "p method(:f).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\nx",
    ] {
        held(&format!("a . b\nA:: B\n{observer}\n"));
    }
    assert_correction(
        &SpaceAroundMethodCallOperator,
        "a . b\np File.read('example.rb')\n",
        "a.b\np File.read('example.rb')\n",
    );
}
