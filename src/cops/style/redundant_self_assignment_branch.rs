//! `Style/RedundantSelfAssignmentBranch` — a local conditional's redundant self-read arm.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};
pub struct RedundantSelfAssignmentBranch;
impl Cop for RedundantSelfAssignmentBranch {
    fn name(&self) -> &'static str {
        "Style/RedundantSelfAssignmentBranch"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_local_variable_write_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(write) = node.as_local_variable_write_node() else {
            return;
        };
        let value = write.value();
        let Some((predicate, truthy, falsy)) = branches(&value) else {
            return;
        };
        if !statement(cx) {
            return;
        }
        let name = write.name();
        let (branch, opposite, keyword) = if truthy
            .as_ref()
            .is_some_and(|n| n.location().as_slice() == name.as_slice())
        {
            (truthy, falsy, "unless")
        } else if falsy
            .as_ref()
            .is_some_and(|n| n.location().as_slice() == name.as_slice())
        {
            (falsy, truthy, "if")
        } else {
            return;
        };
        let correction = cx
            .corrections_requested()
            .then(|| correction(&value, &predicate, opposite.as_ref(), keyword, file))
            .flatten();
        super::negated_if::report(
            self,
            &branch.unwrap(),
            file,
            "Remove the self-assignment branch.".into(),
            correction,
            out,
        );
    }
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
/// RuboCop's corrector: the conditional becomes `opposite keyword
/// condition` (`nil` without an opposite branch), keeping a heredoc body.
fn correction(
    value: &Node<'_>,
    predicate: &Node<'_>,
    opposite: Option<&Node<'_>>,
    keyword: &str,
    file: &SourceFile,
) -> Option<Correction> {
    let text = |n: &Node<'_>| {
        std::str::from_utf8(n.location().as_slice())
            .ok()
            .map(str::to_owned)
    };
    let rhs = match opposite {
        Some(n) => text(n)?,
        None => "nil".into(),
    };
    let mut replacement = format!("{rhs} {keyword} {}", text(predicate)?);
    if let Some(string) = opposite.and_then(|n| n.as_string_node())
        && string
            .opening_loc()
            .is_some_and(|o| o.as_slice().starts_with(b"<<"))
    {
        let closing = string.closing_loc()?;
        let tail =
            std::str::from_utf8(&file.bytes[string.location().end_offset()..closing.end_offset()])
                .ok()?;
        replacement.push_str(
            tail.strip_suffix("\r\n")
                .or_else(|| tail.strip_suffix('\n'))
                .unwrap_or(tail),
        );
    }
    Some(Correction {
        range: value.location().start_offset()..value.location().end_offset(),
        replacement,
    })
}
fn arm<'pr>(body: Option<StatementsNode<'pr>>) -> Option<Option<Node<'pr>>> {
    match body {
        None => Some(None),
        Some(body) => match body.body().len() {
            0 => Some(None),
            1 => {
                let branch = body.body().first()?;
                if branch
                    .as_parentheses_node()
                    .is_some_and(|n| n.body().is_some())
                {
                    return None;
                }
                Some(Some(branch))
            }
            _ => None,
        },
    }
}
fn branches<'pr>(value: &Node<'pr>) -> Option<(Node<'pr>, Option<Node<'pr>>, Option<Node<'pr>>)> {
    if let Some(n) = value.as_if_node() {
        let other = match n.subsequent() {
            None => None,
            Some(n) => Some(n.as_else_node()?),
        };
        Some((
            n.predicate(),
            arm(n.statements())?,
            arm(other.and_then(|n| n.statements()))?,
        ))
    } else {
        let n = value.as_unless_node()?;
        Some((
            n.predicate(),
            arm(n.else_clause().and_then(|n| n.statements()))?,
            arm(n.statements())?,
        ))
    }
}
fn statement(cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors();
    let Some(parent) = ancestors.next() else {
        return true;
    };
    if parent.as_statements_node().is_none() {
        return false;
    }
    let Some(parent) = ancestors.next() else {
        return true;
    };
    match parent {
        Node::IfNode { .. } => parent.as_if_node().unwrap().if_keyword_loc().is_some(),
        Node::UnlessNode { .. } => true,
        Node::ParenthesesNode { .. } => false,
        Node::ProgramNode { .. }
        | Node::DefNode { .. }
        | Node::BlockNode { .. }
        | Node::LambdaNode { .. }
        | Node::ClassNode { .. }
        | Node::ModuleNode { .. }
        | Node::SingletonClassNode { .. }
        | Node::BeginNode { .. }
        | Node::RescueNode { .. }
        | Node::EnsureNode { .. }
        | Node::WhenNode { .. }
        | Node::InNode { .. }
        | Node::ElseNode { .. }
        | Node::WhileNode { .. }
        | Node::UntilNode { .. }
        | Node::ForNode { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_self_assignment_branch_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    #[test]
    fn nonempty_parentheses_are_not_convertible_branches() {
        for source in [
            "foo = c ? (bar) : foo",
            "foo = c ? foo : (bar)",
            "foo = c ? foo : (bar; baz)",
        ] {
            assert!(
                !crate::pipeline::lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Style/RedundantSelfAssignmentBranch"),
                "{source}"
            );
        }
    }
    #[test]
    fn every_pinned_example_has_exact_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_self_assignment_branch_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 34);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/RedundantSelfAssignmentBranch")
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
