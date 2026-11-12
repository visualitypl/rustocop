//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_inside_range_literal_spec.rb` at 97f30c4
//! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). All 7 pinned
//! examples; findings, messages and single-pass corrected bytes equal the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! source and corrected SHA-256 was checked against it when the rows were
//! ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceInsideRangeLiteral";

fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceInsideRangeLiteral], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col)
        })
        .collect();
    found.sort_unstable();
    found
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceInsideRangeLiteral, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Pinned = (u32, &'static str, &'static [(u32, u32)], &'static str);

const PINNED: &[Pinned] = &[
    (
        5,
        "1 .. 2\n1.. 2\n1 ..2\n",
        &[(1, 1), (2, 1), (3, 1)],
        "1..2\n1..2\n1..2\n",
    ),
    (22, "1..2", &[], ""),
    (
        26,
        "1 ... 2\n1... 2\n1 ...2\n",
        &[(1, 1), (2, 1), (3, 1)],
        "1...2\n1...2\n1...2\n",
    ),
    (43, "1...2", &[], ""),
    (47, "0...(line - 1)", &[], ""),
    (51, "x = 0..\n    10\n", &[], ""),
    (58, "x = 0 ..\n    10\n", &[(1, 5)], "x = 0..10\n"),
];

#[test]
fn pinned_examples_match_the_fixed_replay() {
    assert_eq!(PINNED.len(), 7);
    for &(line, source, expected, corrected) in PINNED {
        assert_eq!(findings(source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceInsideRangeLiteral, source).is_empty());
        } else {
            assert_correction(&SpaceInsideRangeLiteral, source, corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceInsideRangeLiteral;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn every_range_shape_is_checked_and_corrected() {
    for (source, expected, after) in [
        ("x = (.. 1)\n", &[(1, 6)][..], "x = (..1)\n"),
        ("x = (1 ..)\n", &[(1, 6)], "x = (1..)\n"),
        ("x = (1 ...)\n", &[(1, 6)], "x = (1...)\n"),
        ("x = 'a' .. 'z'\n", &[(1, 5)], "x = 'a'..'z'\n"),
        ("x = 1\t..\t2\n", &[(1, 5)], "x = 1..2\n"),
        ("x = a .. b\n", &[(1, 5)], "x = a..b\n"),
        ("p 1 .. 2\n", &[(1, 3)], "p 1..2\n"),
        ("p(1 ..)\n", &[(1, 3)], "p(1..)\n"),
        (
            "x = [1 .. 2, 3... 4]\n",
            &[(1, 6), (1, 14)],
            "x = [1..2, 3...4]\n",
        ),
        (
            "case 3\nin 1 .. 5 then p 1\nend\n",
            &[(2, 4)],
            "case 3\nin 1..5 then p 1\nend\n",
        ),
        // Both ranges are reported; each edit touches its own operator.
        (
            "x = (1 .. 2) .. (3 .. 4)\n",
            &[(1, 5), (1, 6), (1, 18)],
            "x = (1..2)..(3..4)\n",
        ),
        ("x = 0 ..\n\n  10\n", &[(1, 5)], "x = 0..10\n"),
        ("x = 0 ..\r\n  10\r\n", &[(1, 5)], "x = 0..10\r\n"),
        // A comment after the operator stays; upstream's bytes.
        ("x = 0 .. # c\n  10\n", &[(1, 5)], "x = 0..# c\n  10\n"),
        ("x = 0 .. \\\n  10\n", &[(1, 5)], "x = 0..\\\n  10\n"),
        ("x = é .. 2\n", &[(1, 5)], "x = é..2\n"),
        ("\u{feff}x = 1 .. 2\n", &[(1, 5)], "\u{feff}x = 1..2\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        assert_correction(&SpaceInsideRangeLiteral, source, after);
    }
}

#[test]
fn the_upstream_text_test_is_kept() {
    // Upstream tests the node's source text, so a blank next to an operator
    // spelled inside an operand is reported too. No blank sits at the
    // actual operator: the finding has no edit.
    for source in [
        "x = 'a .. b'..'z'\n",
        "x = (1 ...2)..3\n",
        "x = [1 .. 2]...[3]\n",
    ] {
        let reported = findings(source);
        assert!(!reported.is_empty(), "{source:?}");
    }
    held("x = 'a .. b'..'z'\n");
    assert_correction(
        &SpaceInsideRangeLiteral,
        "x = 'a .. b' .. 'z'\n",
        "x = 'a .. b'..'z'\n",
    );
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = 1..2\n",
        "x = (..1)\n",
        "x = (1..)\n",
        "x = 0..\n  10\n",
        "x = 0...\n  10\n",
        "if a .. b then 1 end\n",
        "if a ... b then 1 end\n",
        "x = 'a .. b'\n",
        "# 1 .. 2\n",
        "def f(...) = g(...)\n",
        "x = 1.0..2.5\n",
        "x = a&.b..c\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn uncertain_joins_keep_the_finding_without_an_edit() {
    // A line continuation before the operator cannot be removed alone.
    held("x = 0 \\\n  .. 10\np x\n");
    // `...` at the end of a line warns; joining would drop the warning.
    held("x = 0 ...\n  10\np x\n");
    held("x = 0 ...\n\n  10\n");
    let found = lint_str_with(&[&SpaceInsideRangeLiteral], "x = 0 ...\n  10\n");
    assert_eq!((found[0].line, found[0].end_line), (1, 2));
    // A line break followed by anything but the operand: a comment line,
    // or a heredoc body between the operator and the operand.
    held("x = 0 ..\n  # c\n  10\n");
    held("x = <<~A ..\n  body\nA\n  10\n");
    // A comment after `...` keeps the warning: only the blank goes.
    assert_correction(
        &SpaceInsideRangeLiteral,
        "x = 0 ... # c\n  10\n",
        "x = 0...# c\n  10\n",
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
        "p method(:p).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\nx",
    ] {
        held(&format!("x = 1 .. 2\n{observer}\n"));
    }
    assert_correction(
        &SpaceInsideRangeLiteral,
        "x = 1 .. 2\np File.read('example.rb')\n",
        "x = 1..2\np File.read('example.rb')\n",
    );
}
