//! `Lint/DuplicateCaseCondition` — flags a `when` condition that repeats an
//! earlier condition of the same `case`.
//!
//! Matches RuboCop's semantics: conditions are compared structurally (see
//! [`crate::cops::ast`]), across all `when` branches, including values listed
//! together in one branch. Pattern-matching `case ... in` is not checked.

use std::collections::HashSet;

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast::fingerprint_for_duplicate},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/DuplicateCaseCondition` cop.
pub struct DuplicateCaseCondition;

impl Cop for DuplicateCaseCondition {
    fn name(&self) -> &'static str {
        "Lint/DuplicateCaseCondition"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(case) = node.as_case_node() else {
            return;
        };
        let mut seen = HashSet::new();
        for branch in &case.conditions() {
            let Some(when) = branch.as_when_node() else {
                continue;
            };
            for condition in &when.conditions() {
                if seen.insert(fingerprint_for_duplicate(&condition, file)) {
                    continue;
                }
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    condition.location().start_offset(),
                    file,
                );
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Duplicate `when` condition detected.".to_string(),
                    line,
                    end_line: file.last_line_of(condition.location()),
                    col,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/duplicate_case_condition_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    const MESSAGE: &str = "Duplicate `when` condition detected.";

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateCaseCondition")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_repeated_condition_with_message() {
        let found = offenses(
            "case x\nwhen false\n  first_method\nwhen true\n  second_method\nwhen false\n  third_method\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (6, 6));
        assert_eq!(found[0].message, MESSAGE);
    }

    #[test]
    fn flags_subsequent_repeated_condition() {
        assert_eq!(
            positions("case x\nwhen false\n  first_method\nwhen false\n  second_method\nend\n"),
            vec![(4, 6)]
        );
    }

    #[test]
    fn flags_multiple_repeated_conditions() {
        assert_eq!(
            positions(
                "case x\nwhen false\n  first_method\nwhen true\n  second_method\nwhen false\n  third_method\nwhen true\n  fourth_method\nend\n"
            ),
            vec![(6, 6), (8, 6)]
        );
    }

    #[test]
    fn flags_repeated_multi_value_conditions() {
        assert_eq!(
            positions("case x\nwhen a, b\n  first_method\nwhen b, a\n  second_method\nend\n"),
            vec![(4, 6), (4, 9)]
        );
    }

    #[test]
    fn flags_repeated_logical_operator_conditions() {
        assert_eq!(
            positions("case x\nwhen a && b\n  first_method\nwhen a && b\n  second_method\nend\n"),
            vec![(4, 6)]
        );
    }

    #[test]
    fn flags_values_repeated_within_one_branch_and_across_spellings() {
        assert_eq!(positions("case x\nwhen 1, 1\nend\n"), vec![(2, 9)]);
        assert_eq!(
            positions("case x\nwhen 'a'\n  1\nwhen \"a\"\n  2\nend\n"),
            vec![(4, 6)]
        );
        assert_eq!(
            positions("case x\nwhen *LIST\n  1\nwhen *LIST\n  2\nend\n"),
            vec![(4, 6)]
        );
    }

    #[test]
    fn flags_nested_cases_independently() {
        let source = "case x\nwhen 1\n  case y\n  when 1\n  when 2\n  end\nwhen 1\nend\n";
        assert_eq!(positions(source), vec![(7, 6)]);
    }

    #[test]
    fn accepts_trivial_case() {
        assert!(offenses("case x\nwhen false\n  first_method\nend\n").is_empty());
    }

    #[test]
    fn accepts_non_redundant_case() {
        assert!(
            offenses("case x\nwhen false\n  first_method\nwhen true\n  second_method\nend\n")
                .is_empty()
        );
    }

    #[test]
    fn accepts_non_redundant_case_with_else() {
        assert!(
            offenses(
                "case x\nwhen false\n  method_name\nwhen true\n  second_method\nelse\n  third_method\nend\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn accepts_similar_but_not_equivalent_and_expressions() {
        assert!(
            offenses(
                "case x\nwhen something && another && other\n  first_method\nwhen something && another\n  second_method\nend\n"
            )
            .is_empty()
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
            let source = format!("case x\nwhen {a}\nwhen {b}\nend");
            assert_eq!(
                positions(&source),
                if duplicate { vec![(3, 6)] } else { vec![] },
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
            let body = format!("case x\nwhen {a}\nwhen {b}\nend");
            assert_eq!(
                positions(&format!("# coding: UTF8-MAC\n{body}")),
                if duplicate { vec![(4, 6)] } else { vec![] },
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
        assert!(offenses("case x; when :\"\"; when :\"\\u{}\"; end").is_empty());
        assert_eq!(
            positions("case x; when __LINE__; when __LINE__; end"),
            vec![(1, 29)]
        );
        assert!(offenses("case x; when __LINE__;\nwhen __LINE__; end").is_empty());
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "case x\nwhen 1\nwhen 1.0\nend\n",
            "case x\nwhen :a\nwhen 'a'\nend\n",
            "case x\nwhen foo(1)\nwhen foo(2)\nend\n",
            "case x\nin 1\nin 1\nend\n",
            "case x\nwhen 1 then a\nend\ncase y\nwhen 1 then b\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
