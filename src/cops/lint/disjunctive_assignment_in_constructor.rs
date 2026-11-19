//! `Lint/DisjunctiveAssignmentInConstructor` — leading instance ||= in initialize.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};

pub struct DisjunctiveAssignmentInConstructor;

impl Cop for DisjunctiveAssignmentInConstructor {
    fn name(&self) -> &'static str {
        "Lint/DisjunctiveAssignmentInConstructor"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        if def.receiver().is_some() || def.name().as_slice() != b"initialize" {
            return;
        }
        let Some(body) = def.body().and_then(|body| body.as_statements_node()) else {
            return;
        };
        // RuboCop's body is a begin only for multiple statements or explicit
        // parentheses. Unwrap exactly that outer begin, never recursively:
        // a parenthesized statement in a sequence is itself a stopping barrier.
        let statements = if body.body().len() == 1 {
            let first = body.body().iter().next().unwrap();
            if let Some(parens) = first.as_parentheses_node() {
                parens.body().and_then(|n| n.as_statements_node())
            } else {
                Some(body)
            }
        } else {
            Some(body)
        };
        if let Some(statements) = statements {
            self.check_lines(statements, file, out);
        }
    }
}

impl DisjunctiveAssignmentInConstructor {
    fn check_lines(
        &self,
        statements: StatementsNode<'_>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        for statement in statements.body().iter() {
            if let Some(write) = statement.as_instance_variable_or_write_node() {
                let (line, col) = file.line_col(write.operator_loc().start_offset());
                out.push(Offense {
                    correction: Some(crate::offense::Correction {
                        range: write.operator_loc().start_offset()
                            ..write.operator_loc().end_offset(),
                        replacement: "=".into(),
                    }),
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Unnecessary disjunctive assignment. Use plain assignment.".into(),
                    line,
                    col,
                    end_line: line,
                });
            } else if !matches!(
                statement,
                Node::LocalVariableOrWriteNode { .. }
                    | Node::ClassVariableOrWriteNode { .. }
                    | Node::GlobalVariableOrWriteNode { .. }
                    | Node::ConstantOrWriteNode { .. }
                    | Node::ConstantPathOrWriteNode { .. }
                    | Node::CallOrWriteNode { .. }
                    | Node::IndexOrWriteNode { .. }
            ) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's disjunctive_assignment_in_constructor_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::pipeline::lint_str;

    #[test]
    fn leading_instance_or_assignments_report_operators_without_edits() {
        for tail in ["", "\n    super"] {
            let source = format!(
                "class Banana\n  def initialize\n    @delicious ||= true{tail}\n  end\nend"
            );
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Lint/DisjunctiveAssignmentInConstructor")
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 16, 3));
            assert_eq!(
                found[0].message,
                "Unnecessary disjunctive assignment. Use plain assignment."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn empty_plain_local_and_prior_effects_are_accepted() {
        for body in [
            "",
            "@delicious = true",
            "delicious ||= true",
            "super\n@delicious ||= true",
            "absolutely_any_method\n@delicious ||= true",
        ] {
            let source = format!("class Banana\ndef initialize\n{body}\nend\nend");
            assert!(
                !lint_str(&source)
                    .iter()
                    .any(|o| o.cop == "Lint/DisjunctiveAssignmentInConstructor"),
                "{source}"
            );
        }
    }
}
