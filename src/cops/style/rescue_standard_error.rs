//! `Style/RescueStandardError` — require an explicit error class (fixed explicit style).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct RescueStandardError;
impl Cop for RescueStandardError {
    fn name(&self) -> &'static str {
        "Style/RescueStandardError"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(rescue) = node.as_rescue_node() else {
            return;
        };
        if !rescue.exceptions().is_empty() {
            return;
        }
        let keyword = rescue.keyword_loc();
        let (line, col) = file.line_col(keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid rescuing without specifying an error class.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(cx, file, keyword.end_offset()))
                .flatten(),
        });
    }
}

fn correction(
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    offset: usize,
) -> Option<crate::offense::Correction> {
    let root = cx.ancestors().next_back()?;
    // An implicit rescue uses the VM's StandardError, whereas an explicit
    // unqualified constant is lexically resolved. Reopened/inherited namespaces
    // and source-wide binding mutations are not proof of identical lookup.
    if cx.ancestors().any(|n| {
        matches!(
            n,
            Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. }
        )
    }) || file.exception_safety(root).changed()
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: offset..offset,
        replacement: " StandardError".into(),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/rescue_standard_error_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};

    #[test]
    fn every_applicable_pinned_diagnostic_and_correction() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/rescue_standard_error_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 17);
        let mut fixes = 0;
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == RescueStandardError.name())
                .collect();
            let expected: Vec<Vec<u32>> =
                serde_json::from_value(case["positions"].clone()).unwrap();
            assert_eq!(
                found
                    .iter()
                    .map(|o| vec![o.line, o.col])
                    .collect::<Vec<_>>(),
                expected,
                "{source}"
            );
            let messages: Vec<String> = serde_json::from_value(case["messages"].clone()).unwrap();
            assert_eq!(
                found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
                messages,
                "{source}"
            );
            if let Some(after) = case["correction"].as_str() {
                assert_correction(&RescueStandardError, source, after);
                fixes += 1;
            }
        }
        assert_eq!(fixes, 4);
    }

    #[test]
    fn blocks_lambdas_rescue_chains_empty_bodies_and_inline_comments() {
        for (before, after) in [
            (
                "foo do; work; rescue; end",
                "foo do; work; rescue StandardError; end",
            ),
            (
                "-> do; work; rescue => e; e; end",
                "-> do; work; rescue StandardError => e; e; end",
            ),
            (
                "begin; work; rescue Error; a; rescue; b; ensure; cleanup; end",
                "begin; work; rescue Error; a; rescue StandardError; b; ensure; cleanup; end",
            ),
            (
                "begin\nrescue # keep\nend",
                "begin\nrescue StandardError # keep\nend",
            ),
        ] {
            assert_correction(&RescueStandardError, before, after);
        }
    }

    #[test]
    fn uncertain_constant_bindings_keep_every_offense_without_edits() {
        for source in [
            "module Existing; begin; rescue; end; end",
            "class Existing; def f; work; rescue; end; end",
            "class Existing < Base; def f; work; rescue; end; end",
            "class << object; def f; work; rescue; end; end",
            "StandardError = Exception; begin; rescue; end",
            "Object.const_set(:StandardError, Exception); begin; rescue; end",
            "require 'changes'; begin; rescue; end",
            "autoload :StandardError, 'errors'; begin; rescue; end",
            "def change; Object.const_set(:StandardError, Exception); end; begin; rescue; end",
        ] {
            let found = correction_offenses(&RescueStandardError, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn insertion_preserves_bom_crlf_line_values_comments_and_data_bytes() {
        for before in [
            "\u{feff}begin; work; rescue => e; e; end\r\np __LINE__\r\n__END__\r\npayload",
            "# coding: UTF-8\n# frozen_string_literal: true\nbegin\n  'é'\nrescue # retained\n  'fallback'\nensure\n  cleanup\nend\n",
        ] {
            assert_correction(
                &RescueStandardError,
                before,
                &before.replacen("rescue", "rescue StandardError", 1),
            );
        }
    }

    #[test]
    fn lookalikes_are_not_implicit_rescues() {
        for source in [
            "foo rescue 42",
            "object.rescue",
            "begin; rescue *classes; end",
            "begin; rescue ::StandardError; end",
            "begin; rescue Namespace::StandardError; end",
            "begin; rescue Error, StandardError => e; end",
        ] {
            assert!(
                correction_offenses(&RescueStandardError, source).is_empty(),
                "{source}"
            );
        }
    }
}
