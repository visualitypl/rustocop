//! `Lint/MultipleComparison` — comparison on another comparison's result.
//! Detection only: the upstream conjunction rewrite changes evaluation/value
//! and exception semantics (an explicit no-source-edit exception).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

pub struct MultipleComparison;
impl Cop for MultipleComparison {
    fn name(&self) -> &'static str {
        "Lint/MultipleComparison"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(outer) = node.as_call_node().filter(comparison) else {
            return;
        };
        let Some(inner) = outer
            .receiver()
            .and_then(|n| n.as_call_node())
            .filter(|c| comparison(c) && !literal_block(c))
        else {
            return;
        };
        let args = ast::call_arguments(&inner);
        let [center] = args.as_slice() else { return };
        if center.as_call_node().is_some_and(|c| {
            !c.is_safe_navigation()
                && !literal_block(&c)
                && matches!(c.name().as_slice(), b"&" | b"|" | b"^")
        }) {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use the `&&` operator to compare multiple values.".into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            correction: None,
        });
    }
}
fn literal_block(call: &CallNode<'_>) -> bool {
    call.block().is_some_and(|b| b.as_block_node().is_some())
}
fn comparison(call: &CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && matches!(call.name().as_slice(), b"<" | b">" | b"<=" | b">=")
        && ast::call_arguments(call).len() == 1
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/multiple_comparison_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/MultipleComparison")
            .collect()
    }
    #[test]
    fn all_sixteen_operator_permutations() {
        for first in ["<", ">", "<=", ">="] {
            for second in ["<", ">", "<=", ">="] {
                let source = format!("# comparison\n  x {first} y {second} z\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                let o = &found[0];
                assert_eq!((o.line, o.col, o.end_line), (2, 3, 2));
                assert_eq!(
                    o.message,
                    "Use the `&&` operator to compare multiple values."
                );
                assert!(o.correction.is_none());
            }
        }
    }
    #[test]
    fn exact_send_shapes_and_chain_multiplicity() {
        let found = offenses("x < y < z < last\n");
        assert_eq!(found.len(), 2);
        for source in [
            "(x < y) < z",
            "x.<(y) {}.<(z)",
            "x&.<(y).<(z)",
            "x.<(y)&.<(z)",
            "x.<(y, &b).<(z)",
            "x.<(y).<(z, &b)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("x.<(y).<(z) { body }").len(), 1);
        // The center exemption is by send selector alone, not receiver/arity.
        assert!(offenses("x.<(y.&).<(z)").is_empty());
        assert!(offenses("x.<(y.&(a, b)).<(z)").is_empty());
        assert_eq!(offenses("x.<(y&.&(a)).<(z)").len(), 1);
        assert_eq!(offenses("x.<(y.&(a) {}).<(z)").len(), 1);
    }
    #[test]
    fn literal_block_does_not_extend_the_send_offense_range() {
        let found = offenses("x.<(y).<(z) do\n body\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].end_line), (1, 1));
    }
    #[test]
    fn one_comparison_and_set_operation_centers_are_accepted() {
        for source in [
            "x < 1",
            "x >= y & x < z",
            "x >= y | x < z",
            "x >= y ^ x < z",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
