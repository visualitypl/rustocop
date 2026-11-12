//! Owned diagnostics from the one main Prism parse, including rejected input.

use std::ops::Range;

use ruby_prism::ParseResult;

use super::SourceFile;

/// An actual Prism diagnostic. Ranges always address the original source bytes,
/// not a transcoded buffer or an expanded display highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    /// Prism's printable message, without cop prefixes or configuration advice.
    pub message: String,
    /// Original byte offsets, including zero-length EOF locations. These are
    /// diagnostic locations, not correction ranges or expanded highlights.
    pub range: Range<usize>,
}

/// A fatal lifecycle finding. A buffer exception has no source location;
/// parser errors carry a display range, never a correction or recovered AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FatalDiagnostic {
    /// Printable diagnostic, without cop prefixes.
    pub message: String,
    /// Original-byte display highlight; `None` for a global buffer exception.
    pub range: Option<Range<usize>>,
}

/// Backward-compatible name for warning consumers.
pub type ParseWarning = ParseDiagnostic;

/// Owned main-parse facts. Severity is explicit in the two separate collections:
/// errors reject the file; warnings alone do not prohibit cop dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostics {
    /// Source::Buffer encoding failure, distinct from actual Prism errors.
    /// This rejects input even when Prism accepts invalid bytes in comments.
    pub buffer_error: Option<String>,
    /// All errors in Prism's emission order (preserves fatal delivery order).
    pub errors: Vec<ParseDiagnostic>,
    /// All warnings in stable source order, as existing warning consumers expect.
    pub warnings: Vec<ParseWarning>,
}

impl ParseDiagnostics {
    fn from_parse(result: &ParseResult<'_>) -> Self {
        let collect = |diagnostic: ruby_prism::Diagnostic<'_>| {
            let location = diagnostic.location();
            ParseDiagnostic {
                message: diagnostic.message().into_owned(),
                range: location.start_offset()..location.end_offset(),
            }
        };
        let errors = result.errors().map(collect).collect();
        let mut warnings: Vec<_> = result.warnings().map(collect).collect();
        warnings.sort_by_key(|warning| (warning.range.start, warning.range.end));
        Self {
            buffer_error: super::encoding::source_buffer_error(result),
            errors,
            warnings,
        }
    }
}

impl SourceFile {
    /// Record diagnostics and encoding from the parse of this file's bytes,
    /// before testing validity or dispatching any cops. No candidate parse,
    /// recovered-tree walk, or synthetic diagnostic is involved.
    pub fn record_diagnostics(&self, result: &ParseResult<'_>) {
        self.record_encoding(result);
        let _ = self.diagnostics.set(ParseDiagnostics::from_parse(result));
    }

    /// Actual Prism errors/warnings, including on invalid input. Pipeline users
    /// share the recorded main parse. Standalone SourceFiles parse lazily once;
    /// encoding is recorded at the same time for `line_col`/`line_cols` mapping.
    pub fn diagnostics(&self) -> &ParseDiagnostics {
        self.diagnostics.get_or_init(|| {
            let result = crate::parser::parse_as(&self.bytes, self.ruby_version());
            self.record_encoding(&result);
            ParseDiagnostics::from_parse(&result)
        })
    }

    /// Record warnings without reparsing. Also retains errors and encoding.
    /// Kept for existing callers; new main-parse callers use record_diagnostics.
    pub fn record_warnings(&self, result: &ParseResult<'_>) {
        self.record_diagnostics(result);
    }

    /// Every warning the parser reported, in stable source order.
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.diagnostics().warnings
    }
}

#[cfg(test)]
mod tests {
    // Syntax fixtures ported from RuboCop's lint/syntax_spec.rb (MIT License,
    // Copyright (c) 2012-26 Bozhidar Batsov). No fixture is executed as Ruby.
    use crate::source::SourceFile;

    #[test]
    fn encoded_ranges_and_positions_match_main_parse_and_lazy_view() {
        for (prefix, column) in [
            (b"# coding: ascii-8bit\n\"\xe3\x80\x80\"; ".as_slice(), 8),
            (b"# coding: ISO-8859-1\n\"\xe3\x80\x80\"; ", 8),
            (b"# coding: Windows-1252\n\"\xc3\xa9\"; ", 7),
            (b"# coding: Shift_JIS\n\"\x82\xa0\"; ", 6),
            (b"# coding: EUC-JP\n\"\xa4\xa2\"; ", 6),
            (b"# coding: UTF-8\n\"\xe3\x80\x80\"; ", 6),
            (b"\xef\xbb\xbf# coding: ascii-8bit\n\"\xe3\x80\x80\"; ", 6),
        ] {
            for newline in [b"\n".as_slice(), b"\r\n"] {
                let prefix = prefix
                    .split(|&b| b == b'\n')
                    .collect::<Vec<_>>()
                    .join(newline);
                for tail in [b"foo *args; p /pattern/".as_slice(), b"foo *args; foo(1, ]"] {
                    let mut bytes = prefix.clone();
                    bytes.extend_from_slice(tail);
                    bytes.extend_from_slice(newline);
                    let recorded = SourceFile::new("file.rb".into(), bytes.clone());
                    let fallback = SourceFile::new("stdin.rb".into(), bytes);
                    let parsed = crate::parser::parse(&recorded.bytes);
                    recorded.record_diagnostics(&parsed);
                    assert!(recorded.encoding.get().is_some());
                    assert_eq!(recorded.diagnostics(), fallback.diagnostics());
                    assert!(fallback.encoding.get().is_some());
                    let diagnostics = recorded.diagnostics();
                    assert_eq!(diagnostics.errors.len(), parsed.errors().count());
                    assert_eq!(diagnostics.warnings.len(), parsed.warnings().count());
                    let star = diagnostics
                        .warnings
                        .iter()
                        .find(|d| d.message.contains("ambiguous `*`"))
                        .unwrap();
                    assert_eq!(&recorded.bytes[star.range.clone()], b"*");
                    assert_eq!(recorded.line_col(star.range.start), (2, column + 4));
                    if tail.ends_with(b"]") {
                        assert!(!diagnostics.errors.is_empty());
                        for error in &diagnostics.errors {
                            assert_eq!(&recorded.bytes[error.range.clone()], b"]");
                            assert_eq!(recorded.line_col(error.range.start), (2, column + 18));
                        }
                    }
                    for diagnostic in diagnostics.errors.iter().chain(&diagnostics.warnings) {
                        assert_eq!(
                            recorded.line_col(diagnostic.range.start),
                            fallback.line_col(diagnostic.range.start)
                        );
                        assert_eq!(
                            recorded.line_col(diagnostic.range.end),
                            fallback.line_col(diagnostic.range.end)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn eof_ranges_are_not_fabricated_display_highlights() {
        for source in ["(", "foo(", "\u{feff}("] {
            let file = SourceFile::new("eof.rb".into(), source.as_bytes().to_vec());
            let diagnostics = file.diagnostics();
            assert_eq!(diagnostics.errors.len(), 1);
            assert_eq!(diagnostics.errors[0].range, source.len()..source.len());
        }
        // The pinned EOF fixture does NOT yield a zero-length range in raw
        // Prism: preserve both the EOF sentinel and the opening-keyword hint,
        // in emission order, rather than projecting the translated buffer.
        for source in [
            "if true\n  puts 'x'\n",
            "if true\r\n  puts 'é'\r\n",
            "\u{feff}if true\n  puts 'x'\n",
        ] {
            let file = SourceFile::new("eof.rb".into(), source.as_bytes().to_vec());
            let errors = &file.diagnostics().errors;
            assert_eq!(errors.len(), 2);
            assert_eq!(
                errors[0].range,
                source.len() - 1..source.len(),
                "{source:?}: {errors:?}"
            );
            assert_eq!(
                file.line_col(errors[0].range.start),
                (2, if source.contains('\r') { 12 } else { 11 })
            );
            let start = usize::from(source.starts_with('\u{feff}')) * 3;
            assert_eq!(errors[1].range, start..start + 2);
        }
    }

    #[test]
    fn heredocs_data_and_unrelated_warnings_remain_actual_parser_facts() {
        for (source, warning_count) in [
            (
                "puts <<~TEXT\nfoo *args; p /pattern/\nTEXT\n__END__\n(\n",
                0,
            ),
            ("puts <<~TEXT\n#{foo *args}\nTEXT\n", 1),
            ("def f; value = 1; end\n", 1),
        ] {
            let file = SourceFile::new("data.rb".into(), source.as_bytes().to_vec());
            assert!(file.diagnostics().errors.is_empty(), "{source}");
            assert_eq!(file.warnings().len(), warning_count, "{source}");
            assert!(
                file.warnings()
                    .windows(2)
                    .all(|pair| pair[0].range.start <= pair[1].range.start)
            );
        }
        // This pinned Syntax fixture is a Source::Buffer encoding exception
        // upstream, NOT a diagnostic emitted by the main Prism parse. Preserve
        // that distinction: never fabricate a Prism error or reparse it.
        let file = SourceFile::new("comment.rb".into(), b"# \xf9\n".to_vec());
        assert_eq!(
            file.diagnostics().buffer_error.as_deref(),
            Some("Invalid byte sequence in utf-8.")
        );
        assert!(file.diagnostics().errors.is_empty());
        assert!(file.warnings().is_empty());
    }

    #[test]
    fn syntax_first_consumer_retains_errors_separately_from_warnings() {
        for (source, errors, warnings) in [
            ("(", 1, 0),
            (
                "# warning: ambiguous `*` has been interpreted as an argument prefix\nfoo *some_array\n(\n",
                1,
                1,
            ),
            ("foo *some_array\n", 0, 1),
            ("foo(*some_array)\n", 0, 0),
            ("", 0, 0),
        ] {
            let file = SourceFile::new("virtual.rb".into(), source.as_bytes().to_vec());
            let parsed = crate::parser::parse(&file.bytes);
            file.record_diagnostics(&parsed);
            let diagnostics = file.diagnostics();
            assert_eq!(diagnostics.errors.len(), errors, "{source:?}");
            assert_eq!(diagnostics.warnings.len(), warnings, "{source:?}");
            for (actual, expected) in diagnostics.errors.iter().zip(parsed.errors()) {
                assert_eq!(actual.message, expected.message());
                assert_eq!(
                    actual.range,
                    expected.location().start_offset()..expected.location().end_offset()
                );
            }
            assert_eq!(file.warnings(), diagnostics.warnings);
            // The snapshot owns its text and ranges after parser teardown.
            drop(parsed);
            assert!(std::ptr::eq(diagnostics, file.diagnostics()));
        }
    }
}
