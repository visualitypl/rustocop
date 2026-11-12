//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_inside_percent_literal_delimiters_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The
//! spec's loops over the `i`, `I`, `w`, `W` and `x` types and the `{}`,
//! `()`, `[]` and `!!` delimiters are generated here the same way: all 262
//! pinned examples. Findings and messages equal the fixed replay recorded
//! in the pinned RuboCop reference run, and so do the single-pass
//! corrected bytes of the array types (each generated source and corrected
//! SHA-256 was checked against it when the rows were ported). The `%x`
//! rows keep their findings without an edit: trimming a command changes the
//! string passed to `` ` `` (a known deviation).
use super::*;
use crate::{
    cops::testing::{assert_correction, correction_offenses},
    pipeline::lint_str_with,
};

const COP: &str = "Layout/SpaceInsidePercentLiteralDelimiters";

fn findings(source: &str) -> Vec<(u32, u32)> {
    assert_eq!(
        crate::parser::parse(source.as_bytes()).errors().count(),
        0,
        "{source:?}"
    );
    let mut found: Vec<_> = lint_str_with(&[&SpaceInsidePercentLiteralDelimiters], source)
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
    let found = correction_offenses(&SpaceInsidePercentLiteralDelimiters, source);
    assert!(!found.is_empty(), "{source:?}");
    assert!(found.iter().all(|o| o.correction.is_none()), "{source:?}");
}

/// `(spec line, source, findings, upstream's single-pass correction)`; an
/// empty correction means the source has no findings.
type Row = (u32, String, Vec<(u32, u32)>, String);

/// The spec's examples in its order: the type and delimiter loops, then
/// the other literals.
fn pinned() -> Vec<Row> {
    let mut rows = Vec::new();
    for ty in ["i", "I", "w", "W", "x"] {
        for (l, r) in [("{", "}"), ("(", ")"), ("[", "]"), ("!", "!")] {
            let code = |content: &str| format!("%{ty}{l}{content}{r}");
            let fixed = format!("{}\n", code("1 2"));
            rows.extend([
                (
                    12,
                    format!("{}\n", code(" 1 2  ")),
                    vec![(1, 4), (1, 8)],
                    fixed.clone(),
                ),
                (
                    22,
                    format!("{}\n", code(" 1 2")),
                    vec![(1, 4)],
                    fixed.clone(),
                ),
                (31, format!("{}\n", code("1 2 ")), vec![(1, 7)], fixed),
                (
                    40,
                    format!("{}\n", code(" \\ a b c\\  ")),
                    vec![(1, 4), (1, 14)],
                    format!("{}\n", code("\\ a b c\\ ")),
                ),
                (50, code("a b c"), vec![], String::new()),
                (54, code("\\ a b c\\ "), vec![], String::new()),
                (
                    58,
                    format!("%{ty}(\n  a\n  b\n  c\n)\n"),
                    vec![],
                    String::new(),
                ),
                (
                    68,
                    format!("def foo\n  %{ty}(\n    a\n    b\n    c\n  )\nend\n"),
                    vec![],
                    String::new(),
                ),
                (80, format!("%{ty}(a b\n   c)\n"), vec![], String::new()),
                (87, code("a  b  c"), vec![], String::new()),
                (
                    92,
                    format!("{}\n", code(" ")),
                    vec![(1, 4)],
                    format!("{}\n", code("")),
                ),
                (
                    103,
                    format!("{}\n", code("  ")),
                    vec![(1, 4)],
                    format!("{}\n", code("")),
                ),
                (114, code("\n"), vec![(1, 4)], code("")),
            ]);
        }
    }
    rows.push((
        124,
        "%q( a  b c )\n%r( a  b c )\n%s( a  b c )\n".into(),
        vec![],
        String::new(),
    ));
    rows.push((132, "` curl `".into(), vec![], String::new()));
    rows
}

#[test]
fn pinned_examples_match_the_fixed_replay() {
    let rows = pinned();
    assert_eq!(rows.len(), 262);
    for (line, source, expected, corrected) in rows {
        assert_eq!(findings(&source), expected, "spec line {line} {source:?}");
        if expected.is_empty() {
            assert!(corrected.is_empty());
            assert!(correction_offenses(&SpaceInsidePercentLiteralDelimiters, &source).is_empty());
        } else if source.starts_with("%x") {
            held(&source);
        } else {
            assert_correction(&SpaceInsidePercentLiteralDelimiters, &source, &corrected);
        }
    }
}

#[test]
fn metadata() {
    let cop = SpaceInsidePercentLiteralDelimiters;
    assert!(cop.default_enabled());
    assert!(cop.department_selection_enables());
    assert!(cop.safe_fixes());
    assert!(cop.autocorrect_incompatible_with().is_empty());
    assert!(cop.max_setting().is_none() && cop.path_policy().is_none());
}

#[test]
fn delimiter_spaces_and_blank_bodies_are_removed() {
    for (source, expected, after) in [
        ("x = %w( a b )\n", &[(1, 8), (1, 12)][..], "x = %w(a b)\n"),
        ("x = %i[   a]\n", &[(1, 8)], "x = %i[a]\n"),
        ("x = %W(#{1} )\n", &[(1, 12)], "x = %W(#{1})\n"),
        ("x = %I( a#{1})\n", &[(1, 8)], "x = %I(a#{1})\n"),
        ("x = %w(a\\  )\n", &[(1, 11)], "x = %w(a\\ )\n"),
        ("x = %w(a\\\\  )\n", &[(1, 12)], "x = %w(a\\\\ )\n"),
        // Blank bodies: one finding for the whole body, any whitespace.
        ("x = %w(\t)\n", &[(1, 8)], "x = %w()\n"),
        // The leading run starts where the blank body does: one finding
        // there, as upstream's runner reports; the body's edit covers the
        // trailing run.
        ("x = %w( \t )\n", &[(1, 8), (1, 10)], "x = %w()\n"),
        ("x = %w(\n\n)\n", &[(1, 8)], "x = %w()\n"),
        ("x = %i(  \n  )\n", &[(1, 8)], "x = %i()\n"),
        ("x = %w(\x0b\x0c)\n", &[(1, 8)], "x = %w()\n"),
        (
            "p %w( a ), %i( b )\n",
            &[(1, 6), (1, 8), (1, 15), (1, 17)],
            "p %w(a), %i(b)\n",
        ),
        ("x = %w( é )\n", &[(1, 8), (1, 10)], "x = %w(é)\n"),
        (
            "\u{feff}x = %w( a )\n",
            &[(1, 8), (1, 10)],
            "\u{feff}x = %w(a)\n",
        ),
        ("x = %w( a )\r\n", &[(1, 8), (1, 10)], "x = %w(a)\r\n"),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        assert_correction(&SpaceInsidePercentLiteralDelimiters, source, after);
    }
}

#[test]
fn lookalikes_are_accepted() {
    for source in [
        "x = %w(a b)\n",
        "x = %w()\n",
        "x = %w(\\ a\\ )\n",
        "x = %w(\ta\t)\n",
        "x = %w(\n  a\n)\n",
        "x = %w( a\n  b )\n",
        "x = %q( a )\n",
        "x = %Q( a )\n",
        "x = % a \n",
        "x = %( a )\n",
        "x = %s( a )\n",
        "x = %r( a )\n",
        "x = [ 'a' ]\n",
        "x = ` ls `\n",
        "x = \"%w( a )\"\n",
        "# %w( a )\n",
        "x = <<~`A`\n   ls \nA\n",
    ] {
        assert_eq!(findings(source), [], "{source:?}");
    }
}

#[test]
fn command_literals_keep_their_findings_without_an_edit() {
    // The trimmed blanks are part of the command string, which a
    // redefined `` ` `` receives as is.
    for (source, expected) in [
        ("x = %x( ls )\n", &[(1, 8), (1, 11)][..]),
        ("x = %x( ls #{1})\n", &[(1, 8)]),
        ("x = %x( )\n", &[(1, 8)]),
        ("x = %x(\n)\n", &[(1, 8)]),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
        held(source);
    }
}

#[test]
fn a_blank_body_with_an_element_keeps_its_finding_without_an_edit() {
    // `String#strip` drops NUL, so upstream reports the body, but `%w`
    // keeps NUL as an element: removing it would empty the array.
    for (source, expected) in [
        ("p %w(\0)\n", &[(1, 6)][..]),
        ("p %w( \0 )\n", &[(1, 6), (1, 8)]),
        ("p %i(\0\0)\n", &[(1, 6)]),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
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
        "p method(:p).source_location",
        "SCRIPT_LINES__ = {}",
        "p DATA.read\n__END__\nx",
    ] {
        held(&format!("x = %w( a )\n{observer}\n"));
    }
    assert_correction(
        &SpaceInsidePercentLiteralDelimiters,
        "x = %w( a )\np File.read('example.rb')\n",
        "x = %w(a)\np File.read('example.rb')\n",
    );
}
