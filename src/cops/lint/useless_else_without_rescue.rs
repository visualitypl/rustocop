//! `Lint/UselessElseWithoutRescue` — legacy begin/else without a rescue.
//! Ruby >=2.6 rejects this syntax. The normal pipeline must report a fatal
//! parser error, not run this cop on Prism's recovered tree. The direct matcher
//! retains explicit and implicit legacy bodies through bounded recovery facts.

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};

/// The legacy `Lint/UselessElseWithoutRescue` cop.
pub struct UselessElseWithoutRescue;

impl Cop for UselessElseWithoutRescue {
    fn name(&self) -> &'static str {
        "Lint/UselessElseWithoutRescue"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism retains explicit begin/else (handled below) but discards an
        // implicit body's lonely else. Only consume that exact diagnostic at
        // an actual else token, in a recovered implicit body. Never re-lex or
        // reparse source, or turn arbitrary unexpected tokens into findings.
        let mut recovery = ImplicitElseRecovery {
            candidates: result
                .errors()
                .filter_map(|error| {
                    (error.message() == "unexpected 'else', ignoring it"
                        && error.location().as_slice() == b"else")
                        .then_some((error.location().start_offset(), false))
                })
                .collect(),
        };
        if recovery.candidates.is_empty() {
            return;
        }
        recovery
            .candidates
            .sort_unstable_by_key(|&(offset, _)| offset);
        walk::walk(&mut recovery, &result.node());
        for (offset, implicit_body) in recovery.candidates {
            if implicit_body {
                self.report(offset, file, out);
            }
        }
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(begin) = node.as_begin_node() else {
            return;
        };
        if begin.rescue_clause().is_some() {
            return;
        }
        let Some(otherwise) = begin.else_clause() else {
            return;
        };
        self.report(otherwise.else_keyword_loc().start_offset(), file, out);
    }
}

impl UselessElseWithoutRescue {
    fn report(&self, offset: usize, file: &SourceFile, out: &mut Vec<Offense>) {
        let (line, col) = file.line_col(offset);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "`else` without `rescue` is useless.".into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

// Every containing syntax node overrides the candidate's owner; transparent
// Program/Statements wrappers do not. Thus an unexpected else inside an if,
// argument list, etc. cannot borrow an enclosing method's eligibility. One
// typed-child-complete walk, only when relevant recovery diagnostics exist.
struct ImplicitElseRecovery {
    candidates: Vec<(usize, bool)>,
}

impl<'pr> NodeVisitor<'pr> for ImplicitElseRecovery {
    fn enter(&mut self, node: Node<'pr>) {
        if node.as_program_node().is_some() || node.as_statements_node().is_some() {
            return;
        }
        let location = node.location();
        let start = self
            .candidates
            .partition_point(|&(offset, _)| offset < location.start_offset());
        let end = self
            .candidates
            .partition_point(|&(offset, _)| offset < location.end_offset());
        let implicit_body = node.as_def_node().is_some()
            || node.as_class_node().is_some()
            || node.as_singleton_class_node().is_some()
            || node.as_module_node().is_some()
            || node
                .as_block_node()
                .is_some_and(|block| block.opening_loc().as_slice() == b"do")
            || node
                .as_lambda_node()
                .is_some_and(|lambda| lambda.opening_loc().as_slice() == b"do");
        for (_, eligible) in &mut self.candidates[start..end] {
            *eligible = implicit_body;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's useless_else_without_rescue_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::cops::run_cops;

    fn recovered_shape(source: &str) -> Vec<Offense> {
        let parsed = crate::parser::parse(source.as_bytes());
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let mut out = vec![];
        assert!(run_cops(&[&UselessElseWithoutRescue], &parsed, &file, &mut out).is_empty());
        out
    }

    #[test]
    fn legacy_else_location_and_message_from_recovered_tree() {
        let source = "begin\n  do_something\nelse\n  handle_unknown_errors\nend\n";
        assert!(
            crate::parser::parse(source.as_bytes())
                .errors()
                .next()
                .is_some()
        );
        let out = recovered_shape(source);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].line, out[0].col), (3, 1));
        assert_eq!(out[0].message, "`else` without `rescue` is useless.");
        assert!(out[0].correction.is_none());
    }

    #[test]
    fn legacy_implicit_body_else_diagnostics() {
        for (source, col) in [
            ("def foo; work; else; other; end", 16),
            ("def self.foo; work; else; other; end", 21),
            ("class Foo; work; else; other; end", 18),
            ("class << self; work; else; other; end", 22),
            ("module Foo; work; else; other; end", 19),
            ("foo do; work; else; other; end", 15),
        ] {
            let out = recovered_shape(source);
            assert_eq!(out.len(), 1, "{source}");
            assert_eq!((out[0].line, out[0].col), (1, col), "{source}");
            assert_eq!(out[0].message, "`else` without `rescue` is useless.");
            assert!(out[0].correction.is_none());
            let fixed = crate::pipeline::lint_one(
                std::path::Path::new("example.rb"),
                std::path::Path::new("example.rb"),
                Some(source.as_bytes()),
                &[],
                &crate::config::ResolvedConfig::default(),
            );
            assert!(fixed.result.offenses.is_empty());
            assert!(!fixed.errors.is_empty());
        }
    }

    #[test]
    fn recovery_does_not_borrow_an_outer_implicit_body() {
        for source in [
            "else; other",
            "def foo; if cond; else; else; other; end; end",
            "def foo; while cond; work; else; other; end; end",
            "foo { work; else; other }",
            "def foo; puts(else); end",
        ] {
            assert!(recovered_shape(source).is_empty(), "{source}");
        }
        let out = recovered_shape(
            "def foo\n work\n else\n other\nend\nfoo do\n work\n else\n other\nend",
        );
        assert_eq!(
            out.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 2), (8, 2)]
        );
    }

    #[test]
    fn every_pinned_spec_and_source_shape_matches_the_reference() {
        let rows: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/legacy_lint.json")).unwrap();
        let mut count = 0;
        for row in rows.as_array().unwrap() {
            if row["cop"] != UselessElseWithoutRescue.name() {
                continue;
            }
            let out = recovered_shape(row["source"].as_str().unwrap());
            assert_eq!(
                serde_json::json!(out.iter().map(|o| [o.line, o.col]).collect::<Vec<_>>()),
                row["legacy_positions"],
                "{row}"
            );
            assert_eq!(
                serde_json::json!(out.iter().map(|o| &o.message).collect::<Vec<_>>()),
                row["legacy_messages"],
                "{row}"
            );
            assert!(out.iter().all(|o| o.correction.is_none()));
            count += 1;
        }
        assert_eq!(count, 4);
    }

    #[test]
    fn accepts_rescue_else_and_other_else_shapes() {
        for source in [
            "begin\n do_something\nrescue ArgumentError\n handle_argument_error\nelse\n handle_unknown_errors\nend",
            "begin; work; end",
            "begin; work; ensure; cleanup; end",
            "def foo; work; rescue; recover; else; other; end",
            "class Foo; work; rescue; recover; else; other; end",
            "module Foo; work; rescue; recover; else; other; end",
            "foo do; work; rescue; recover; else; other; end",
            "def foo; puts 'else'; end",
            "foo do; # else\n work; end",
            "if cond; work; else; other; end",
            "case x; when 1; work; else; other; end",
        ] {
            assert!(recovered_shape(source).is_empty(), "{source}");
            assert!(
                crate::pipeline::lint_str(source)
                    .iter()
                    .all(|o| o.cop != UselessElseWithoutRescue.name())
            );
        }
    }
}
