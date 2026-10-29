//! `Style/ArrayIntersectWithSingleElement` — detection only (unsafe).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ArrayIntersectWithSingleElement;
impl Cop for ArrayIntersectWithSingleElement {
    fn name(&self) -> &'static str {
        "Style/ArrayIntersectWithSingleElement"
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
        // The pinned safe-navigation spec explicitly expects no finding.
        if call.is_safe_navigation() || call.name().as_slice() != b"intersect?" {
            return;
        }
        let args = ast::call_arguments(&call);
        let [arg] = args.as_slice() else { return };
        let Some(array) = arg.as_array_node() else {
            return;
        };
        if array.elements().len() != 1
            || array
                .elements()
                .first()
                .is_some_and(|e| e.as_splat_node().is_some())
        {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let end = ast::source_end(node);
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `include?(element)` instead of `intersect?([element])`.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop array_intersect_with_single_element_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ArrayIntersectWithSingleElement.name())
            .collect()
    }
    #[test]
    fn pinned_single_element_and_percent_array_positions() {
        for arg in ["[element]", "%i[element]"] {
            let s = format!("# header\n  array.intersect?({arg})\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 9, 2));
            assert_eq!(
                o[0].message,
                "Use `include?(element)` instead of `intersect?([element])`."
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn nested_empty_splat_percent_encoding_and_effects_are_detection_only() {
        use crate::cops::testing::correction_offenses;
        assert!(!ArrayIntersectWithSingleElement.safe_fixes());
        for s in [
            "array.intersect?([[]])",
            "array.intersect?([[*foo]])",
            "intersect?([effect()])",
            "array::intersect?([a = effect()])",
            "array.intersect?(%w[a\\ b])",
            "array.intersect?(%I[#{effect}])",
            "array.intersect?(%w[é])",
            "array.intersect?([nil]) { effect }",
            "array.intersect?(\n [false]\n)",
            "def f; array.intersect?([yield]); end",
        ] {
            let o = correction_offenses(&ArrayIntersectWithSingleElement, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none());
            let applied = crate::corrections::apply(s.as_bytes(), &o).unwrap();
            assert_eq!(applied.bytes, s.as_bytes());
        }
        let s = "array.intersect?([a]) {\n effect\n}";
        assert_eq!(offenses(s)[0].end_line, 1);
    }
    #[test]
    fn pinned_and_structural_no_offense_lookalikes() {
        for s in [
            "array.include?(element)",
            "array&.intersect?([element])",
            "array.intersect?([*foo])",
            "array.intersect?([])",
            "array.intersect?([a, b])",
            "array.intersect?(a)",
            "array.intersect?(([a]))",
            "array.intersect?([a], &block)",
            "array.intersect?(&block)",
            "array.intersect?([a], [b])",
            "array.intersect",
            "# array.intersect?([a])\n'intersect?'",
            "def intersect?(a); end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
