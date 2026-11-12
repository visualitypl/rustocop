//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_after_colon_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 12 pinned examples
//! (the `Ruby >= 3.1` value-omission context included) run at the fixed
//! Ruby 3.4; findings, messages and single-pass corrected bytes equal the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source and corrected SHA-256 was checked against it when the rows were
//! ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceAfterColon";

/// `(line, col)` of each finding, checking message, severity and that no
/// edit is offered without a correction request.
fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str_with(&[&SpaceAfterColon], source)
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

/// Every finding is kept and no edit is offered.
fn held(source: &str) {
    let found = correction_offenses(&SpaceAfterColon, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (5, "{a:3}\n", &[(1, 3)], "{a: 3}\n"),
    (16, "x = :a", &[], ""),
    (20, "x = w ? a : b", &[], ""),
    (24, "{a: 3}", &[], ""),
    (28, "x = {\"a\"=>1}", &[], ""),
    (32, "x = if w\n      a\n    end\n", &[], ""),
    (40, "str << ':'", &[], ""),
    (44, "def f(x:, y:)\nend\n", &[], ""),
    (51, "def initialize(table:, nodes:)\nend\n", &[], ""),
    (
        58,
        "def m(var:1, other_var: 2)\nend\n",
        &[(1, 10)],
        "def m(var: 1, other_var: 2)\nend\n",
    ),
    (72, "{x:, y:}", &[], ""),
    (76, "foo(table:, nodes:)\n", &[], ""),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 12);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceAfterColon, source).is_empty());
        } else {
            assert_correction(&SpaceAfterColon, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    assert!(SpaceAfterColon.default_enabled());
    assert!(SpaceAfterColon.department_selection_enables());
    assert!(SpaceAfterColon.safe_fixes());
    assert!(SpaceAfterColon.autocorrect_incompatible_with().is_empty());
}

#[test]
fn every_colon_pair_and_optional_keyword_parameter() {
    for (source, expected, after) in [
        ("foo(a:1)\n", (1, 6), "foo(a: 1)\n"),
        ("foo a:1\n", (1, 6), "foo a: 1\n"),
        ("x = {\"a\":1}\n", (1, 9), "x = {\"a\": 1}\n"),
        ("x = {'a':1}\n", (1, 9), "x = {'a': 1}\n"),
        ("x = {\"#{y}\":1}\n", (1, 12), "x = {\"#{y}\": 1}\n"),
        ("x = {a:-1}\n", (1, 7), "x = {a: -1}\n"),
        ("x = {a:?b}\n", (1, 7), "x = {a: ?b}\n"),
        ("x = {a:/b/}\n", (1, 7), "x = {a: /b/}\n"),
        ("x = {a:%w[b]}\n", (1, 7), "x = {a: %w[b]}\n"),
        ("x = {a:->{}}\n", (1, 7), "x = {a: ->{}}\n"),
        ("x = {a:<<~A}\n  b\nA\n", (1, 7), "x = {a: <<~A}\n  b\nA\n"),
        ("x = {é:1}\n", (1, 7), "x = {é: 1}\n"),
        ("x = {a:\\\n1}\n", (1, 7), "x = {a: \\\n1}\n"),
        (
            "case x\nin {a:1}\nend\n",
            (2, 6),
            "case x\nin {a: 1}\nend\n",
        ),
        ("def m(a:1) = a\n", (1, 8), "def m(a: 1) = a\n"),
        ("def self.m(a:1); end\n", (1, 13), "def self.m(a: 1); end\n"),
        ("f { |a:1| a }\n", (1, 7), "f { |a: 1| a }\n"),
        ("f = ->(a:1) { a }\n", (1, 9), "f = ->(a: 1) { a }\n"),
        ("\u{feff}x = {a:1}\n", (1, 7), "\u{feff}x = {a: 1}\n"),
        ("x = {a:1}\r\n", (1, 7), "x = {a: 1}\r\n"),
    ] {
        assert_eq!(findings(source), [expected], "{source:?}");
        assert_correction(&SpaceAfterColon, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = {a:\n1}\n",
        "x = {a:\t1}\n",
        "x = {a:\x0b1}\n",
        "x = {a:\x0c1}\n",
        "x = {a:\r\n1}\n",
        "x = {a: # c\n1}\n",
        "x = {a:, b:}\n",
        "foo(a:)\n",
        "case x\nin {a:}\nend\n",
        "case x\nin {a:, **rest}\nend\n",
        "x = {:a=>1}\n",
        "x = {\"a\" => 1}\n",
        "x = A::B\n",
        "x = :\"a:b\"\n",
        "x = %i[a:b]\n",
        "x = \"a:b\"\n",
        "# a:b\n",
        "def m(a:, b:); end\n",
        "def m(**a); end\n",
        "f { |a:| a }\n",
        "x = /a:b/\n",
        "x = ?:\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn every_offense_of_a_file_is_corrected_in_one_pass() {
    let source = "def m(a:1, b:2)\n  {c:3, \"d\":4, e:}\nend\n";
    assert_eq!(findings(source), [(1, 8), (1, 13), (2, 5), (2, 12)]);
    assert_correction(
        &SpaceAfterColon,
        source,
        "def m(a: 1, b: 2)\n  {c: 3, \"d\": 4, e:}\nend\n",
    );
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
        held(&format!("x = {{a:1}}\n{observer}\n"));
    }
    // Out-of-scope readers do not hold the edit.
    assert_correction(
        &SpaceAfterColon,
        "x = {a:1}\np File.read('example.rb')\n",
        "x = {a: 1}\np File.read('example.rb')\n",
    );
}
