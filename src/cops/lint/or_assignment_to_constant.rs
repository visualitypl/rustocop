//! `Lint/OrAssignmentToConstant` — conditional constant initialization (no fixes).

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct OrAssignmentToConstant;

impl Cop for OrAssignmentToConstant {
    fn name(&self) -> &'static str {
        "Lint/OrAssignmentToConstant"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let operator = match node {
            Node::ConstantOrWriteNode { .. } => {
                node.as_constant_or_write_node().unwrap().operator_loc()
            }
            Node::ConstantPathOrWriteNode { .. } => node
                .as_constant_path_or_write_node()
                .unwrap()
                .operator_loc(),
            _ => return,
        };
        let (line, col) = file.line_col(operator.start_offset());
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: operator.start_offset()..operator.end_offset(),
                    replacement: "=".into(),
                }),
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid using or-assignment with constants.".into(),
            line,
            col,
            end_line: line,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's or_assignment_to_constant_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::pipeline::lint_str;

    #[test]
    fn constant_or_assignment_reports_only_the_operator_even_inside_methods() {
        for (source, line, col) in [
            ("CONST ||= 1", 1, 7),
            ("def foo\n  M::CONST ||= 1\nend", 2, 12),
            ("def self.foo\n  M::CONST ||= 1\nend", 2, 12),
            ("define_method :foo do\n  M::CONST ||= 1\nend", 2, 12),
            ("::CONST ||= 1", 1, 9),
            ("factory::CONST ||=\n  1", 1, 16),
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Lint/OrAssignmentToConstant")
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                "Avoid using or-assignment with constants."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn other_assignment_shapes_are_not_constants() {
        for source in [
            "CONST = 1",
            "CONST &&= 1",
            "CONST += 1",
            "var ||= 1",
            "@var ||= 1",
            "@@var ||= 1",
            "$var ||= 1",
            "self.var ||= 1",
            "obj[CONST] ||= 1",
            "obj.CONST ||= 1",
            "'# CONST ||= 1'",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Lint/OrAssignmentToConstant"),
                "{source}"
            );
        }
    }
}
