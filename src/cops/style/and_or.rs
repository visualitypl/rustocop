//! `Style/AndOr` — keyword operators in conditions only; unsafe, detection-only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AndOr;
impl Cop for AndOr {
    fn name(&self) -> &'static str {
        "Style/AndOr"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let op = if let Some(n) = node.as_and_node() {
            n.operator_loc()
        } else if let Some(n) = node.as_or_node() {
            n.operator_loc()
        } else {
            return;
        };
        let (current, prefer) = match op.as_slice() {
            b"and" => ("and", "&&"),
            b"or" => ("or", "||"),
            _ => return,
        };
        let start = node.location().start_offset();
        let end = node.location().end_offset();
        let in_condition = cx
            .ancestors()
            .zip(cx.ancestors().skip(1))
            .any(|(ancestor, parent)| {
                // Prism models pattern guards as If/Unless, unlike RuboCop's
                // if_guard/unless_guard. Those guards are not on_if callbacks.
                if parent.as_in_node().is_some() {
                    return false;
                }
                let predicate = if let Some(n) = ancestor.as_if_node() {
                    Some(n.predicate())
                } else if let Some(n) = ancestor.as_unless_node() {
                    Some(n.predicate())
                } else if let Some(n) = ancestor.as_while_node() {
                    Some(n.predicate())
                } else {
                    ancestor.as_until_node().map(|n| n.predicate())
                };
                predicate.is_some_and(|n| {
                    n.location().start_offset() <= start && end <= n.location().end_offset()
                })
            });
        if !in_condition {
            return;
        }
        let (line, col) = file.line_col(op.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(op.end_offset() - 1),
            message: format!("Use `{prefer}` instead of `{current}`."),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported/adapted RuboCop style/and_or_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn only_conditional_subtrees_with_exact_operator_locations() {
        for (s, line, col) in [
            ("if a and b; end", 1, 6),
            ("work unless a or b", 1, 15),
            ("begin; work; end while a and b", 1, 26),
            ("if foo { a and b }; end", 1, 12),
            ("(a and b) ? x : y", 1, 4),
        ] {
            let found = correction_offenses(&AndOr, s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{s}");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn bodies_flow_control_and_pattern_guards_are_not_if_conditions() {
        for s in [
            "foo and return",
            "x = a or b",
            "if a; b and c; end",
            "case x; in y if (a or b); end",
            "case x; in y unless (a and b); end",
            "work if a && b",
        ] {
            assert!(correction_offenses(&AndOr, s).is_empty(), "{s}");
        }
    }
}
