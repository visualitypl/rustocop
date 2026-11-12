//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_inside_array_percent_literal_spec.rb` at
//! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The spec's
//! loops over the `i`, `I`, `w` and `W` types and the `{}`, `()`, `[]` and
//! `!!` delimiters are generated here the same way: all 129 pinned
//! examples. Findings, messages and single-pass corrected bytes equal the
//! fixed replay recorded in the pinned RuboCop reference run (each
//! generated source and corrected SHA-256 was checked against it when the
//! rows were ported).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceInsideArrayPercentLiteral";

fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceInsideArrayPercentLiteral], source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.message, MSG);
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert!(o.correction.is_none());
            assert_eq!(o.line, o.end_line);
            (o.line, o.col)
        })
        .collect();
    found.sort_unstable();
    found
}

fn held(source: &str) {
    let found = correction_offenses(&SpaceInsideArrayPercentLiteral, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, single-pass correction)`; an empty
/// correction means the source has no findings.
type Row = (u32, String, Vec<(u32, u32)>, String);

/// The spec's examples in its order: the type and delimiter loops, then
/// the non-array literal.
fn pinned() -> Vec<Row> {
    let mut rows = Vec::new();
    for ty in ["i", "I", "w", "W"] {
        for (l, r) in [("{", "}"), ("(", ")"), ("[", "]"), ("!", "!")] {
            let code = |content: &str| format!("%{ty}{l}{content}{r}");
            rows.extend([
                (
                    10,
                    format!("{}\n", code("1   2")),
                    vec![(1, 5)],
                    format!("{}\n", code("1 2")),
                ),
                (
                    19,
                    format!("{}\n", code("1   2   3")),
                    vec![(1, 5), (1, 9)],
                    format!("{}\n", code("1 2 3")),
                ),
                (
                    29,
                    format!("{}\n", code("a\\   b \\ c")),
                    vec![(1, 7)],
                    format!("{}\n", code("a\\  b \\ c")),
                ),
                (38, code("a b c"), vec![], String::new()),
                (42, code("a\\  b\\ \\  c"), vec![], String::new()),
                (
                    46,
                    format!("%{ty}(\n  a\n  b\n  c\n)\n"),
                    vec![],
                    String::new(),
                ),
                (
                    56,
                    format!("def foo\n  %{ty}(\n    a\n    b\n    c\n  )\nend\n"),
                    vec![],
                    String::new(),
                ),
                (68, format!("%{ty}(a b\n   c)\n"), vec![], String::new()),
            ]);
        }
    }
    rows.push((78, "%q( a  b c )".into(), vec![], String::new()));
    rows
}

#[test]
fn pinned_examples_match_the_fixed_replay() {
    let rows = pinned();
    assert_eq!(rows.len(), 129);
    for (line, source, expected, corrected) in rows {
        assert_eq!(findings(&source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceInsideArrayPercentLiteral, &source).is_empty());
        } else {
            assert_correction(&SpaceInsideArrayPercentLiteral, &source, &corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceInsideArrayPercentLiteral;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn separator_runs_collapse_to_one_space() {
    for (source, expected, after) in [
        ("x = %w(a  b)\n", &[(1, 9)][..], "x = %w(a b)\n"),
        ("x = %w(a\t  b)\n", &[], ""),
        ("x = %w(a  \tb)\n", &[], ""),
        (
            "x = %w(a  b\n  c  d)\n",
            &[(1, 9), (2, 4)],
            "x = %w(a b\n  c d)\n",
        ),
        (
            "x = %i(a    b    c)\n",
            &[(1, 9), (1, 14)],
            "x = %i(a b c)\n",
        ),
        ("x = %W(#{1}  b)\n", &[(1, 12)], "x = %W(#{1} b)\n"),
        ("x = %I(a#{1}  b)\n", &[(1, 13)], "x = %I(a#{1} b)\n"),
        ("x = %w(é  b)\n", &[(1, 9)], "x = %w(é b)\n"),
        ("x = %w( a  b )\n", &[(1, 10)], "x = %w( a b )\n"),
        ("x = %w(a\\ \\   b)\n", &[(1, 13)], "x = %w(a\\ \\  b)\n"),
        (
            "p %w[a  b], %i[c  d]\n",
            &[(1, 7), (1, 17)],
            "p %w[a b], %i[c d]\n",
        ),
        ("\u{feff}x = %w(a  b)\n", &[(1, 9)], "\u{feff}x = %w(a b)\n"),
        ("x = %w(a  b)\r\n", &[(1, 9)], "x = %w(a b)\r\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        if !expected.is_empty() {
            assert_correction(&SpaceInsideArrayPercentLiteral, source, after);
        }
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = %w(a b)\n",
        "x = %w(  a b  )\n",
        "x = %w(a\n     b)\n",
        "x = %q(a  b)\n",
        "x = %Q(a  b)\n",
        "x = %(a  b)\n",
        "x = %s(a  b)\n",
        "x = %r(a  b)\n",
        "x = %x(a  b)\n",
        "x = [\"a\",  \"b\"]\n",
        "x = \"%w(a  b)\"\n",
        "# %w(a  b)\n",
        // Upstream's regexp reads `\\` as a character that cannot precede
        // the run, so an escaped backslash before it is accepted.
        "x = %w(a\\\\  b)\n",
        "x = %w(a\\\\\\   b)\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn runs_inside_an_interpolation_keep_the_finding_without_an_edit() {
    // Upstream matches the literal's source text, interpolated code
    // included; collapsing a run there could change a string's value.
    assert_eq!(findings("x = %W(#{\"a  b\"} c)\n"), [(1, 12)]);
    held("x = %W(#{\"a  b\"} c)\n");
    assert_eq!(findings("x = %I(#{a  + 1})\n"), [(1, 11)]);
    held("x = %I(#{a  + 1})\n");
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
        held(&format!("x = %w(a  b)\n{observer}\n"));
    }
    assert_correction(
        &SpaceInsideArrayPercentLiteral,
        "x = %w(a  b)\np File.read('example.rb')\n",
        "x = %w(a b)\np File.read('example.rb')\n",
    );
}
