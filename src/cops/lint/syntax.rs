//! `Lint/Syntax`: fatal main-parse diagnostics, never recovered-AST checks.

use crate::{
    cops::Cop,
    source::{FatalDiagnostic, SourceFile},
};

/// Fatal Syntax findings are file errors (exit2), not ordinary lint offenses.
pub struct Syntax;

impl Cop for Syntax {
    fn name(&self) -> &'static str {
        "Lint/Syntax"
    }

    fn wants_fatal_diagnostics(&self) -> bool {
        true
    }

    fn check_fatal(&self, file: &SourceFile, out: &mut Vec<FatalDiagnostic>) {
        check_fatal_with_width(file, out, |bytes| file.encoded_character_width(bytes));
    }
}

fn check_fatal_with_width(
    file: &SourceFile,
    out: &mut Vec<FatalDiagnostic>,
    mut character_width: impl FnMut(&[u8]) -> usize,
) {
    let diagnostics = file.diagnostics();
    if let Some(message) = &diagnostics.buffer_error {
        out.push(FatalDiagnostic {
            message: message.clone(),
            range: None,
        });
        return;
    }
    // All empty EOF ranges share one encoded-character boundary. Cache it
    // lazily so repeated errors cannot multiply a long final-line scan.
    let mut eof_start = None;
    out.extend(diagnostics.errors.iter().map(|diagnostic| {
        let mut range = diagnostic.range.clone();
        if range.is_empty() {
            if range.end < file.bytes.len() {
                // Expand to a whole source-encoded character, not one byte.
                range.end += character_width(&file.bytes[range.end..]);
            } else if range.start > 0 {
                // EOF: find the preceding character without splitting UTF-8,
                // a legacy multibyte character or the UTF-8 BOM.
                range.start = *eof_start.get_or_insert_with(|| {
                    let line_start = file.bytes[..range.start]
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .map_or(0, |p| p + 1);
                    let mut cursor = line_start;
                    let mut previous = range.start - 1;
                    while cursor < range.start {
                        previous = cursor;
                        cursor += character_width(&file.bytes[cursor..range.start]);
                    }
                    previous
                });
            }
        }
        FatalDiagnostic {
            message: diagnostic.message.clone(),
            range: Some(range),
        }
    }));
}

#[cfg(test)]
mod tests {
    //! Fixtures ported from RuboCop's lint/syntax_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Sources are never executed.
    use super::*;
    use crate::{cops::Cop, pipeline::lint_one, source::SourceFile};
    use std::path::Path;

    fn diagnostics(source: &[u8]) -> Vec<FatalDiagnostic> {
        let file = SourceFile::new("test.rb".into(), source.to_vec());
        let parsed = crate::parser::parse(&file.bytes);
        file.record_diagnostics(&parsed);
        let mut out = Vec::new();
        Syntax.check_fatal(&file, &mut out);
        out
    }

    #[test]
    fn real_error_expands_eof_without_changing_main_parse_range() {
        for (source, range, position) in [
            ("(", 0..1, (1, 1)),
            ("foo(", 3..4, (1, 4)),
            ("é; foo(", 7..8, (1, 7)),
            ("foo( # é", 7..9, (1, 8)),
            ("\u{feff}(", 3..4, (1, 1)),
        ] {
            let file = SourceFile::new("test.rb".into(), source.as_bytes().to_vec());
            let raw = file.diagnostics().errors[0].range.clone();
            assert_eq!(raw, source.len()..source.len());
            let found = diagnostics(source.as_bytes());
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].range, Some(range.clone()));
            assert_eq!(file.line_col(range.start), position);
            assert_eq!(
                found[0].message,
                if source.contains("foo(") {
                    "unexpected end-of-input; expected a `)` to close the arguments"
                } else {
                    "expected a matching `)`"
                }
            );
            assert_eq!(file.diagnostics().errors[0].range, raw);
        }
    }

    #[test]
    fn repeated_eof_diagnostics_scan_the_final_line_only_once() {
        for (header, character) in [
            (b"".as_slice(), "é".as_bytes()),
            (b"\xef\xbb\xbf".as_slice(), "中".as_bytes()),
            (
                b"# coding: Shift_JIS\r\n".as_slice(),
                b"\x82\xa0".as_slice(),
            ),
            (b"# coding: EUC-JP\n".as_slice(), b"\x8f\xab\xb1".as_slice()),
            (
                b"# coding: GB18030\n".as_slice(),
                b"\x90\x30\xeb\x34".as_slice(),
            ),
        ] {
            let calls = b"foo(".repeat(100);
            let source = [header, &calls, b" # ", &character.repeat(1000)].concat();
            let file = SourceFile::new("test.rb".into(), source.clone());
            let raw = file.diagnostics().errors.clone();
            assert_eq!(raw.len(), 100);
            assert!(raw.iter().all(|d| d.range == (source.len()..source.len())));
            let mut width_calls = 0;
            let mut out = Vec::new();
            check_fatal_with_width(&file, &mut out, |bytes| {
                width_calls += 1;
                file.encoded_character_width(bytes)
            });
            assert_eq!(out.len(), raw.len());
            for (found, original) in out.iter().zip(&raw) {
                assert_eq!(found.message, original.message);
                assert_eq!(
                    found.range,
                    Some(source.len() - character.len()..source.len())
                );
            }
            assert!(
                width_calls <= source.len(),
                "{width_calls} width calls for {} source bytes and {} EOF errors",
                source.len(),
                raw.len()
            );
            assert_eq!(file.diagnostics().errors, raw);
        }
    }

    #[test]
    fn interior_empty_ranges_expand_forward_by_one_encoded_character() {
        for (header, character) in [
            (b"".as_slice(), "é".as_bytes()),
            (b"\xef\xbb\xbf".as_slice(), "中".as_bytes()),
            (b"# coding: Shift_JIS\r\n", b"\x82\xa0".as_slice()),
            (b"# coding: EUC-JP\n", b"\xa4\xa2".as_slice()),
            (b"# coding: ISO-8859-1\n", b"\xe9".as_slice()),
        ] {
            let source = [header, b"[1", character, b"]"].concat();
            let file = SourceFile::new("test.rb".into(), source.clone());
            let offset = header.len() + 2;
            let raw = &file.diagnostics().errors;
            assert_eq!(raw.len(), 1, "{source:?}");
            assert_eq!(raw[0].range, offset..offset);
            let found = diagnostics(&source);
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].range, Some(offset..offset + character.len()));
            assert_eq!(found[0].message, raw[0].message);
            assert_eq!(
                found[0].message,
                "unexpected local variable or method; expected a `,` separator for the array elements"
            );
            assert_eq!(file.diagnostics().errors[0].range, offset..offset);
        }
    }

    #[test]
    fn eof_empty_ranges_expand_back_over_complete_legacy_characters() {
        for (header, character) in [
            (
                b"# coding: Shift_JIS\r\n".as_slice(),
                b"\x82\xa0".as_slice(),
            ),
            (b"# coding: EUC-JP\n".as_slice(), b"\x8f\xab\xb1".as_slice()),
            (
                b"# coding: GB18030\n".as_slice(),
                b"\x90\x30\xeb\x34".as_slice(),
            ),
            (b"# coding: ISO-8859-1\n".as_slice(), b"\xe9".as_slice()),
            // Binary characters remain bytes, not decoded UTF-8 scalars.
            (b"# coding: ascii-8bit\n".as_slice(), b"\xc3\xa9".as_slice()),
        ] {
            let source = [header, b"foo( # ", character].concat();
            let file = SourceFile::new("test.rb".into(), source.clone());
            assert_eq!(
                file.diagnostics().errors[0].range,
                source.len()..source.len()
            );
            let found = diagnostics(&source);
            assert_eq!(found.len(), 1, "{source:?}");
            let width = if header.starts_with(b"# coding: ascii-8bit") {
                1
            } else {
                character.len()
            };
            assert_eq!(found[0].range, Some(source.len() - width..source.len()));
            assert_eq!(
                found[0].message,
                "unexpected end-of-input; expected a `)` to close the arguments"
            );
            assert_eq!(file.bytes, source);
        }
    }

    #[test]
    fn mixed_diagnostics_only_deliver_errors_and_no_ast_is_required() {
        let source = b"# warning: ambiguous `*` has been interpreted as an argument prefix\nfoo *some_array\n(\n";
        let found = diagnostics(source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message, "expected a matching `)`");
        let file = SourceFile::new("test.rb".into(), source.to_vec());
        assert_eq!(file.warnings().len(), 1);
        let report = lint_one(
            Path::new("test.rb"),
            Path::new("test.rb"),
            Some(source),
            &[&Syntax],
            &Default::default(),
        );
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].position, Some((3, 2)));
        assert_eq!(
            report.errors[0].message,
            "syntax error on line 3: expected a matching `)` (Lint/Syntax)"
        );
        assert!(report.result.offenses.is_empty());
        assert_eq!(report.corrections_applied, 0);
    }

    #[test]
    fn pinned_eof_fixture_preserves_both_actual_native_prism_errors() {
        let source = b"if true\n  puts 'x'\n";
        let found = diagnostics(source);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].range, Some(source.len() - 1..source.len()));
        assert_eq!(found[1].range, Some(0..2));
    }

    #[test]
    fn parser_buffer_exception_is_global_and_distinct_from_prism_errors() {
        let source = b"# \xf9\n";
        let file = SourceFile::new("test.rb".into(), source.to_vec());
        assert!(file.diagnostics().errors.is_empty());
        let found = diagnostics(source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range, None);
        assert_eq!(found[0].message, "Invalid byte sequence in utf-8.");
    }

    #[test]
    fn only_actual_native_eof_payloads_are_opaque_not_literal_or_comment_markers() {
        for marker in [0, 4, 26] {
            let opaque = [b"p 1\n".as_slice(), &[marker], b"\xff payload\n"].concat();
            assert!(diagnostics(&opaque).is_empty());
            let literal = [b"\"".as_slice(), &[marker], b"\"; # \xff\n"].concat();
            let comment = [b"# ".as_slice(), &[marker], b" \xff\n"].concat();
            for source in [literal, comment] {
                let found = diagnostics(&source);
                assert_eq!(found.len(), 1, "{source:?}");
                assert_eq!(found[0].range, None);
                assert_eq!(found[0].message, "Invalid byte sequence in utf-8.");
            }
        }
    }

    #[test]
    fn valid_lookalikes_empty_source_warnings_heredocs_and_data() {
        for source in [
            b"".as_slice(),
            b"foo(*args)\n",
            b"foo *args\n",
            b"# (\n'('\n",
            b"puts <<~TEXT\n(\nTEXT\n__END__\n(\n",
            b"# coding: ascii-8bit\n# \xf9\n",
            b"# coding: ISO-8859-1\n# \xf9\n",
        ] {
            assert!(diagnostics(source).is_empty(), "{source:?}");
        }
    }
}
