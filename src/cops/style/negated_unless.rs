//! `Style/NegatedUnless` — fixed `both` prefix/modifier policy.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NegatedUnless;
impl Cop for NegatedUnless {
    fn name(&self) -> &'static str {
        "Style/NegatedUnless"
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
        let Some(n) = node.as_unless_node() else {
            return;
        };
        if n.else_clause().is_some() {
            return;
        }
        super::negated_if::check_single(
            self,
            node,
            n.keyword_loc(),
            n.predicate(),
            "if",
            cx,
            file,
            out,
        );
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop negated_unless_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    #[test]
    fn negative_prefix_and_modifier_locations_and_messages() {
        for op in ["!", "not "] {
            let s = format!(
                "unless {op}a_condition\n  some_method\nend\nsome_method unless {op}a_condition\n"
            );
            let o: Vec<_> = crate::pipeline::lint_str(&s)
                .into_iter()
                .filter(|o| o.cop == "Style/NegatedUnless")
                .collect();
            assert_eq!(o.len(), 2);
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 1, 3));
            assert_eq!((o[1].line, o[1].col), (4, 1));
            assert_eq!(
                o[0].message,
                "Favor `if` over `unless` for negative conditions."
            );
        }
    }
    #[test]
    fn canonical_closed_immediate_corrections() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("unless !false\n  work\nend\n", "if false\n  work\nend\n"),
            (":work unless not true\n", ":work if true\n"),
            (":work unless (!nil)\n", ":work if (nil)\n"),
            ("value = 1 unless !false\n", "value = 1 if false\n"),
        ] {
            assert_correction(&super::NegatedUnless, before, after);
        }
    }
    #[test]
    fn ignores_else_double_partial_and_empty_conditions() {
        for s in [
            "unless !x\n a\nelse\n b\nend",
            "unless !!x; a; end",
            "a unless not x or y",
            "unless !x && y; a; end",
            "unless (); end",
            "!x ? a : b",
            "if !x; a; end",
        ] {
            assert!(
                crate::pipeline::lint_str(s)
                    .iter()
                    .all(|o| o.cop != "Style/NegatedUnless"),
                "{s}"
            );
        }
    }
}
