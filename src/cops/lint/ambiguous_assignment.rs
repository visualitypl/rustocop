//! `Lint/AmbiguousAssignment` — adjacent `=-`, `=+`, `=*` and `=!`.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};

pub struct AmbiguousAssignment;
impl Cop for AmbiguousAssignment {
    fn name(&self) -> &'static str {
        "Lint/AmbiguousAssignment"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((operator, rhs)) = assignment(node) else {
            return;
        };
        let start = operator.end_offset() - 1;
        let end = rhs.location().start_offset() + 1;
        let intended = match file.bytes.get(start..end) {
            Some(b"=-") => "-=",
            Some(b"=+") => "+=",
            Some(b"=*") => "*=",
            Some(b"=!") => "!=",
            _ => return,
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Suspicious assignment detected. Did you mean `{intended}`?"),
            line,
            col,
            end_line: line,
        });
    }
}

// CheckAssignment also visits compound/multiple assignments and setter sends.
// Match the operator's *last* byte, not just a plain equals token.
fn assignment<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Node<'pr>)> {
    macro_rules! write {
        ($cast:ident) => {
            write!($cast, operator_loc)
        };
        ($cast:ident, $operator:ident) => {{
            let n = node.$cast().unwrap();
            Some((n.$operator(), n.value()))
        }};
    }
    match node {
        Node::LocalVariableWriteNode { .. } => write!(as_local_variable_write_node),
        Node::InstanceVariableWriteNode { .. } => write!(as_instance_variable_write_node),
        Node::ClassVariableWriteNode { .. } => write!(as_class_variable_write_node),
        Node::GlobalVariableWriteNode { .. } => write!(as_global_variable_write_node),
        Node::ConstantWriteNode { .. } => write!(as_constant_write_node),
        Node::ConstantPathWriteNode { .. } => write!(as_constant_path_write_node),
        Node::MultiWriteNode { .. } => write!(as_multi_write_node),
        Node::LocalVariableOperatorWriteNode { .. } => {
            write!(as_local_variable_operator_write_node, binary_operator_loc)
        }
        Node::InstanceVariableOperatorWriteNode { .. } => write!(
            as_instance_variable_operator_write_node,
            binary_operator_loc
        ),
        Node::ClassVariableOperatorWriteNode { .. } => {
            write!(as_class_variable_operator_write_node, binary_operator_loc)
        }
        Node::GlobalVariableOperatorWriteNode { .. } => {
            write!(as_global_variable_operator_write_node, binary_operator_loc)
        }
        Node::ConstantOperatorWriteNode { .. } => {
            write!(as_constant_operator_write_node, binary_operator_loc)
        }
        Node::ConstantPathOperatorWriteNode { .. } => {
            write!(as_constant_path_operator_write_node, binary_operator_loc)
        }
        Node::CallOperatorWriteNode { .. } => {
            write!(as_call_operator_write_node, binary_operator_loc)
        }
        Node::IndexOperatorWriteNode { .. } => {
            write!(as_index_operator_write_node, binary_operator_loc)
        }
        Node::LocalVariableAndWriteNode { .. } => write!(as_local_variable_and_write_node),
        Node::InstanceVariableAndWriteNode { .. } => write!(as_instance_variable_and_write_node),
        Node::ClassVariableAndWriteNode { .. } => write!(as_class_variable_and_write_node),
        Node::GlobalVariableAndWriteNode { .. } => write!(as_global_variable_and_write_node),
        Node::ConstantAndWriteNode { .. } => write!(as_constant_and_write_node),
        Node::ConstantPathAndWriteNode { .. } => write!(as_constant_path_and_write_node),
        Node::CallAndWriteNode { .. } => write!(as_call_and_write_node),
        Node::IndexAndWriteNode { .. } => write!(as_index_and_write_node),
        Node::LocalVariableOrWriteNode { .. } => write!(as_local_variable_or_write_node),
        Node::InstanceVariableOrWriteNode { .. } => write!(as_instance_variable_or_write_node),
        Node::ClassVariableOrWriteNode { .. } => write!(as_class_variable_or_write_node),
        Node::GlobalVariableOrWriteNode { .. } => write!(as_global_variable_or_write_node),
        Node::ConstantOrWriteNode { .. } => write!(as_constant_or_write_node),
        Node::ConstantPathOrWriteNode { .. } => write!(as_constant_path_or_write_node),
        Node::CallOrWriteNode { .. } => write!(as_call_or_write_node),
        Node::IndexOrWriteNode { .. } => write!(as_index_or_write_node),
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            Some((
                call.equal_loc()?,
                call.arguments()?.arguments().iter().last()?,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's ambiguous_assignment_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/AmbiguousAssignment")
            .collect()
    }
    #[test]
    fn all_pinned_targets_and_mistakes() {
        for lhs in [
            "x",
            "@x",
            "@@x",
            "$x",
            "X",
            "obj.foo",
            "arr[0]",
            "hash[:key]",
            "self.foo",
            "obj&.foo",
            "h[k]",
        ] {
            for (op, intended) in [("-", "-="), ("+", "+="), ("*", "*="), ("!", "!=")] {
                let source = format!("{lhs} ={op} y\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, lhs.len() as u32 + 2, 1),
                    "{source}"
                );
                assert_eq!(
                    found[0].message,
                    format!("Suspicious assignment detected. Did you mean `{intended}`?")
                );
                assert!(found[0].correction.is_none());
                assert!(offenses(&format!("{lhs} {op}= y\n{lhs} = {op}y\n")).is_empty());
            }
        }
    }
    #[test]
    fn adjacency_not_semantic_guessing() {
        for source in [
            "x = -1",
            "x = +1",
            "x = !y",
            "x = *y",
            "x =(-y)",
            "foo(-y)",
            "x == -y",
            "x != y",
            "x =\n-y",
            "# x =- y\n'x =- y'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "x =-1",
            "x =+1",
            "A::B =!y",
            "a, b =*y",
            "x +=-y",
            "a.b ||=!y",
            "a[0] &&=+y",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
}
