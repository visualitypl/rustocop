//! `Lint/NumberedParameterAssignment` — numeric-looking local assignment names.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NumberedParameterAssignment;
impl Cop for NumberedParameterAssignment {
    fn name(&self) -> &'static str {
        "Lint/NumberedParameterAssignment"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // This is a syntactic lvasgn callback, not binding/liveness analysis.
        if !crate::cops::variables::is_local_assignment(node) {
            return;
        }
        let (name, location) = match node {
            Node::LocalVariableWriteNode { .. } => {
                let n = node.as_local_variable_write_node().unwrap();
                (n.name(), n.location())
            }
            Node::LocalVariableTargetNode { .. } => {
                // Pattern/capture bindings are match_var, not lvasgn. Only
                // actual multiple/for/rescue targets receive this callback.
                let owner = cx
                    .ancestors()
                    .find(|p| !matches!(p, Node::SplatNode { .. } | Node::MultiTargetNode { .. }));
                if !owner.is_some_and(|p| {
                    matches!(
                        p,
                        Node::MultiWriteNode { .. }
                            | Node::ForNode { .. }
                            | Node::RescueNode { .. }
                    )
                }) {
                    return;
                }
                let n = node.as_local_variable_target_node().unwrap();
                (n.name(), n.location())
            }
            Node::LocalVariableAndWriteNode { .. } => {
                let n = node.as_local_variable_and_write_node().unwrap();
                (n.name(), n.name_loc())
            }
            Node::LocalVariableOrWriteNode { .. } => {
                let n = node.as_local_variable_or_write_node().unwrap();
                (n.name(), n.name_loc())
            }
            Node::LocalVariableOperatorWriteNode { .. } => {
                let n = node.as_local_variable_operator_write_node().unwrap();
                (n.name(), n.name_loc())
            }
            _ => return,
        };
        let Some(digits) = name.as_slice().strip_prefix(b"_") else {
            return;
        };
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return;
        }
        // Ruby Integer#to_i is unbounded. Normalize text, never machine-parse.
        let number = std::str::from_utf8(digits).unwrap().trim_start_matches('0');
        let number = if number.is_empty() { "0" } else { number };
        let relation = if number.len() == 1 && number != "0" {
            "is reserved for"
        } else {
            "is similar to"
        };
        let (line, col) = file.reference_line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            message: format!("`_{number}` {relation} numbered parameter; consider another name."),
            line,
            col,
            end_line: file.last_line_of(location),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/numbered_parameter_assignment_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NumberedParameterAssignment")
            .collect()
    }
    #[test]
    fn reports_local_assignments_and_normalizes_unbounded_decimal_names() {
        for (name, number, reserved) in [
            ("_0", "0", false),
            ("_10", "10", false),
            ("_001", "1", true),
            ("_000", "0", false),
            (
                "_999999999999999999999999999999",
                "999999999999999999999999999999",
                false,
            ),
        ] {
            let found = offenses(&format!("\n  {name} = :value\n"));
            assert_eq!(found.len(), 1, "{name}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            let relation = if reserved {
                "is reserved for"
            } else {
                "is similar to"
            };
            assert_eq!(
                found[0].message,
                format!("`_{number}` {relation} numbered parameter; consider another name.")
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn nonassignments_parameters_index_targets_and_nondecimal_lookalikes() {
        for source in [
            "non_numbered_parameter_name = :value",
            "Hash.new { _1[_2] = :value }",
            "_10",
            "def f(_10); _10; end",
            "x { |_10| _10 }",
            "_10x = 1",
            "__10 = 1",
            "_１２ = 1",
            "@_10 = 1",
            "self._10 = 1",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn numbered_parameter_one_through_nine_are_fatal_at_fixed_ruby34() {
        let registry = crate::cops::Registry::new();
        let config = crate::config::ResolvedConfig::default();
        let active = crate::cops::select_active(
            &registry,
            &config,
            &["Lint/NumberedParameterAssignment".into()],
            &[],
        );
        for n in 1..=9 {
            let source = format!("_{n} = :value");
            let path = std::path::Path::new("example.rb");
            let report =
                crate::pipeline::lint_one(path, path, Some(source.as_bytes()), &active, &config);
            assert!(!report.errors.is_empty(), "{source}");
            assert!(report.result.offenses.is_empty());
        }
    }
}
