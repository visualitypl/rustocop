//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_after_method_name_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 8 pinned examples
//! run at the fixed defaults; findings, messages and single-pass corrected
//! bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (each source and corrected
//! SHA-256 was checked against it when the rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceAfterMethodName";

fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str_with(&[&SpaceAfterMethodName], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect()
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceAfterMethodName, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (
        5,
        "def func (x)\n  a\nend\n",
        &[(1, 9)],
        "def func(x)\n  a\nend\n",
    ),
    (
        20,
        "def self.func (x)\n  a\nend\n",
        &[(1, 14)],
        "def self.func(x)\n  a\nend\n",
    ),
    (
        35,
        "def func= (x)\n  a\nend\n",
        &[(1, 10)],
        "def func=(x)\n  a\nend\n",
    ),
    (50, "def func\n  a\nend\n", &[], ""),
    (58, "def self.func\n  a\nend\n", &[], ""),
    (66, "def func x\n  a\nend\n", &[], ""),
    (74, "def self.func x\n  a\nend\n", &[], ""),
    (82, "def func= x\n  a\nend\n", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 8);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceAfterMethodName, source).is_empty());
        } else {
            assert_correction(&SpaceAfterMethodName, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    assert!(SpaceAfterMethodName.default_enabled());
    assert!(SpaceAfterMethodName.department_selection_enables());
    assert!(SpaceAfterMethodName.safe_fixes());
    assert!(
        SpaceAfterMethodName
            .autocorrect_incompatible_with()
            .is_empty()
    );
}

#[test]
fn every_parenthesized_definition() {
    for (source, expected, after) in [
        ("def f ()\nend\n", (1, 6), "def f()\nend\n"),
        ("def f (x) = x\n", (1, 6), "def f(x) = x\n"),
        ("def f? (x); end\n", (1, 7), "def f?(x); end\n"),
        ("def + (x); end\n", (1, 6), "def +(x); end\n"),
        ("def []= (k, v); end\n", (1, 8), "def []=(k, v); end\n"),
        ("def a.f (x); end\n", (1, 8), "def a.f(x); end\n"),
        ("def self.é (x); end\n", (1, 11), "def self.é(x); end\n"),
        (
            "class A\n  def f (x)\n  end\nend\n",
            (2, 8),
            "class A\n  def f(x)\n  end\nend\n",
        ),
        // One space per pass, as upstream.
        ("def f  (x); end\n", (1, 7), "def f (x); end\n"),
        (
            "\u{feff}def f (x); end\n",
            (1, 6),
            "\u{feff}def f(x); end\n",
        ),
        ("def f (x)\r\nend\r\n", (1, 6), "def f(x)\r\nend\r\n"),
    ] {
        assert_eq!(findings(source), [expected], "{source:?}");
        assert_correction(&SpaceAfterMethodName, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "def f(x); end\n",
        "def f\t(x); end\n",
        "def f \\\n(x); end\n",
        "def f x, y; end\n",
        "def f = 1\n",
        "f (x)\n",
        "f.g (x)\n",
        "define_method (:f) { }\n",
        "x = -> (a) { a }\n",
        "f { |x| }\n",
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
        held(&format!("def f (x); end\n{observer}\n"));
    }
    assert_correction(
        &SpaceAfterMethodName,
        "def f (x); end\np File.read('example.rb')\n",
        "def f(x); end\np File.read('example.rb')\n",
    );
}
