//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_after_not_spec.rb` at 97f30c4 (MIT
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

const COP: &str = "Layout/SpaceAfterNot";

/// `(line, col, end line)` of each finding.
fn findings(source: &str) -> Vec<(u32, u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    lint_str_with(&[&SpaceAfterNot], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col, o.end_line)
        })
        .collect()
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceAfterNot, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (5, "! something\n", &[(1, 1)], "!something\n"),
    (16, "!   something\n", &[(1, 1)], "!something\n"),
    (27, "!\nsomething\n", &[(1, 1)], "!something\n"),
    (39, "!something", &[], ""),
    (43, "not something", &[], ""),
    (47, "! ~something", &[], ""),
    (
        51,
        "! (~something).zero?\n",
        &[(1, 1)],
        "!(~something).zero?\n",
    ),
    (63, "! (model)\n", &[(1, 1)], "!(model)\n"),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 8);
    for &(line, source, expected, corrected) in PINNED {
        let found: Vec<_> = findings(source)
            .into_iter()
            .map(|(line, col, _)| (line, col))
            .collect();
        assert_eq!(found, expected, "spec line {line}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceAfterNot, source).is_empty());
        } else {
            assert_correction(&SpaceAfterNot, source, corrected);
        }
    }
    // The newline example's highlight ends on the receiver's line.
    assert_eq!(findings("!\nsomething\n"), [(1, 1, 2)]);
}

#[test]
fn metadata() {
    assert!(SpaceAfterNot.default_enabled());
    assert!(SpaceAfterNot.department_selection_enables());
    assert!(SpaceAfterNot.safe_fixes());
    assert!(SpaceAfterNot.autocorrect_incompatible_with().is_empty());
}

#[test]
fn every_prefix_bang_with_a_gap() {
    for (source, expected, after) in [
        ("x = ! a\n", (1, 5, 1), "x = !a\n"),
        ("x = !\ta\n", (1, 5, 1), "x = !a\n"),
        ("x = !\\\n  a\n", (1, 5, 2), "x = !a\n"),
        ("x = !\r\n  a\r\n", (1, 5, 2), "x = !a\r\n"),
        ("x = ! !a\n", (1, 5, 1), "x = !!a\n"),
        ("x = ! -a\n", (1, 5, 1), "x = !-a\n"),
        ("x = ! @a\n", (1, 5, 1), "x = !@a\n"),
        ("x = ! a.b(1)\n", (1, 5, 1), "x = !a.b(1)\n"),
        ("x = ! a do\nend\n", (1, 5, 2), "x = !a do\nend\n"),
        ("x = ! é\n", (1, 5, 1), "x = !é\n"),
        ("if ! a then end\n", (1, 4, 1), "if !a then end\n"),
        ("\u{feff}! a\n", (1, 1, 1), "\u{feff}!a\n"),
    ] {
        assert_eq!(findings(source), [expected], "{source:?}");
        assert_correction(&SpaceAfterNot, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = !a\n",
        "x = not a\n",
        "x = not(a)\n",
        "x = a.!\n",
        "x = a.! ()\n",
        "x = a != b\n",
        "x = a !~ b\n",
        "x = ! ~a\n",
        "x = ! ~ a\n",
        "x = ! ~a.b\n",
        // `!` binds tighter than a binary operator: the operand is `~a`.
        "x = ! ~a + 1\n",
        "x = ! ~a ** 2\n",
        "x = !(a)\n",
        "x = \"! a\"\n",
        "# ! a\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn uncertain_gaps_keep_the_finding() {
    for source in [
        // A comment or an embedded document between `!` and the receiver.
        "x = ! # c\n  a\n",
        "x = !\n=begin\nc\n=end\n  a\n",
    ] {
        held(source);
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
        held(&format!("x = ! a\n{observer}\n"));
    }
    assert_correction(
        &SpaceAfterNot,
        "x = ! a\np File.read('example.rb')\n",
        "x = !a\np File.read('example.rb')\n",
    );
}
