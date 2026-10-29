//! `Lint/ConstantResolution` — opt-in syntactic root qualification, without an index.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ConstantResolution;
impl Cop for ConstantResolution {
    fn name(&self) -> &'static str {
        "Lint/ConstantResolution"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(constant) = node.as_constant_read_node() else {
            return;
        };
        // The upstream exemption is the immediate parent's defined_module, not
        // every reference anywhere in a class header (notably A::Base).
        if cx
            .ancestors()
            .find(|p| !p.as_statements_node().is_some_and(|s| s.body().len() == 1))
            .is_some_and(|p| p.as_class_node().is_some() || p.as_module_node().is_some())
        {
            return;
        }
        if cx.parent().is_some_and(|p| {
            p.as_constant_path_target_node().is_some() || p.as_constant_path_node().is_some()
        }) && cx.ancestors().nth(1).is_some_and(|p| {
            p.as_constant_path_write_node()
                .is_some_and(|w| defines_module(&w.value()))
        }) {
            return;
        }
        let loc = constant.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset() - 1),
            message: "Fully qualify this constant to avoid possibly ambiguous resolution.".into(),
            correction: None,
        });
    }
}

/// The factory-value branch of pinned Node#defined_module0, also used by
/// DeprecatedReference's direct constant-qualifier exemption.
pub(super) fn defines_module(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        // defined_module0 accepts ordinary block, not numblock/itblock.
        let ordinary_block = !call
            .block()
            .and_then(|n| n.as_block_node())
            .and_then(|b| b.parameters())
            .is_some_and(|p| {
                matches!(
                    p,
                    Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. }
                )
            });
        !call.is_safe_navigation()
            && call.name().as_slice() == b"new"
            && ordinary_block
            && call.receiver().is_some_and(|r| {
                ast::is_global_const(&r, &["Class"]) || ast::is_global_const(&r, &["Module"])
            })
    })
}

#[cfg(test)]
mod tests {
    //! RuboCop lint/constant_resolution_spec.rb examples (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov); source-only reference replay.
    #[test]
    fn pinned_specs_sources_and_lexical_controls() {
        let registry = crate::cops::Registry::new();
        let active: Vec<_> = registry
            .cops()
            .iter()
            .filter(|c| c.name() == "Lint/ConstantResolution")
            .map(|c| c.as_ref())
            .collect();
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/constant_scoping.json"
        ))
        .unwrap();
        for row in rows
            .iter()
            .filter(|r| r["cop"] == "Lint/ConstantResolution")
        {
            let source = row["source"].as_str().unwrap();
            let found = crate::pipeline::lint_str_with(&active, source);
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            let mut actual: Vec<_> = found
                .iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "{row}");
        }
    }
}
