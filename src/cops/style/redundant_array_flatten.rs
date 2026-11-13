//! `Style/RedundantArrayFlatten` — detection only; conversion and separator behavior differ.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantArrayFlatten;
impl Cop for RedundantArrayFlatten {
    fn name(&self) -> &'static str {
        "Style/RedundantArrayFlatten"
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
        let Some(flatten) = node.as_call_node() else {
            return;
        };
        if flatten.name().as_slice() != b"flatten"
            || flatten.receiver().is_none()
            || ast::call_arguments(&flatten).len() > 1
            || flatten.block().is_some_and(|b| b.as_block_node().is_some())
        {
            return;
        }
        let Some(join) = cx.parent().and_then(|n| n.as_call_node()) else {
            return;
        };
        let args = ast::call_arguments(&join);
        if join.name().as_slice() != b"join"
            || !join.receiver().is_some_and(|r| {
                r.location().start_offset() == node.location().start_offset()
                    && r.location().end_offset() == node.location().end_offset()
            })
            || !(args.is_empty() || matches!(args.as_slice(), [a] if a.as_nil_node().is_some()))
        {
            return;
        }
        let Some(dot) = flatten.call_operator_loc() else {
            return;
        };
        let (line, col) = file.line_col(dot.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Remove the redundant `flatten`.".into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop redundant_array_flatten_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantArrayFlatten.name())
            .collect()
    }
    #[test]
    fn pinned_diagnostics_and_navigation() {
        for s in [
            "x.flatten.join",
            "x.flatten.join(nil)",
            "x.flatten(depth).join",
            "x&.flatten&.join",
        ] {
            let o = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 4, 2));
            assert_eq!(o[0].message, "Remove the redundant `flatten`.");
        }
    }
    #[test]
    fn pinned_no_offenses_and_block_wrappers() {
        for s in [
            "x.flatten.foo",
            "x.flatten",
            "x.flatten.join(separator)",
            "flatten.join",
            "x.flatten(depth, extra_arg).join",
            "x.flatten.join(separator, extra_arg)",
            "x.flatten { foo }.join",
            "(x.flatten).join",
            "x.flatten.join((nil))",
            "x.flatten.join(&b)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn nested_arrays_depth_splat_nil_and_effects_are_not_corrected() {
        assert!(!RedundantArrayFlatten.safe_fixes());
        for s in [
            "[].flatten.join",
            "[[[1]]].flatten(1).join",
            "[*values].flatten(0).join(nil)",
            "x.flatten(-1).join",
            "x.flatten(*args).join",
            "x.flatten(&b).join",
            "x.flatten(effect()).join",
            "x&.flatten.join",
            "x.flatten&.join(nil)",
            "x.flatten.join(nil) { effect }",
            "%w[a\\ b].flatten.join",
        ] {
            let o = crate::cops::testing::correction_offenses(&RedundantArrayFlatten, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none());
        }
    }
}
