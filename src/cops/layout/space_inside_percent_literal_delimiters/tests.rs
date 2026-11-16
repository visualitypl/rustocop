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
use crate::{cops::testing::assert_correction, pipeline::lint_str_with};

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
