//! `Style/OrAssignment` — replace redundant variable initialization with `||=`.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct OrAssignment;
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
impl Cop for OrAssignment {
    fn name(&self) -> &'static str {
        "Style/OrAssignment"
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
        let Some((variable, default, _)) = candidate(node) else {
            return;
        };
        let correction = cx
            .corrections_requested()
            .then(|| correction(node, variable, &default))
            .flatten();
        super::negated_if::report(
            self,
            node,
            file,
            "Use the double pipe equals operator `||=` instead.".into(),
            correction,
            out,
        );
    }
}

// Matching read/write kind, name and lexical depth is sufficient here: no
// declarations cross one another. In particular a captured local's identity
// must not be confused with a block-owned local with the same spelling.
type Variable<'pr> = (Kind, &'pr [u8], u32);
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Local,
    Instance,
    Class,
    Global,
}

fn correction(node: &Node<'_>, variable: Variable<'_>, default: &Node<'_>) -> Option<Correction> {
    // RuboCop writes `name ||= default` with the default's source.
    let loc = node.location();
    let name = std::str::from_utf8(variable.1).ok()?;
    let default = std::str::from_utf8(default.location().as_slice()).ok()?;
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!("{name} ||= {default}"),
    })
}
fn read<'pr>(node: &Node<'pr>) -> Option<Variable<'pr>> {
    Some(match node {
        Node::LocalVariableReadNode { .. } => {
            let n = node.as_local_variable_read_node()?;
            (Kind::Local, n.name().as_slice(), n.depth())
        }
        Node::InstanceVariableReadNode { .. } => (
            Kind::Instance,
            node.as_instance_variable_read_node()?.name().as_slice(),
            0,
        ),
        Node::ClassVariableReadNode { .. } => (
            Kind::Class,
            node.as_class_variable_read_node()?.name().as_slice(),
            0,
        ),
        Node::GlobalVariableReadNode { .. } => (
            Kind::Global,
            node.as_global_variable_read_node()?.name().as_slice(),
            0,
        ),
        _ => return None,
    })
}
fn write<'pr>(node: &Node<'pr>) -> Option<(Variable<'pr>, Node<'pr>)> {
    Some(match node {
        Node::LocalVariableWriteNode { .. } => {
            let n = node.as_local_variable_write_node()?;
            ((Kind::Local, n.name().as_slice(), n.depth()), n.value())
        }
        Node::InstanceVariableWriteNode { .. } => {
            let n = node.as_instance_variable_write_node()?;
            ((Kind::Instance, n.name().as_slice(), 0), n.value())
        }
        Node::ClassVariableWriteNode { .. } => {
            let n = node.as_class_variable_write_node()?;
            ((Kind::Class, n.name().as_slice(), 0), n.value())
        }
        Node::GlobalVariableWriteNode { .. } => {
            let n = node.as_global_variable_write_node()?;
            ((Kind::Global, n.name().as_slice(), 0), n.value())
        }
        _ => return None,
    })
}
fn only<'pr>(body: Option<ruby_prism::StatementsNode<'pr>>) -> Option<Node<'pr>> {
    let body = body?;
    (body.body().len() == 1).then(|| body.body().first().unwrap())
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<(Variable<'pr>, Node<'pr>, bool)> {
    if let Some((variable, value)) = write(node) {
        let (predicate, truthy, default) = if let Some(condition) = value.as_if_node() {
            (
                condition.predicate(),
                only(condition.statements())?,
                only(condition.subsequent()?.as_else_node()?.statements())?,
            )
        } else {
            let condition = value.as_unless_node()?;
            (
                condition.predicate(),
                only(condition.else_clause()?.statements())?,
                only(condition.statements())?,
            )
        };
        if default.as_if_node().is_some() || default.as_unless_node().is_some() {
            return None;
        }
        return (read(&predicate)? == variable && read(&truthy)? == variable)
            .then_some((variable, default, false));
    }
    let (predicate, assignment) = if let Some(n) = node.as_unless_node() {
        if n.else_clause().is_some_and(|e| e.statements().is_some()) {
            return None;
        }
        (n.predicate(), only(n.statements())?)
    } else {
        let n = node.as_if_node()?;
        if n.statements().is_some() {
            return None;
        }
        (
            n.predicate(),
            only(n.subsequent()?.as_else_node()?.statements())?,
        )
    };
    let (variable, default) = write(&assignment)?;
    (read(&predicate)? == variable).then_some((variable, default, true))
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/or_assignment_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn every_pinned_example_has_exact_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/or_assignment_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 25);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == OrAssignment.name())
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
    #[test]
    fn reversed_unless_expression_is_normalized_like_upstream() {
        assert_correction(
            &OrAssignment,
            "foo = unless foo\n 'default'\nelse\n foo\nend",
            "foo ||= 'default'",
        );
    }
    #[test]
    fn lookalikes_do_not_match() {
        for source in [
            "foo = bar ? foo : 3",
            "foo = foo ? bar : 3",
            "foo = 3 unless bar",
            "unless foo\n bar = 3\nend",
            "foo = if foo\n foo\nelsif bar\n 2\nelse\n 3\nend",
            "CONST = CONST ? CONST : 3",
            "obj.x = obj.x ? obj.x : 3",
            "obj[0] = obj[0] ? obj[0] : 3",
            "a,b = a ? a : 3",
            "foo = foo ? foo : bar ? bar : 3",
            "unless @foo\n @foo = 1\n work\nend",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == OrAssignment.name()),
                "{source}"
            );
        }
    }
}
