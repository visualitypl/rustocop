//! `Style/UnlessLogicalOperators` — mixed logical operators in unless conditions.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct UnlessLogicalOperators;
impl Cop for UnlessLogicalOperators {
    fn name(&self) -> &'static str {
        "Style/UnlessLogicalOperators"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_unless_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(unless) = node.as_unless_node() else {
            return;
        };
        let condition = unless.predicate();
        let mut operators = Operators::default();
        crate::cops::walk::walk(&mut operators, &condition);
        let mixed_types = (condition.as_and_node().is_some() && operators.or != 0)
            || (condition.as_or_node().is_some() && operators.and != 0);
        if !mixed_types && operators.and != 3 && operators.or != 3 {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: "Do not use mixed logical operators in an `unless`.".into(),
            correction: None,
        });
    }
}

// One traversal of the predicate, never the unless body. RuboCop checks mixed
// AND/OR kinds only beneath a logical root, but mixed spellings at any depth.
#[derive(Default)]
struct Operators {
    and: u8,
    or: u8,
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Operators {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(n) = node.as_and_node() {
            self.and |= if n.operator_loc().as_slice() == b"&&" {
                1
            } else {
                2
            };
        } else if let Some(n) = node.as_or_node() {
            self.or |= if n.operator_loc().as_slice() == b"||" {
                1
            } else {
                2
            };
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/unless_logical_operators_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        let registry = crate::cops::Registry::new();
        let config = crate::config::ResolvedConfig::default();
        let active = crate::cops::select_active(
            &registry,
            &config,
            &["Style/UnlessLogicalOperators".into()],
            &[],
        );
        let result = crate::pipeline::lint_one(
            std::path::Path::new("a.rb"),
            std::path::Path::new("a.rb"),
            Some(source.as_bytes()),
            &active,
            &config,
        );
        assert!(
            result.errors.is_empty() && result.internal_errors.is_empty(),
            "{result:?}"
        );
        result.result.offenses
    }
    #[test]
    fn flags_all_mixed_operator_spellings() {
        for condition in [
            "a && b || c",
            "a || b && c",
            "a && b and c",
            "a and b && c",
            "a && b or c",
            "a or b && c",
            "a || b or c",
            "a or b || c",
            "a || b and c",
            "a and b || c",
            "a || (b && c) || d",
        ] {
            let s = format!("# condition\n  return unless {condition}\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, 3));
            assert_eq!(
                o[0].message,
                "Do not use mixed logical operators in an `unless`."
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn respects_condition_root_grouping_and_never_checks_the_body() {
        for s in [
            "work unless (a || b && c)",
            "work unless test(a || b && c)",
            "work unless (assigned = a || b && c)",
            "unless test; a || b && c; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        // Mixed spellings are checked at any predicate depth, even when mixed
        // AND/OR kinds alone would be accepted under a grouped/call root.
        for s in [
            "work unless (a && b and c)",
            "work unless test((a && b and c))",
            "unless a || test(b && c); work; end",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }

    #[test]
    fn accepts_homogeneous_conditions_and_other_contexts() {
        for s in [
            "return unless a && b && c",
            "return unless a || b || c",
            "return unless a and b and c",
            "return unless a or b or c",
            "return unless a?",
            "return if a || b && c || d",
            "def condition?; a or b && c || d; end",
            "unless condition; includes_or_in_the_name; foo || bar; end",
            "unless condition; includes_and_in_the_name; foo && bar; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
