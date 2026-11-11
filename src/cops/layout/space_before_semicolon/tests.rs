//! Examples ported from RuboCop's
//! `spec/rubocop/cop/layout/space_before_semicolon_spec.rb` at 97f30c4 (MIT
//! License, Copyright (c) 2012-26 Bozhidar Batsov). All 9 pinned examples
//! (8 distinct sources) run with `Layout/SpaceInsideBlockBraces` at its fixed
//! `space`; the `no_space` context is replayed at that default. Findings and
//! single-pass corrected bytes equal the fixed replay recorded in
//! the pinned RuboCop reference run (source and corrected SHA-256 of
//! every row were checked against it when the rows were ported).
use super::*;
use crate::cops::testing::{assert_correction, correction_offenses};

const COP: &str = "Layout/SpaceBeforeSemicolon";
const MESSAGE: &str = "Space found before semicolon.";

/// `(line, col)` of each finding from the full default pipeline.
fn findings(source: &str) -> Vec<(u32, u32)> {
    crate::pipeline::lint_str(source)
        .into_iter()
        .filter(|o| o.cop == COP)
        .map(|o| {
            assert_eq!(o.severity, crate::offense::Severity::Convention);
            assert_eq!(o.message, MESSAGE);
            assert_eq!(o.end_line, o.line);
            (o.line, o.col)
        })
        .collect()
}

/// `(spec lines, source, findings, corrected source)`.
type Pinned = (
    &'static [u32],
    &'static str,
    &'static [(u32, u32)],
    &'static str,
);

const PINNED: &[Pinned] = &[
    (&[8], "x = 1 ; y = 2\n", &[(1, 6)], "x = 1; y = 2\n"),
    (&[19], "x = 1; y = 2", &[], ""),
    (&[23], "x = 1  ; y = 2\n", &[(1, 6)], "x = 1; y = 2\n"),
    (&[46], "test { ; }", &[], ""),
    (&[50], "-> { ; }", &[], ""),
    (&[36, 36], "test {; }", &[], ""),
    (&[60], "test { ; }\n", &[], ""),
    (
        &[74],
        "<<~STR ; x = 1\n  text\nSTR\n",
        &[(1, 7)],
        "<<~STR; x = 1\n  text\nSTR\n",
    ),
];

#[test]
fn pinned_examples_replay_the_fixed_results() {
    let mut examples = 0;
    for &(spec, source, expected, after) in PINNED {
        examples += spec.len();
        assert_eq!(findings(source), expected, "spec lines {spec:?}");
        if expected.is_empty() {
            assert!(
                correction_offenses(&SpaceBeforeSemicolon, source).is_empty(),
                "{spec:?}"
            );
        } else {
            assert_correction(&SpaceBeforeSemicolon, source, after);
        }
    }
    assert_eq!(examples, 9);
}

#[test]
fn only_real_semicolon_tokens_after_a_gap() {
    for source in [
        "x = 1; y = 2\n",
        "x = \"a ;\"\n",
        "x = 'a ;'\n",
        "x = 1 # a ;\n",
        "x = %w[a ;]\n",
        "x = /a ;/\n",
        "p $;\n",
        "test { ; }\n",
        "test {; }\n",
        "-> { ; }\n",
        "x = <<~EOS\n  a ;\nEOS\n",
        "x = 1\n;\n",
        "x = 1 \\\n; y = 2\n",
        "x = 1\n__END__\na ;\n",
    ] {
        assert!(findings(source).is_empty(), "{source:?}");
    }
    for (source, expected) in [
        ("p \"#{ ;1}\"\n", vec![(1, 6)]),
        ("def f ; end\n", vec![(1, 6)]),
        ("foo {|x| ; }\n", vec![(1, 9)]),
        ("x = 1\t;\n", vec![(1, 6)]),
        ("p $; ;\n", vec![(1, 5)]),
        ("x = {} ; y = 2\n", vec![(1, 7)]),
        ("\u{feff}é = 1 ;\r\n", vec![(1, 6)]),
        (
            "x = <<~A ; y = <<~B ;\n  a\nA\n  b\nB\n",
            vec![(1, 9), (1, 20)],
        ),
    ] {
        assert_eq!(findings(source), expected, "{source:?}");
    }
}

#[test]
fn corrections_keep_the_ast_and_hold_for_listed_observers() {
    for (before, after) in [
        ("x = 1  ; y = 2\n", "x = 1; y = 2\n"),
        ("x = 1\t;\r\n", "x = 1;\r\n"),
        ("p \"#{ ;1}\"\n", "p \"#{;1}\"\n"),
        ("x = %q x  ;\n", "x = %q x ;\n"),
        ("x = ?a ;\n", "x = ?a;\n"),
    ] {
        assert_correction(&SpaceBeforeSemicolon, before, after);
        let (a, b) = (
            crate::parser::parse(before.as_bytes()),
            crate::parser::parse(after.as_bytes()),
        );
        assert_eq!(
            crate::cops::ast::fingerprint_ignoring_grouping(&a.node(), before.as_bytes()),
            crate::cops::ast::fingerprint_ignoring_grouping(&b.node(), after.as_bytes()),
            "{before:?}"
        );
    }
    for observer in [
        "__FILE__",
        "__dir__",
        "$0",
        "$PROGRAM_NAME",
        "caller",
        "caller_locations",
        "method(:x).source_location",
        "SCRIPT_LINES__",
        "DATA.read\n__END__\nx",
    ] {
        let source = format!("x = 1 ; y = 2\np {observer}\n");
        let found = correction_offenses(&SpaceBeforeSemicolon, &source);
        assert_eq!(found.len(), 1, "{source:?}");
        assert!(found[0].correction.is_none(), "{source:?}");
    }
}
