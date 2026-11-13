//! `Lint/RedundantWithObject` — redundant accumulator block parameter shapes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct RedundantWithObject;
impl Cop for RedundantWithObject {
    fn name(&self) -> &'static str {
        "Lint/RedundantWithObject"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let each = call.name().as_slice() == b"each_with_object";
        if (!each && call.name().as_slice() != b"with_object")
            || ast::call_arguments(&call).len() != 1
        {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if !super::redundant_with_index::redundant_parameters(&block, false) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(selector.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: if each {
                "Use `each` instead of `each_with_object`."
            } else {
                "Remove redundant `with_object`."
            }
            .into(),
            // Unsafe upstream correction changes the value and drops memo evaluation.
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_with_object_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantWithObject")
            .collect()
    }
    #[test]
    fn pinned_single_required_numbered_and_it_examples() {
        for (call, col, message) in [
            (
                "ary.each_with_object([])",
                7,
                "Use `each` instead of `each_with_object`.",
            ),
            (
                "ary&.each_with_object([])",
                8,
                "Use `each` instead of `each_with_object`.",
            ),
            (
                "ary.each.with_object([])",
                12,
                "Remove redundant `with_object`.",
            ),
            (
                "each_with_object([])",
                3,
                "Use `each` instead of `each_with_object`.",
            ),
            ("with_object([])", 3, "Remove redundant `with_object`."),
        ] {
            for block in ["{ |v| v }", "{ _1 }", "{ it }", "do |v|\n v\nend"] {
                let source = format!("\n  {call} {block}\n");
                let got = offenses(&source);
                assert_eq!(got.len(), 1, "{source}");
                assert_eq!((got[0].line, got[0].col, got[0].end_line), (2, col, 2));
                assert_eq!(got[0].message, message);
                assert!(got[0].correction.is_none());
            }
        }
    }
    #[test]
    fn ignores_empty_extra_and_non_required_shapes() {
        for source in [
            "ary.each_with_object([]) { |v, o| v }",
            "ary.each_with_object { |v| v }",
            "ary.each_with_object { |v, o| v; o }",
            "ary.each_with_object([]) { _1 << _2 }",
            "ary.each_with_object([]) { nil }",
            "ary.each_with_object([]) { || nil }",
            "ary.each_with_object([], 1) { |v| v }",
            "ary.each_with_object([]) { |(v)| v }",
            "ary.each_with_object([]) { |v = 1| v }",
            "ary.each_with_object([]) { |*v| v }",
            "ary.each_with_object([]) { |v; x| v }",
            "ary.each_with_object([], &blk)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn highlights_arguments_not_the_block() {
        let got = offenses("ary.each_with_object(\n []\n) do |v|\n v\nend\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].line, got[0].col, got[0].end_line), (1, 5, 3));
    }
}
