//! `Style/RedundantFetchBlock` — literal fetch defaults, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantFetchBlock;
impl Cop for RedundantFetchBlock {
    fn name(&self) -> &'static str {
        "Style/RedundantFetchBlock"
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
        if call.name().as_slice() != b"fetch" {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if block.parameters().is_some_and(|p| {
            p.as_block_parameters_node()
                .is_none_or(|p| p.parameters().is_some() || !p.locals().is_empty())
        }) {
            return;
        }
        let args = ast::call_arguments(&call);
        let [key] = args.as_slice() else { return };
        if matches!(
            key,
            Node::SplatNode { .. }
                | Node::KeywordHashNode { .. }
                | Node::ForwardingArgumentsNode { .. }
        ) {
            return;
        }
        if call.receiver().is_some_and(|receiver| {
            receiver.as_call_node().is_some_and(|cache| {
                cache.name().as_slice() == b"cache"
                    && !cache.is_safe_navigation()
                    && cache.arguments().is_none()
                    && cache.block().is_none()
                    && cache.receiver().is_some_and(|namespace| {
                        ast::short_const_name(&namespace).as_deref() == Some("Rails")
                    })
            })
        }) {
            return;
        }
        let body = if let Some(body) = block.body() {
            let Some(statements) = body.as_statements_node() else {
                return;
            };
            if statements.body().len() != 1 {
                return;
            }
            statements.body().iter().next()
        } else {
            None
        };
        if body.as_ref().is_some_and(|b| {
            !ast::is_basic_literal(b)
                || (matches!(b, Node::StringNode { .. } | Node::SourceFileNode { .. })
                    && file.frozen_string_setting() != Some(true))
        }) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let value = body
            .as_ref()
            .map(|b| String::from_utf8_lossy(b.location().as_slice()).into_owned());
        let key = String::from_utf8_lossy(key.location().as_slice());
        let good = format!("fetch({key}, {})", value.as_deref().unwrap_or("nil"));
        let bad = format!(
            "fetch({key}) {}",
            value.map_or_else(|| "{}".into(), |v| format!("{{ {v} }}"))
        );
        let correction = cx
            .corrections_requested()
            .then(|| crate::offense::Correction {
                range: selector.start_offset()..block.location().end_offset(),
                replacement: good.clone(),
            });
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(block.location()),
            message: format!("Use `{good}` instead of `{bad}`."),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_fetch_block_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::correction_offenses, offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantFetchBlock")
            .collect()
    }
    #[test]
    fn all_applicable_pinned_calls_and_nine_unsafe_corrections() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_fetch_block_pinned.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let before = case["source"].as_str().unwrap();
            let found = correction_offenses(&RedundantFetchBlock, before);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{case}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(messages), case["messages"], "{case}");
            assert!(found.iter().all(|o| o.correction.is_some()), "{case}");
        }
    }
    #[test]
    fn literal_defaults_empty_blocks_and_safe_navigation_are_corrected() {
        for value in ["5", "2.5", ":value", "2.0r", "1i", "true", "false", "nil"] {
            for (receiver, col) in [("hash.", 6), ("hash&.", 7), ("", 1)] {
                let source = format!("{receiver}fetch(:key) {{ {value} }}");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, col));
                assert_eq!(
                    found[0].message,
                    format!("Use `fetch(:key, {value})` instead of `fetch(:key) {{ {value} }}`.")
                );
                assert!(
                    correction_offenses(&RedundantFetchBlock, &source)[0]
                        .correction
                        .is_some()
                );
            }
        }
        let found = offenses("hash.fetch(:key) {}");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "Use `fetch(:key, nil)` instead of `fetch(:key) {}`."
        );
    }
    #[test]
    fn fixed_constants_mutable_objects_and_lookalikes_are_not_candidates() {
        for source in [
            "hash.fetch(:key) { CONSTANT }",
            "hash.fetch(:key) { ::C::VALUE }",
            "hash.fetch(:key) { __ENCODING__ }",
            "hash.fetch(:key) { 'value' }",
            "hash.fetch(:key) { [] }",
            "hash.fetch(:key) { {} }",
            "hash.fetch(:key) { /x/ }",
            "hash.fetch(:key) { 1..2 }",
            "hash.fetch(:key) { :\"value_#{value}\" }",
            "hash.fetch(:key) { |k| 5 }",
            "hash.fetch(:key) { |; x| 5 }",
            "hash.fetch(:key) { compute }",
            "hash.fetch(:key) { return 5 }",
            "hash.fetch(:key) { 1; 5 }",
            "hash.fetch(:key) { 'x'.freeze }",
            "hash.fetch(:key, 5)",
            "hash.fetch { 5 }",
            "hash.fetch(:key, :other) { 5 }",
            "hash.fetch(*args) { 5 }",
            "hash&.fetch(*args) { 5 }",
            "hash.fetch(**opts) { 5 }",
            "hash.fetch(key: :value) { 5 }",
            "hash.fetch(:key, &block)",
            "hash.fetch_value(:key) { 5 }",
            "Rails.cache.fetch(:key) { 5 }",
            "::Rails.cache.fetch(:key) { 5 }",
            "Other::Rails.cache.fetch(:key) { 5 }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn frozen_strings_and_braced_hash_keys() {
        let source = "# frozen_string_literal: true\nhash.fetch(:key) { 'value' }\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 6));
        assert_eq!(
            found[0].message,
            "Use `fetch(:key, 'value')` instead of `fetch(:key) { 'value' }`."
        );
        assert!(offenses(&source.replace(": true", ": false")).is_empty());
        assert_eq!(offenses("hash.fetch({ key: :value }) { 5 }").len(), 1);
        assert_eq!(offenses("hash.fetch(:key) { || 5 }").len(), 1);
        assert_eq!(offenses("Other.cache.fetch(:key) { 5 }").len(), 1);
    }
}
