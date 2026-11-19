//! `Lint/UselessDefaultValueArgument` — a literal block supersedes a default argument.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct UselessDefaultValueArgument;
impl Cop for UselessDefaultValueArgument {
    fn name(&self) -> &'static str {
        "Lint/UselessDefaultValueArgument"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let Some(receiver) = call.receiver() else {
            return;
        };
        // Pinned `call` accepts send/csend for fetch, but Array.new uses
        // `send (const _ :Array)`: arbitrary qualifier, never safe navigation.
        // No inferred receiver types or custom receiver allowances (fixed []).
        let matches = match call.name().as_slice() {
            b"fetch" => true,
            b"new" if !call.is_safe_navigation() => {
                crate::cops::ast::short_const_name(&receiver).as_deref() == Some("Array")
            }
            _ => false,
        };
        if !matches {
            return;
        }
        if !call.block().is_some_and(|b| b.as_block_node().is_some()) {
            return;
        }
        let arguments = crate::cops::ast::call_arguments(&call);
        let [previous, default] = arguments.as_slice() else {
            return;
        };
        if default.as_keyword_hash_node().is_some() {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            default.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Block supersedes default value argument.".into(),
            line,
            col,
            end_line: file.last_line_of(default.location()),
            // From the end of the previous argument through the default.
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: previous.location().end_offset()..default.location().end_offset(),
                    replacement: String::new(),
                }),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_default_value_argument_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessDefaultValueArgument")
            .collect()
    }

    #[test]
    fn pinned_array_new_and_all_block_forms() {
        for receiver in ["Array", "::Array", "Other::Array", "object::Array"] {
            for block in [
                "{ block_value }",
                "{ |arg| arg }",
                "{ _1 }",
                "{ it }",
                "do\n value\nend",
            ] {
                let source = format!("{receiver}.new(size, default_value) {block}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, receiver.len() as u32 + 12, 1)
                );
                assert_eq!(found[0].message, "Block supersedes default value argument.");
            }
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes_and_keyword_defaults() {
        for source in [
            "x.fetch(key, default: value) { block_value }",
            "x.fetch(key, **kwarg) { block_value }",
            "x.fetch(key, default_value)",
            "x.fetch(key) { block_value }",
            "x.fetch(key) { |arg1, arg2| block_value }",
            "x.fetch(key, default_value, third_argument) { block_value }",
            "fetch(key, default_value) { |arg| arg }",
            "Array.new(size, default_value)",
            "Array.new(size)",
            "Array.new(size) { block_value }",
            "Array.new(size) { |arg1, arg2| block_value }",
            "Array.new(size, default_value, third_argument) { block_value }",
            "Array&.new(size, default_value) { it }",
            "(Array).new(size, default_value) { it }",
            "array.new(size, default_value) { it }",
            "Other.new(size, default_value) { it }",
            "x.fetch(key, default_value, &block)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn fetch_any_receiver_safe_navigation_and_all_block_forms() {
        for receiver in [
            "x.",
            "x&.",
            "nil.",
            "false.",
            "ENV.",
            "Thread.current.",
            "Rails.cache.",
        ] {
            for block in ["{ block_value }", "{ |arg| arg }", "{ _1 }", "{ it }"] {
                for default in ["default_value", "{}"] {
                    let source = format!("{receiver}fetch(key, {default}) {block}");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col),
                        (1, receiver.len() as u32 + 12)
                    );
                    assert_eq!(found[0].message, "Block supersedes default value argument.");
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }

    #[test]
    fn all_reference_original_bytes_exact_positions_ranges_and_messages() {
        use std::path::Path;
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/useless_default_value_argument.json"
        ))
        .unwrap();
        let config = crate::config::ResolvedConfig::default();
        let active = [&UselessDefaultValueArgument as &dyn Cop];
        let mut errors = Vec::new();
        for row in rows {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            let report = crate::pipeline::lint_one(
                Path::new("example.rb"),
                Path::new("example.rb"),
                Some(&bytes),
                &active,
                &config,
            );
            if row.get("parser_rejection").is_some() {
                assert!(!report.errors.is_empty(), "{row}");
                assert!(report.result.offenses.is_empty());
                continue;
            }
            assert!(
                report.errors.is_empty() && report.internal_errors.is_empty(),
                "{row}\n{report:?}"
            );
            let mut actual: Vec<_> = report
                .result
                .offenses
                .iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, crate::offense::Severity::Error);
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            actual.sort();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            if actual != expected {
                errors.push(format!(
                    "{}: {actual:?} != {expected:?}",
                    String::from_utf8_lossy(&bytes)
                ));
            }
        }
        assert!(
            errors.is_empty(),
            "{} mismatches:\n{}",
            errors.len(),
            errors.join("\n")
        );
    }

    #[test]
    fn pinned_fetch_default_with_literal_block() {
        let found = offenses("# example\n  x.fetch(key, default_value) { block_value }\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 16, 2));
        assert_eq!(found[0].message, "Block supersedes default value argument.");
        assert!(found[0].correction.is_none());
    }
}
