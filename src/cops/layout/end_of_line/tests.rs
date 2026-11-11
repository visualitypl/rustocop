//! Examples ported from RuboCop's `spec/rubocop/cop/layout/end_of_line_spec.rb`
//! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
//! pinned example, including the `crlf` and `lf` contexts and the four
//! `iso-8859-15` shared examples (which assert that a nested
//! `expect_no_offenses` fails, and which the oracle skips for their custom
//! style), is replayed at the fixed `native` style, which is `lf` on this non-Windows
//! host: findings and messages equal the fixed replay recorded in
//! the pinned RuboCop reference run, and the bytes never change.
use std::path::Path;

use super::*;
use crate::{config::ResolvedConfig, offense::Severity};

const COP: &str = "Layout/EndOfLine";
const DETECTED: &str = "Carriage return character detected.";

/// Lint raw bytes (any source encoding) with the cop alone, collecting
/// corrections, and return its findings.
fn findings(source: &[u8]) -> Vec<(u32, u32)> {
    let report = crate::pipeline::lint_one(
        Path::new("example.rb"),
        Path::new("example.rb"),
        Some(source),
        &[&EndOfLine],
        &ResolvedConfig::default(),
    );
    assert!(report.errors.is_empty(), "{source:?} {:?}", report.errors);
    let offenses = &report.result.offenses;
    assert!(
        crate::pipeline::lint_str_with_corrections(&[&EndOfLine], "x = 1\r\n")
            .iter()
            .all(|o| o.correction.is_none())
    );
    offenses
        .iter()
        .map(|o| {
            assert_eq!(o.cop, COP);
            assert_eq!(o.message, DETECTED);
            assert_eq!(o.severity, Severity::Convention);
            assert!(o.correction.is_none());
            (o.line, o.col)
        })
        .collect()
}

const EPD_CRLF: &str = "class Epd::ReportsController < EpdAreaController\r\n  'terecht bij uw ROM-co\u{f6}rdinator.'\r\nend\r\n";
const EPD_LF: &str = "class Epd::ReportsController < EpdAreaController\n  'terecht bij uw ROM-co\u{f6}rdinator.'\nend\n";

/// `(spec line, source, fixed-default finding)`.
type Pinned = (u32, &'static [u8], Option<(u32, u32)>);

const PINNED: &[Pinned] = &[
    // EnforcedStyle: native
    (47, b"x=0\n\ny=1\r\n", Some((3, 1))),
    // EnforcedStyle: crlf, replayed at the fixed native (lf) style
    (63, b"x=0\n\ny=1\r\n", Some((3, 1))),
    (72, b"x=0", None),
    (76, b"x=0\r\n__END__\nx=0\n", Some((1, 1))),
    // all configurations: accepts an empty file
    (8, b"", None),
    // and there are many lines ending with LF
    (85, b"x=0\n\ny=1\n", None),
    // iso-8859-15 with eol ""
    (27, ISO_LF, None),
    // and the default external encoding is US_ASCII
    (102, EPD_CRLF.as_bytes(), Some((1, 1))),
    // iso-8859-15 with eol ""
    (27, ISO_LF, None),
    // EnforcedStyle: lf
    (119, b"x=0\n\ny=1\r\n", Some((3, 1))),
    (128, b"x=0\r\n", Some((1, 1))),
    (135, b"x=0\n__END__\nx=0\r\n", None),
    // all configurations: accepts an empty file
    (8, b"", None),
    // and there are many lines ending with CR+LF
    (144, b"x=0\r\n\r\ny=1\n", Some((1, 1))),
    // iso-8859-15 with eol "\r"
    (27, ISO_CRLF, Some((1, 1))),
    // and the default external encoding is US_ASCII
    (161, EPD_LF.as_bytes(), None),
    // iso-8859-15 with eol "\r"
    (27, ISO_CRLF, Some((1, 1))),
];

const ISO_LF: &[u8] = b"# coding: ISO-8859-15\n# Euro symbol: \xa4\n";
const ISO_CRLF: &[u8] = b"# coding: ISO-8859-15\r\n# Euro symbol: \xa4\r\n";

#[test]
fn pinned_examples() {
    for &(line, source, finding) in PINNED {
        assert_eq!(
            findings(source),
            finding.into_iter().collect::<Vec<_>>(),
            "spec line {line}"
        );
    }
}

#[test]
fn only_the_first_carriage_return_line_is_reported() {
    for (source, finding) in [
        (b"a = 1\r\nb = 2\r\nc = 3\r\n".as_slice(), Some((1, 1))),
        (b"a = 1\nb = 2\r\nc = 3\r\n", Some((2, 1))),
        // A BOM does not move the column.
        (b"\xef\xbb\xbfa = 1\r\n", Some((1, 1))),
        // String content counts: the raw line ends in CR LF.
        (b"x = \"a\r\nb\"\n", Some((1, 1))),
        (b"x = \"a\nb\"\n", None),
    ] {
        assert_eq!(
            findings(source),
            finding.into_iter().collect::<Vec<_>>(),
            "{source:?}"
        );
    }
}

/// Upstream stops at the line of `processed_source.tokens.last` (Prism's
/// token stream translated for the parser gem).
#[test]
fn lines_after_the_last_token_are_not_checked() {
    for (source, finding) in [
        // Trailing blank lines.
        (b"x = 1\n\r\n".as_slice(), None),
        (b"x = 1\n\n\r\n\r\n", None),
        // A run of comments right after the last code line ends with that
        // line's deferred newline token.
        (b"x = 1\n# c\r\n", None),
        (b"x = 1\n# a\n  # b\r\n", None),
        (b"x = 1 # a\n# b\r\n", None),
        (b"x = 1 # a\r\n", Some((1, 1))),
        // A blank line ends the run: the last comment is the last token.
        (b"x = 1\n\n# c\r\n", Some((3, 1))),
        (b"x = 1\n# a\n\n# b\r\n", Some((4, 1))),
        // Comments only.
        (b"# a\n# b\r\n", Some((2, 1))),
        // A heredoc body is emitted at its opener, before the line's newline.
        (b"x = <<~X\n  a\r\nX\n", None),
        (b"x = <<~X\n  a\r\nX\ny = 1\n", Some((2, 1))),
        (b"x = <<~X\n  a\nX\n# c\r\n", None),
        (b"x = <<~X\n  a\nX\n\n# c\r\n", Some((5, 1))),
        // A block comment is one token at its first line.
        (b"x = 1\n=begin\nfoo\r\n=end\n", None),
        (b"x = 1\n=begin\r\nfoo\n=end\n", Some((2, 1))),
        // The data section follows the last token.
        (b"x = 1\n__END__\r\nfoo\r\n", None),
        // Without any token every raw line is checked, data included.
        (b"\r\n\r\n", Some((1, 1))),
        (b"\n\n  \r\n", Some((3, 1))),
        (b"__END__\r\n", Some((1, 1))),
        (b"\n__END__\nx\r\n", Some((3, 1))),
    ] {
        assert_eq!(
            findings(source),
            finding.into_iter().collect::<Vec<_>>(),
            "{:?}",
            String::from_utf8_lossy(source)
        );
    }
}

#[test]
fn metadata_is_fixed_and_detection_only() {
    let cop = EndOfLine;
    assert!(cop.default_enabled() && !cop.safe_fixes());
    assert!(cop.department_selection_enables());
    assert_eq!(cop.severity(), Severity::Convention);
    assert_eq!(cop.ruby_versions(), Default::default());
    assert!(cop.path_policy().is_none() && cop.max_setting().is_none());
}
