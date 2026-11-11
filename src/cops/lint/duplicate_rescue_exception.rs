//! `Lint/DuplicateRescueException` — flags an exception class rescued more
//! than once by the same `begin`/`def`/block body.
//!
//! Matches RuboCop's semantics: exceptions (including splats such as
//! `*ERRORS`) are compared structurally across all `rescue` clauses of one
//! body; modifier `rescue` is not checked.

use std::collections::HashSet;

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast::fingerprint_for_duplicate},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/DuplicateRescueException` cop.
pub struct DuplicateRescueException;

impl Cop for DuplicateRescueException {
    fn name(&self) -> &'static str {
        "Lint/DuplicateRescueException"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Rescue clauses hang off a `BeginNode` (explicit, or the implicit
        // body of a `def`, block or class), never off the walker directly.
        let Some(begin) = node.as_begin_node() else {
            return;
        };
        let mut seen = HashSet::new();
        let mut clause = begin.rescue_clause();
        while let Some(rescue) = clause {
            for exception in &rescue.exceptions() {
                if seen.insert(fingerprint_for_duplicate(&exception, file)) {
                    continue;
                }
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    exception.location().start_offset(),
                    file,
                );
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Duplicate `rescue` exception detected.".to_string(),
                    line,
                    end_line: file.last_line_of(exception.location()),
                    col,
                });
            }
            clause = rescue.subsequent();
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/duplicate_rescue_exception_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateRescueException")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_duplicate_exception_with_message() {
        let found = offenses(
            "begin\n  something\nrescue FirstError\nrescue SecondError, FirstError\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (4, 21));
        assert_eq!(found[0].message, "Duplicate `rescue` exception detected.");
    }

    #[test]
    fn flags_duplicate_exception_splat() {
        assert_eq!(
            positions("begin\n  something\nrescue *ERRORS\nrescue SecondError, *ERRORS\nend\n"),
            vec![(4, 21)]
        );
    }

    #[test]
    fn flags_multiple_duplicate_exceptions() {
        assert_eq!(
            positions(
                "begin\n  something\nrescue FirstError\nrescue SecondError\nrescue FirstError\nrescue SecondError\nend\n"
            ),
            vec![(5, 8), (6, 8)]
        );
    }

    #[test]
    fn flags_duplicate_with_else_branch() {
        assert_eq!(
            positions(
                "begin\n  something\nrescue FirstError\nrescue SecondError, FirstError\nelse\nend\n"
            ),
            vec![(4, 21)]
        );
    }

    #[test]
    fn flags_duplicate_with_empty_rescue_branch() {
        assert_eq!(
            positions(
                "begin\n  something\nrescue FirstError\nrescue SecondError, FirstError\nrescue\nend\n"
            ),
            vec![(4, 21)]
        );
    }

    #[test]
    fn flags_implicit_begin_bodies() {
        assert_eq!(
            positions("def foo\n  bar\nrescue A\n  1\nrescue A => e\n  2\nend\n"),
            vec![(5, 8)]
        );
        assert_eq!(
            positions("foo do\n  bar\nrescue A, B\nrescue B\nend\n"),
            vec![(4, 8)]
        );
        assert_eq!(positions("begin\nrescue A, A\nend\n"), vec![(2, 11)]);
        assert_eq!(
            positions("begin\nrescue Foo::Bar\nrescue Foo::Bar\nend\n"),
            vec![(3, 8)]
        );
    }

    #[test]
    fn accepts_distinct_exceptions() {
        assert!(
            offenses("begin\n  something\nrescue FirstError\nrescue SecondError\nend\n").is_empty()
        );
    }

    #[test]
    fn complex_hash_keys_preserve_component_types_and_normalize_zero() {
        for (a, b, duplicate) in [
            ("1i", "1ri", false),
            ("1i", "1.0i", false),
            ("0.0i", "-0.0i", true),
            ("1ri", "1.0ri", true),
            ("[1i]", "[1ri]", false),
            ("[0.0i]", "[-0.0i]", true),
        ] {
            let source = format!("begin\nx\nrescue {a}\nrescue {b}\nend");
            assert_eq!(
                positions(&source),
                if duplicate { vec![(4, 8)] } else { vec![] },
                "{source}"
            );
        }
    }

    #[test]
    fn normalizes_source_buffer_recursively_in_compound_keys() {
        for (a, b, duplicate) in [
            ("[/e\u{301}/im]", "[/é/mi]", true),
            ("[[/e\u{301}/]]", "[[/é/]]", true),
            ("\"#{/e\u{301}/}\"", "\"#{/é/}\"", true),
            ("[e\u{301}.f]", "[é.f]", true),
            ("[/e\u{301}/i]", "[/é/m]", false),
            ("[/e\u{301}/]", "[/è/]", false),
        ] {
            let body = format!("begin\nx\nrescue {a}\nrescue {b}\nend");
            assert_eq!(
                positions(&format!("# coding: UTF8-MAC\n{body}")),
                if duplicate { vec![(5, 8)] } else { vec![] },
                "{body}"
            );
            assert!(
                offenses(&body).is_empty(),
                "UTF-8 must retain distinct spellings: {body}"
            );
        }
    }

    #[test]
    fn compares_parser_literal_kinds_and_evaluated_source_lines() {
        assert!(offenses("begin; x; rescue :\"\"; rescue :\"\\u{}\"; end").is_empty());
        assert_eq!(
            positions("begin; x; rescue __LINE__; rescue __LINE__; end"),
            vec![(1, 35)]
        );
        assert!(offenses("begin; x; rescue __LINE__;\nrescue __LINE__; end").is_empty());
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "begin\nrescue A\nend\nbegin\nrescue A\nend\n",
            "begin\n  begin\n  rescue A\n  end\nrescue A\nend\n",
            "begin\nrescue Foo::A\nrescue Bar::A\nend\n",
            "foo rescue bar\n",
            "begin\nrescue => e\nrescue\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
