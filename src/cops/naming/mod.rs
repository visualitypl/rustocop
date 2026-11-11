//! The `Naming` department: naming conventions (snake_case methods and
//! variables, CamelCase classes and modules, SCREAMING_SNAKE_CASE constants,
//! descriptive parameters, ASCII identifiers and normalcase numbering).
//!
//! v0.1 ships MethodName, VariableName, ConstantName, ClassAndModuleCamelCase,
//! and MethodParameterName at RuboCop's default settings, with no style
//! parameters. The Naming policy adds AsciiIdentifiers,
//! BlockParameterName, VariableNumber, AccessorMethodName, PredicatePrefix,
//! BinaryOperatorParameterName, RescuedExceptionsVariableName, BlockForwarding,
//! HeredocDelimiterCase, HeredocDelimiterNaming, MemoizedInstanceVariableName and
//! PredicateMethod at pinned fixed defaults. InclusiveLanguage is opt-in with
//! three fixed ambiguous-suggestion terms (no edits). Memoization and predicate
//! results remain detection-only. FileName checks the supplied basename with
//! fixed shebang/standard-filename exemptions, never renaming or probing files.
//! The rename and block-forwarding cops reuse lazy binding proofs; the heredoc delimiter cops share physical heredoc boundaries.

mod accessor_method_name;
mod ascii_identifiers;
mod binary_operator_parameter_name;
mod block_forwarding;
mod block_parameter_name;
mod class_and_module_camel_case;
pub(crate) mod class_emitters;
mod constant_name;
mod file_name;
mod format;
pub(crate) mod heredoc;
mod heredoc_delimiter_case;
mod heredoc_delimiter_naming;
mod inclusive_language;
pub(crate) mod local_rename;
pub(crate) mod memoized_instance_variable_name;
mod method_name;
mod method_parameter_name;
pub(crate) mod predicate_method;
mod predicate_prefix;
pub(crate) mod rescued_exceptions_variable_name;
mod variable_name;
mod variable_number;

pub use accessor_method_name::AccessorMethodName;
pub use ascii_identifiers::AsciiIdentifiers;
pub use binary_operator_parameter_name::BinaryOperatorParameterName;
pub use block_forwarding::BlockForwarding;
pub use block_parameter_name::BlockParameterName;
pub use class_and_module_camel_case::ClassAndModuleCamelCase;
pub use constant_name::ConstantName;
pub use file_name::FileName;
pub use heredoc_delimiter_case::HeredocDelimiterCase;
pub use heredoc_delimiter_naming::HeredocDelimiterNaming;
pub use inclusive_language::InclusiveLanguage;
pub use memoized_instance_variable_name::MemoizedInstanceVariableName;
pub use method_name::MethodName;
pub use method_parameter_name::MethodParameterName;
pub use predicate_method::PredicateMethod;
pub use predicate_prefix::PredicatePrefix;
pub use rescued_exceptions_variable_name::RescuedExceptionsVariableName;
pub use variable_name::VariableName;
pub use variable_number::VariableNumber;

#[cfg(test)]
mod pinned_naming_tests {
    //! Pinned RuboCop Naming specs (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    //! The Ruby completeness test checks every source, expected location/message,
    //! and individually reasoned fixed-config/Prism exclusion against the pin.
    #[test]
    fn all_applicable_pinned_calls_have_exact_pipeline_diagnostics() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns02_pinned.json")).unwrap();
        let calls = fixture["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 38);
        assert_pinned_diagnostics(calls);
    }

    #[test]
    fn all_pinned_source_documentation_has_exact_fixed_policy_diagnostics() {
        let calls: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns02_source.json")).unwrap();
        assert_eq!(calls.len(), 46);
        assert_pinned_diagnostics(&calls);
    }

    #[test]
    fn ns03_all_applicable_pinned_calls_have_exact_pipeline_diagnostics() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns03_pinned.json")).unwrap();
        let calls = fixture["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 34);
        assert_pinned_diagnostics(calls);
    }

    #[test]
    fn ns03_all_source_documentation_has_exact_fixed_policy_diagnostics() {
        let calls: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns03_source.json")).unwrap();
        assert_eq!(calls.len(), 22);
        assert_pinned_diagnostics(&calls);
    }

    #[test]
    fn ns04_every_pinned_call_and_source_example_matches() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns04_pinned.json")).unwrap();
        let calls = fixture["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 48);
        assert_pinned_diagnostics(calls);
        let source: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns04_source.json")).unwrap();
        assert_eq!(source.len(), 8);
        assert_pinned_diagnostics(&source);
    }

    #[test]
    fn ns04_all_pinned_corrections_are_canonical_or_individually_withheld() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns04_pinned.json")).unwrap();
        let mut canonical = 0;
        let mut withheld = 0;
        for call in fixture["calls"].as_array().unwrap() {
            let Some(corrected) = call["correction"].as_str() else {
                continue;
            };
            let cop: &dyn crate::cops::Cop = if call["cop"] == "Naming/BinaryOperatorParameterName"
            {
                &super::BinaryOperatorParameterName
            } else {
                &super::RescuedExceptionsVariableName
            };
            let source = call["source"].as_str().unwrap();
            if cop.name() == "Naming/RescuedExceptionsVariableName"
                && [372, 418, 441].contains(&call["spec_line"].as_u64().unwrap())
            {
                let found = crate::cops::testing::correction_offenses(cop, source);
                assert!(!found.is_empty());
                assert!(found.iter().all(|o| o.correction.is_none()), "{call}");
                withheld += 1;
            } else {
                crate::cops::testing::assert_correction(cop, source, corrected);
                canonical += 1;
            }
        }
        assert_eq!((canonical, withheld), (22, 3));
    }

    #[test]
    fn ns05_all_pinned_calls_source_and_corrections_are_accounted_for() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns05_pinned.json")).unwrap();
        let calls = fixture["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 20);
        assert_pinned_diagnostics(calls);
        let source: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns05_source.json")).unwrap();
        assert_eq!(source.len(), 5);
        assert_pinned_diagnostics(&source);
        let mut counts = (0, 0);
        for call in calls {
            if let Some(after) = call["correction"].as_str() {
                let before = call["source"].as_str().unwrap();
                // A symbol-to-proc or an unrelated &bar conversion can call
                // reflective code. These two pinned examples retain findings.
                if [47, 109].contains(&call["spec_line"].as_u64().unwrap()) {
                    crate::cops::testing::assert_correction(
                        &super::BlockForwarding,
                        before,
                        before,
                    );
                    counts.1 += 1;
                } else {
                    crate::cops::testing::assert_correction(&super::BlockForwarding, before, after);
                    counts.0 += 1;
                }
            }
        }
        assert_eq!(counts, (9, 2));
    }

    #[test]
    fn ns06_every_pinned_call_and_source_example_matches() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns06_pinned.json")).unwrap();
        let calls = fixture["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 34);
        assert_pinned_diagnostics(calls);
        let source: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns06_source.json")).unwrap();
        assert_eq!(source.len(), 7);
        assert_pinned_diagnostics(&source);
    }

    #[test]
    fn ns06_exact_label_scope_quote_unicode_and_collision_matrix() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns06_syntax.json")).unwrap();
        assert_eq!(rows.len(), 122);
        assert_pinned_diagnostics(&rows);
    }

    #[test]
    fn ns06_all_nine_pinned_corrections_are_exact_canonical_edits() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns06_pinned.json")).unwrap();
        let mut count = 0;
        for call in fixture["calls"].as_array().unwrap() {
            if let Some(after) = call["correction"].as_str() {
                crate::cops::testing::assert_correction(
                    &super::HeredocDelimiterCase,
                    call["source"].as_str().unwrap(),
                    after,
                );
                count += 1;
            }
        }
        assert_eq!(count, 9);
    }

    #[test]
    fn ns03_many_sibling_definitions_and_macros_need_no_root_rescans() {
        let source = "def get_value; end\ndef set_value(value); end\ndef is_ready; false; end\ndefine_singleton_method(:does_work) {}\n".repeat(2000);
        let found = crate::pipeline::lint_str_with(
            &[&super::AccessorMethodName, &super::PredicatePrefix],
            &source,
        );
        assert_eq!(found.len(), 8000);
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn ns07_every_applicable_spec_and_source_example_matches() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns07_pinned.json")).unwrap();
        assert_eq!(fixture["calls"].as_array().unwrap().len(), 48);
        assert_eq!(fixture["excluded"].as_array().unwrap().len(), 26);
        assert_pinned_diagnostics(fixture["calls"].as_array().unwrap());
        let source: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns07_source.json")).unwrap();
        assert_eq!(source.len(), 23);
        assert_pinned_diagnostics(&source);
    }

    #[test]
    fn ns07_exact_method_body_children_and_ownership_matrix() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns07_syntax.json")).unwrap();
        assert_eq!(rows.len(), 226);
        let mut failures = Vec::new();
        for row in &rows {
            let actual = crate::pipeline::lint_str_with(
                &[&super::MemoizedInstanceVariableName],
                row["source"].as_str().unwrap(),
            );
            let positions: Vec<_> = actual.iter().map(|o| vec![o.line, o.col]).collect();
            if serde_json::to_value(positions).unwrap() != row["positions"] {
                failures.push(format!(
                    "{}: expected {} actual {:?}",
                    row["case"], row["positions"], actual
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        assert_pinned_diagnostics(&rows);
    }

    fn assert_pinned_diagnostics(calls: &[serde_json::Value]) {
        for call in calls {
            let cop = call["cop"].as_str().unwrap();
            let source = call["source"].as_str().unwrap();
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{call}"
            );
            let actual: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == cop)
                .collect();
            let positions: Vec<_> = actual.iter().map(|o| (o.line, o.col)).collect();
            let expected: Vec<_> = call["positions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_u64().unwrap() as u32, p[1].as_u64().unwrap() as u32))
                .collect();
            assert_eq!(positions, expected, "{call}");
            let messages: Vec<_> = actual.iter().map(|o| o.message.as_str()).collect();
            let expected: Vec<_> = call["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap())
                .collect();
            assert_eq!(messages, expected, "{call}");
            assert!(actual.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn many_sibling_scopes_need_no_root_rescans_or_correction_indexes() {
        let source = "foo { |aB| föö_1 = 1; föö_1 }\n".repeat(2000);
        let offenses = crate::pipeline::lint_str_with(
            &[
                &super::AsciiIdentifiers,
                &super::BlockParameterName,
                &super::VariableNumber,
            ],
            &source,
        );
        assert_eq!(
            offenses
                .iter()
                .filter(|o| o.cop == "Naming/AsciiIdentifiers")
                .count(),
            4000
        );
        assert_eq!(
            offenses
                .iter()
                .filter(|o| o.cop == "Naming/BlockParameterName")
                .count(),
            2000
        );
        assert_eq!(
            offenses
                .iter()
                .filter(|o| o.cop == "Naming/VariableNumber")
                .count(),
            2000
        );
        assert!(offenses.iter().all(|o| o.correction.is_none()));
    }
}
