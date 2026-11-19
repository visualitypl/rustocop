//! `Style/RedundantSelfAssignment` — receiver-returning method heuristic; unsafe correction.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct RedundantSelfAssignment;
const METHODS: &[&[u8]] = &[
    b"append",
    b"clear",
    b"collect!",
    b"compare_by_identity",
    b"concat",
    b"delete_if",
    b"fill",
    b"initialize_copy",
    b"insert",
    b"keep_if",
    b"map!",
    b"merge!",
    b"prepend",
    b"push",
    b"rehash",
    b"replace",
    b"reverse!",
    b"rotate!",
    b"shuffle!",
    b"sort!",
    b"sort_by!",
    b"transform_keys!",
    b"transform_values!",
    b"unshift",
    b"update",
];
impl Cop for RedundantSelfAssignment {
    fn name(&self) -> &'static str {
        "Style/RedundantSelfAssignment"
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
        let Some((rhs, operator, variable)) = assignment(node) else {
            return;
        };
        let Some(call) = rhs.as_call_node() else {
            return;
        };
        if !METHODS.contains(&call.name().as_slice()) {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let same = if let Some((kind, name, depth)) = variable {
            read(&receiver).is_some_and(|r| r == (kind, name, depth))
        } else {
            // RuboCop's setter matcher accepts a send, not a literal block.
            if call.block().is_some_and(|n| n.as_block_node().is_some()) {
                return;
            }
            let lhs = node.as_call_node().unwrap();
            receiver.as_call_node().is_some_and(|get| {
                ast::call_arguments(&get).is_empty()
                    && get.block().is_none()
                    && get.name().as_slice()
                        == lhs.name().as_slice().strip_suffix(b"=").unwrap_or_default()
                    && ast::same_optional_node(lhs.receiver().as_ref(), get.receiver().as_ref())
            })
        };
        if !same {
            return;
        }
        // A variable assignment becomes its value; a setter loses everything
        // before its argument.
        let correction = cx.corrections_requested().then(|| {
            let loc = node.location();
            if variable.is_some() {
                crate::offense::Correction {
                    range: loc.start_offset()..loc.end_offset(),
                    replacement: String::from_utf8_lossy(rhs.location().as_slice()).into_owned(),
                }
            } else {
                crate::offense::Correction {
                    range: loc.start_offset()..rhs.location().start_offset(),
                    replacement: String::new(),
                }
            }
        });
        let (line, col) = file.line_col(operator.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            correction,
            message: format!(
                "Redundant self assignment detected. Method `{}` modifies its receiver in place.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
        });
    }
}
type Variable<'a> = (u8, &'a [u8], u32);
fn assignment<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Location<'pr>, Option<Variable<'pr>>)> {
    macro_rules! var {
        ($as:ident,$kind:expr,$depth:expr) => {{
            let n = node.$as()?;
            Some((
                n.value(),
                n.operator_loc(),
                Some(($kind, n.name().as_slice(), $depth)),
            ))
        }};
    }
    match node {
        Node::LocalVariableWriteNode { .. } => var!(
            as_local_variable_write_node,
            0,
            node.as_local_variable_write_node()?.depth()
        ),
        Node::InstanceVariableWriteNode { .. } => var!(as_instance_variable_write_node, 1, 0),
        Node::ClassVariableWriteNode { .. } => var!(as_class_variable_write_node, 2, 0),
        Node::GlobalVariableWriteNode { .. } => var!(as_global_variable_write_node, 3, 0),
        Node::CallNode { .. } => {
            let n = node.as_call_node()?;
            if !n.is_attribute_write() {
                return None;
            }
            let mut args = ast::call_arguments(&n);
            if args.len() != 1 {
                return None;
            }
            Some((args.pop()?, n.equal_loc()?, None))
        }
        _ => None,
    }
}
fn read<'pr>(node: &Node<'pr>) -> Option<Variable<'pr>> {
    Some(match node {
        Node::LocalVariableReadNode { .. } => {
            let n = node.as_local_variable_read_node()?;
            (0, n.name().as_slice(), n.depth())
        }
        Node::InstanceVariableReadNode { .. } => (
            1,
            node.as_instance_variable_read_node()?.name().as_slice(),
            0,
        ),
        Node::ClassVariableReadNode { .. } => {
            (2, node.as_class_variable_read_node()?.name().as_slice(), 0)
        }
        Node::GlobalVariableReadNode { .. } => {
            (3, node.as_global_variable_read_node()?.name().as_slice(), 0)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_self_assignment_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    #[test]
    fn every_mutator_and_variable_kind_is_corrected() {
        for method in super::METHODS {
            let method = std::str::from_utf8(method).unwrap();
            for variable in ["foo", "@foo", "@@foo", "$foo"] {
                let source = format!("  {variable} = {variable}&.{method}(arg) {{ work }}\n");
                let found = crate::cops::testing::correction_offenses(
                    &super::RedundantSelfAssignment,
                    &source,
                );
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, variable.len() as u32 + 4)
                );
                assert_eq!(
                    found[0].message,
                    format!(
                        "Redundant self assignment detected. Method `{method}` modifies its receiver in place."
                    )
                );
                assert!(found[0].correction.is_some());
            }
        }
    }
    #[test]
    fn setters_blocks_capture_and_lookalikes() {
        for source in [
            "foo = nil; proc { foo = foo.concat(ary) }",
            "foo = nil; proc { |foo| foo = foo.concat(ary) }",
            "foo = nil; proc { |;foo| foo = foo.concat(ary) }",
            "self.foo = self.foo.concat(ary)",
            "make.foo = make.foo.concat(ary)",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&super::RedundantSelfAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_some());
        }
        for source in [
            "C = C.concat(ary)",
            "Namespace::C = Namespace::C.concat(ary)",
            "foo = foo.strip!",
            "foo = foo.compact!",
            "foo = foo.sort",
            "foo = other.concat(ary)",
            "@foo = foo.concat(ary)",
            "foo ||= foo.concat(ary)",
            "foo += foo.concat(ary)",
            "foo,bar = foo.concat(ary),nil",
            "self.foo = foo.concat(ary)",
            "other.foo = other.foo(1).concat(ary)",
            "other.foo = other.foo { 1 }.concat(ary)",
            "obj.x = obj.x.delete_if { true }",
            "self.x = self.x.delete_if { true }",
            "other.foo = other.bar.concat(ary)",
            "object[0] = object[0].concat(ary)",
            "foo = foo.concat(ary).size",
        ] {
            assert!(
                !crate::pipeline::lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Style/RedundantSelfAssignment"),
                "{source}"
            );
        }
    }
    #[test]
    fn every_pinned_example_has_exact_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_self_assignment_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 14);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/RedundantSelfAssignment")
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "{source}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{source}"
            );
        }
    }
}
