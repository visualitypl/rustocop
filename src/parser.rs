//! Fixed Ruby 3.4 Prism parsing on a dedicated stack segment, on the calling thread.
//!
//! Our pinned Prism 1.9 limits recursion to 10,000 (`PRISM_DEPTH_MAXIMUM`),
//! but can exhaust a default 2 MiB Rust worker stack before reaching that
//! diagnostic. Reserve ample headroom for that C budget, independently of
//! RUST_MIN_STACK / Rayon settings. No threads or !Send parser handles cross
//! thread boundaries. Only parsing uses this segment; Rust analyses retain
//! their separate, much smaller AST-depth budget. The local sys patch extends
//! Prism's existing recursion guard to patterns and destructured parameters;
//! see `vendor/ruby-prism-sys/RUSTOCOP.md` before upgrading either binding.

mod cleanup;
mod continuations;
#[cfg(test)]
mod english_not;
pub(crate) use continuations::Continuation;

use std::ops::Deref;

use ruby_prism::ParseResult;

use crate::cops::depth::{self, DepthError};

const PARSER_STACK_SIZE: usize = 64 * 1024 * 1024;

/// A parse whose deep (including recovered) tree is always destroyed with
/// bounded stack usage, even when a caller only reads metadata or unwinds.
pub(crate) struct Parsed<'src> {
    result: Option<ParseResult<'src>>,
    depth_error: Option<DepthError>,
    pub(crate) continuations: Vec<Continuation>,
}

impl Parsed<'_> {
    pub(crate) fn depth_error(&self) -> Option<&DepthError> {
        self.depth_error.as_ref()
    }
}

impl<'src> Deref for Parsed<'src> {
    type Target = ParseResult<'src>;

    fn deref(&self) -> &Self::Target {
        self.result.as_ref().expect("live parse")
    }
}

impl Drop for Parsed<'_> {
    fn drop(&mut self) {
        let result = self.result.take().expect("live parse");
        if self.depth_error.is_some() {
            cleanup::drop_rejected(result);
        }
        // Shallow trees can use Prism's normal destructor, including syntax
        // errors: the preflight checks recovered trees as well as valid ones.
    }
}

/// All production parses, including metadata fallbacks and correction probes,
/// go through this fixed Ruby 3.4 entrypoint, not Prism's evolving latest grammar.
/// stacker switches stacks on this same thread, so ParseResult's !Send contract
/// and the caller's panic isolation hold.
pub(crate) fn parse(source: &[u8]) -> Parsed<'_> {
    let (result, continuations) = continuations::collect(|| {
        stacker::grow(PARSER_STACK_SIZE, || ruby_prism::parse_ruby_3_4(source))
    });
    let depth_error = depth::check(&result.node().as_program_node().unwrap()).err();
    Parsed {
        result: Some(result),
        depth_error,
        continuations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;

    #[test]
    fn legacy_identifier_warnings_are_printable_without_changing_byte_ranges() {
        for (encoding, identifier) in [
            ("ISO-8859-1", b"\xe9".as_slice()),
            ("Windows-1252", b"\xe9".as_slice()),
            ("Shift_JIS", b"\x82\xa0".as_slice()),
            ("UTF-8", "é".as_bytes()),
            ("US-ASCII", b"value".as_slice()),
        ] {
            let mut source = format!("# encoding: {encoding}\ndef example\n  ").into_bytes();
            let start = source.len();
            source.extend_from_slice(identifier);
            source.extend_from_slice(b" = 1\nend\n");
            let parsed = parse(&source);
            assert_eq!(parsed.errors().count(), 0, "{encoding}");
            let warnings: Vec<_> = parsed.warnings().collect();
            assert_eq!(warnings.len(), 1, "{encoding}");
            let warning = &warnings[0];
            let expected = format!(
                "assigned but unused variable - {}",
                String::from_utf8_lossy(identifier)
            );
            assert_eq!(warning.message(), expected, "{encoding}");
            assert_eq!(
                matches!(warning.message(), std::borrow::Cow::Borrowed(_)),
                std::str::from_utf8(identifier).is_ok(),
                "valid UTF-8 diagnostics should not allocate: {encoding}"
            );
            assert_eq!(warning.location().as_slice(), identifier);
            assert_eq!(warning.location().start_offset(), start);
            assert_eq!(warning.location().end_offset(), start + identifier.len());

            // Both main-parse recording and lazy metadata fallback use the
            // binding's safe diagnostic text, but keep original source ranges.
            let recorded = SourceFile::new("legacy.rb".into(), source.clone());
            recorded.record_warnings(&parsed);
            let fallback = SourceFile::new("legacy.rb".into(), source.clone());
            assert_eq!(recorded.warnings(), fallback.warnings());
            assert_eq!(recorded.warnings()[0].message, expected);
            assert_eq!(
                recorded.warnings()[0].range,
                start..start + identifier.len()
            );
            assert_eq!(recorded.bytes, source);
        }
    }

    #[test]
    fn legacy_identifier_syntax_errors_keep_raw_locations_and_printable_messages() {
        for (encoding, identifier) in [
            ("ISO-8859-1", b"\xe9".as_slice()),
            ("Windows-1252", b"\xe9".as_slice()),
            ("Shift_JIS", b"\x82\xa0".as_slice()),
            ("UTF-8", "é".as_bytes()),
            ("US-ASCII", b"value".as_slice()),
        ] {
            let mut source = format!("# encoding: {encoding}\ncase value\nin ^").into_bytes();
            let start = source.len();
            source.extend_from_slice(identifier);
            source.extend_from_slice(b"\nend\n");
            let parsed = parse(&source);
            let errors: Vec<_> = parsed.errors().collect();
            assert_eq!(errors.len(), 1, "{encoding}");
            let error = &errors[0];
            assert_eq!(
                error.message(),
                format!(
                    "{}: no such local variable",
                    String::from_utf8_lossy(identifier)
                ),
                "{encoding}"
            );
            assert_eq!(error.location().as_slice(), identifier);
            assert_eq!(error.location().start_offset(), start);
            assert_eq!(error.location().end_offset(), start + identifier.len());
            let file = SourceFile::new("invalid.rb".into(), source.clone());
            file.record_encoding(&parsed);
            assert_eq!(file.line_col(start), (3, 5));
            assert_eq!(file.bytes, source);
        }
    }

    #[test]
    fn ruby34_exposed_when_not_does_not_enable_ordinary_command_arguments() {
        // Native Ruby 3.4 accepts these, unlike Prism 1.9's latest grammar.
        for source in [
            "case\nwhen not false\n :yes\nend",
            "case :x\nwhen not false, not true\n :yes\nend",
            "case\nwhen false, not false\n :yes\nend",
            "case\nwhen not probe false\n :yes\nend",
            "case\nwhen not # keep\n false\n :yes\nend",
        ] {
            assert_eq!(parse(source.as_bytes()).errors().count(), 0, "{source}");
            assert!(ruby_prism::parse(source.as_bytes()).errors().count() > 0);
        }
        // The compatibility exception is only for the exposed when operand;
        // ordinary command calls and `and` remain syntax errors on Ruby 3.4.
        for source in [
            "case\nwhen probe false\n :yes\nend",
            "case\nwhen obj.probe false\n :yes\nend",
            "case\nwhen not false and true\n :yes\nend",
        ] {
            assert!(parse(source.as_bytes()).errors().count() > 0, "{source}");
        }
        // These contexts already accepted English `not` upstream. Nested
        // value contexts are audited separately in english_not's native matrix.
        for source in [
            "case\nwhen probe(not false)\n :yes\nend",
            "if not false\n :yes\nend",
        ] {
            assert_eq!(
                parse(source.as_bytes()).errors().count(),
                ruby_prism::parse(source.as_bytes()).errors().count(),
                "{source}"
            );
        }
    }
    #[test]
    fn rejects_ruby_4_leading_logical_chains_at_ruby_34_positions() {
        for operator in ["&&", "||"] {
            for newline in ["\n", "\r\n"] {
                let source = format!("foo{newline}  {operator} bar{newline}");
                let parsed = parse(source.as_bytes());
                let error = parsed
                    .errors()
                    .next()
                    .expect("Ruby 3.4 rejects leading logical chains");
                assert_eq!(error.location().start_offset(), 3 + newline.len() + 2);
                let token = if operator == "||" { "|" } else { "&&" };
                assert_eq!(
                    error.message(),
                    format!("unexpected '{token}', ignoring it")
                );
                assert_eq!(parsed.depth_error(), None);
            }
        }
    }

    #[test]
    fn fixes_only_the_grammar_not_the_other_null_option_defaults() {
        use ruby_prism::{Node, ParseResult};
        use ruby_prism_sys as sys;

        fn parser_state(
            result: &ParseResult<'_>,
        ) -> (sys::pm_options_version_t, i32, i8, bool, bool, bool, u8) {
            let Node::ProgramNode { parser, .. } = result.node() else {
                unreachable!()
            };
            // SAFETY: the live result owns this parser, read only while borrowed.
            let parser = unsafe { parser.as_ref() };
            (
                parser.version,
                parser.start_line,
                parser.frozen_string_literal,
                parser.parsing_eval,
                parser.partial_script,
                parser.encoding_locked,
                parser.command_line,
            )
        }

        let fixed = parse(b"items.map { it }\n");
        let upstream = ruby_prism::parse(b"items.map { it }\n");
        assert_eq!(
            parser_state(&fixed),
            (
                sys::PM_OPTIONS_VERSION_CRUBY_3_4,
                1,
                0,
                false,
                false,
                false,
                0
            )
        );
        assert_eq!(
            parser_state(&upstream),
            (sys::PM_OPTIONS_VERSION_LATEST, 1, 0, false, false, false, 0)
        );
        // The local binding must not silently change the upstream default API.
        assert_eq!(ruby_prism::parse(b"foo\n  && bar\n").errors().count(), 0);
    }

    #[test]
    fn retains_null_option_shebang_diagnostics_without_implicit_x() {
        for source in [
            b"#!/usr/bin/env ruby\r\nputs :ok\r\n".as_slice(),
            b"#!/usr/bin/env ruby\nputs :ok\n",
            b"#!/usr/bin/env ruby\r",
            b"#!/usr/bin/env ruby",
            b"#!/usr/bin/env RUBY\r\nputs :ok\n",
            b"#!/bin/sh\nputs :ok\n",
            b"#!/bin/sh\r\nbefore\n#!/usr/bin/env ruby\r\nafter\n",
            b"#!/bin/sh\n)\n#!/usr/bin/env ruby\nputs :ok\n",
            b"\xef\xbb\xbf#!/usr/bin/env ruby\r\nputs :ok\n",
            b"\xef\xbb\xbf#!ruby\r\nputs :ok\n",
            b"#!\r\nputs :ok\n",
            b"#!",
            b"",
        ] {
            let fixed = parse(source);
            let upstream = ruby_prism::parse(source);
            let warnings = |result: &ParseResult<'_>| {
                result
                    .warnings()
                    .map(|w| {
                        (
                            w.message().into_owned(),
                            w.location().start_offset(),
                            w.location().end_offset(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let errors = |result: &ParseResult<'_>| {
                result
                    .errors()
                    .map(|e| {
                        (
                            e.message().into_owned(),
                            e.location().start_offset(),
                            e.location().end_offset(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(warnings(&fixed), warnings(&upstream), "{source:?}");
            assert_eq!(errors(&fixed), errors(&upstream), "{source:?}");
            assert_eq!(
                fixed
                    .node()
                    .as_program_node()
                    .unwrap()
                    .statements()
                    .body()
                    .len(),
                upstream
                    .node()
                    .as_program_node()
                    .unwrap()
                    .statements()
                    .body()
                    .len(),
                "non-Ruby shebangs must not skip source: {source:?}"
            );
        }

        #[cfg(not(windows))]
        {
            let source = b"#!/usr/bin/env ruby\r\nputs :ok\r\n";
            let parsed = parse(source);
            let warning = parsed.warnings().next().unwrap();
            assert_eq!(
                warning.message(),
                "shebang line ending with \\r may cause problems"
            );
            assert_eq!(warning.location().start_offset(), 0);
            assert_eq!(warning.location().end_offset(), 21);
        }
    }

    #[test]
    fn accepts_ruby_34_logical_continuations_with_the_expected_ast() {
        for operator in ["&&", "||"] {
            for source in [
                format!("foo {operator}\n  bar\n"),
                format!("foo \\\n  {operator} bar\n"),
                format!("(foo {operator}\n  bar)\n"),
            ] {
                let parsed = parse(source.as_bytes());
                assert_eq!(parsed.errors().count(), 0, "{source}");
                let statement = parsed
                    .node()
                    .as_program_node()
                    .unwrap()
                    .statements()
                    .body()
                    .first()
                    .unwrap();
                let expression = statement.as_parentheses_node().map_or(statement, |p| {
                    p.body()
                        .unwrap()
                        .as_statements_node()
                        .unwrap()
                        .body()
                        .first()
                        .unwrap()
                });
                assert_eq!(
                    expression.as_and_node().is_some(),
                    operator == "&&",
                    "{source}"
                );
                assert_eq!(
                    expression.as_or_node().is_some(),
                    operator == "||",
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn retains_implicit_it_and_lexical_local_scope() {
        for (source, implicit) in [
            ("items.map { it }\n", true),
            ("it = 1; items.map { it }\n", false),
            ("items.map { |it| it }\n", false),
        ] {
            let parsed = parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let call = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .last()
                .unwrap()
                .as_call_node()
                .unwrap();
            let block = call.block().unwrap().as_block_node().unwrap();
            assert_eq!(
                block
                    .parameters()
                    .is_some_and(|p| p.as_it_parameters_node().is_some()),
                implicit
            );
            let read = block
                .body()
                .unwrap()
                .as_statements_node()
                .unwrap()
                .body()
                .first()
                .unwrap();
            assert_eq!(read.as_it_local_variable_read_node().is_some(), implicit);
            if !implicit {
                assert_eq!(
                    read.as_local_variable_read_node()
                        .unwrap()
                        .name()
                        .as_slice(),
                    b"it"
                );
            }
        }
    }

    #[test]
    fn retains_full_and_anonymous_argument_forwarding() {
        let parsed = parse(b"def forward(...); target(...); end\n");
        assert_eq!(parsed.errors().count(), 0);
        let definition = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .first()
            .unwrap()
            .as_def_node()
            .unwrap();
        assert!(
            definition
                .parameters()
                .unwrap()
                .keyword_rest()
                .unwrap()
                .as_forwarding_parameter_node()
                .is_some()
        );
        let call = definition
            .body()
            .unwrap()
            .as_statements_node()
            .unwrap()
            .body()
            .first()
            .unwrap()
            .as_call_node()
            .unwrap();
        assert!(
            call.arguments()
                .unwrap()
                .arguments()
                .first()
                .unwrap()
                .as_forwarding_arguments_node()
                .is_some()
        );
        for source in [
            "def forward(*, **, &); target(*, **, &); end\n",
            "def forward(&); items.each { target(&) }; end\n",
        ] {
            assert_eq!(parse(source.as_bytes()).errors().count(), 0, "{source}");
        }
    }

    #[test]
    fn retains_string_modes_encoding_comments_warnings_and_data_metadata() {
        for (comment, frozen, mutable) in [
            ("", false, false), // Ruby 3.4 chilled string, not an explicit false.
            ("# frozen_string_literal: false\n", false, true),
            ("# frozen_string_literal: true\n", true, false),
        ] {
            let source = format!("{comment}\"text\"\n");
            let parsed = parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            let string = parsed
                .node()
                .as_program_node()
                .unwrap()
                .statements()
                .body()
                .first()
                .unwrap()
                .as_string_node()
                .unwrap();
            assert_eq!((string.is_frozen(), string.is_mutable()), (frozen, mutable));
        }
        for source in [
            b"# encoding: Windows-31J\n'\x83\\'; foo = 1; foo = 2\n__END__\nraw\xff\n".as_slice(),
            b"\xef\xbb\xbf# encoding: utf-8\n'\xc3\xa9'; foo = 1; foo = 2\n__END__\nraw\xff\n",
        ] {
            let parsed = parse(source);
            assert_eq!(parsed.errors().count(), 0);
            let recorded = SourceFile::new("metadata.rb".into(), source.to_vec());
            recorded.record_encoding(&parsed);
            recorded.record_comments(&parsed);
            recorded.record_warnings(&parsed);
            recorded.record_data_start(&parsed);
            let fallback = SourceFile::new("metadata.rb".into(), source.to_vec());
            let offset = source.windows(3).position(|s| s == b"foo").unwrap();
            assert_eq!(recorded.line_col(offset), (2, 6));
            assert_eq!(fallback.line_col(offset), recorded.line_col(offset));
            assert_eq!(fallback.comments(), recorded.comments());
            assert!(!recorded.warnings().is_empty());
            assert_eq!(fallback.warnings(), recorded.warnings());
            assert_eq!(fallback.data_start(), recorded.data_start());
            assert_eq!(&source[recorded.data_start().unwrap()..][..7], b"__END__");
        }
    }

    #[test]
    fn syntax_diagnostics_retain_encoding_aware_scalar_positions() {
        for source in [
            b"# encoding: Windows-31J\n'\x83\\'; )\n".as_slice(),
            b"\xef\xbb\xbf# encoding: utf-8\n'\xc3\xa9'; )\n",
        ] {
            let parsed = parse(source);
            let error = parsed.errors().next().unwrap();
            assert_eq!(error.message(), "unexpected ')', ignoring it");
            let file = SourceFile::new("invalid.rb".into(), source.to_vec());
            file.record_encoding(&parsed);
            assert_eq!(file.line_col(error.location().start_offset()), (2, 6));
        }
    }

    fn definitions(levels: usize) -> String {
        format!(
            "{}{}",
            "def foo(*args)\nbar(*args)\n".repeat(levels),
            "end\n".repeat(levels)
        )
    }

    #[test]
    fn parses_on_an_isolated_stack_before_checking_ast_depth() {
        let source = definitions(1000);
        let parsed = parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let file = SourceFile::new("deep.rb".into(), source.as_bytes().to_vec());
        file.record_encoding(&parsed);
        assert_eq!(file.line_col(parsed.depth_error().unwrap().offset), (60, 5));
    }

    #[test]
    fn reaches_prisms_recursion_diagnostic_and_cleans_up_recovered_trees() {
        let source = definitions(12_000);
        let parsed = parse(source.as_bytes());
        assert!(
            parsed
                .errors()
                .any(|error| error.message() == "nesting too deep")
        );
        assert!(parsed.depth_error().is_some());
    }

    #[test]
    fn hash_pattern_label_value_cycles_stop_at_the_parser_budget() {
        // Neither the label nor keyword-rest dispatch goes through a pattern
        // primitive before recursing into the next hash value.
        for prefix in ["a:", "**rest,a:"] {
            let source = format!("x in {}b\n", prefix.repeat(200_000));
            let parsed = parse(source.as_bytes());
            let error = parsed
                .errors()
                .find(|error| error.message() == "nesting too deep")
                .expect("hash values must share the parser recursion budget");
            assert!(error.location().start_offset() <= 5 + prefix.len() * 10_000);
            assert!(parsed.depth_error().is_some());
        }
        for source in ["x in {a: {b: capture}}", "x in {a: {b: capture}, **rest}"] {
            let parsed = parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            assert_eq!(parsed.depth_error(), None);
        }
    }

    #[test]
    fn deep_tree_cleanup_also_runs_during_unwinding() {
        let source = format!("root{}\ndef\n", ".foo".repeat(20_000));
        assert!(
            std::panic::catch_unwind(|| {
                let parsed = parse(source.as_bytes());
                assert!(parsed.depth_error().is_some());
                assert!(parsed.errors().next().is_some());
                panic!("unwind after reading parse metadata");
            })
            .is_err()
        );
    }

    #[test]
    fn source_metadata_fallbacks_use_isolated_parsing_and_bounded_cleanup() {
        let source = format!(
            "# encoding: utf-8\n{}__END__\nraw data\n",
            definitions(1000)
        );
        let data_offset = source.find("__END__").unwrap();
        let file = SourceFile::new("deep.rb".into(), source.into_bytes());
        assert_eq!(file.line_col(0), (1, 1));
        assert_eq!(file.comments().len(), 1);
        assert_eq!(file.comments()[0], 0..17);
        assert!(file.warnings().is_empty());
        assert_eq!(file.data_start(), Some(data_offset));
    }

    #[test]
    fn lexical_lookalikes_and_wide_programs_do_not_consume_the_depth_budget() {
        let text = "def foo(*args) [[[ ((( { end\n".repeat(12_000);
        let literal = format!("text = <<'RUBY'\n{text}RUBY\n");
        let comments = format!("# {}\n", "def foo; [({ ".repeat(12_000));
        let data = format!("x = 1\n__END__\n{text}");
        let wide = "foo(1)\n".repeat(12_000);
        for source in [literal, comments, data, wide] {
            let parsed = parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            assert_eq!(parsed.depth_error(), None);
        }
    }
}
