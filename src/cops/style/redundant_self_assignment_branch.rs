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
            .then(|| {
                correction(
                    node,
                    &value,
                    &predicate,
                    opposite.as_ref(),
                    keyword,
                    cx,
                    file,
                )
            })
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
fn correction(
    node: &Node<'_>,
    value: &Node<'_>,
    predicate: &Node<'_>,
    opposite: Option<&Node<'_>>,
    keyword: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    if loc.end_offset() - loc.start_offset() > 16 * 1024
        || !discarded(node, cx)
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let write = node.as_local_variable_write_node()?;
    if file
        .redundant_assignment_safety(root)
        .dynamic_writes
        .contains(&loc.start_offset())
        || file.heredoc_pending_at(root, loc.start_offset())
    {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + loc.end_offset() - loc.start_offset()));
    let declared = loc.start_offset() > 0
        && file.visible_locals(root).declared_before(
            write.name().as_slice(),
            node,
            cx,
            loc.start_offset().saturating_sub(1),
        );
    if !declared && predicate_hazard(predicate, write.name().as_slice()) {
        return None;
    }
    let mut suffix = "";
    if file.contains_heredoc(root, node) {
        // Only a single direct literal whose body is enclosed by the old if.
        // Keep the body and terminator byte-for-byte (including squiggly indent).
        let string = opposite?.as_string_node()?;
        let opening = string.opening_loc()?;
        let closing = string.closing_loc()?;
        if !opening.as_slice().starts_with(b"<<")
            || closing.end_offset() > loc.end_offset()
            || file.contains_heredoc(root, predicate)
        {
            return None;
        }
        if file.bytes[loc.end_offset()..file.line_start(file.line_of(loc.end_offset()) + 1)]
            .iter()
            .any(|b| !b.is_ascii_whitespace())
        {
            return None;
        }
        let body =
            std::str::from_utf8(&file.bytes[string.location().end_offset()..closing.end_offset()])
                .ok()?;
        suffix = body
            .strip_suffix("\r\n")
            .or_else(|| body.strip_suffix('\n'))
            .unwrap_or(body);
    }
    if opposite.is_some_and(contains_jump) {
        return None;
    }
    let rhs = match opposite {
        Some(n) => envelope(n)?,
        None => "nil".into(),
    };
    let predicate = envelope(predicate)?;
    let prefix =
        std::str::from_utf8(&file.bytes[loc.start_offset()..value.location().start_offset()])
            .ok()?;
    super::negated_if::finish(
        Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: format!("{prefix}{rhs} {keyword} {predicate}{suffix}"),
        },
        cx,
        file,
    )
}
fn envelope(node: &Node<'_>) -> Option<String> {
    let text = std::str::from_utf8(node.location().as_slice()).ok()?;
    Some(
        if crate::cops::ast::is_basic_literal(node)
            || node
                .as_call_node()
                .is_some_and(|n| !n.message_loc().is_some_and(|m| m.as_slice() == b"not"))
            || node.as_parentheses_node().is_some()
            || node.as_string_node().is_some()
            || node.as_local_variable_read_node().is_some()
            || node.as_instance_variable_read_node().is_some()
            || node.as_constant_read_node().is_some()
        {
            text.into()
        } else {
            format!("({text})")
        },
    )
}
fn contains_jump(node: &Node<'_>) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Jumps(bool);
    impl<'pr> NodeVisitor<'pr> for Jumps {
        fn enter(&mut self, node: Node<'pr>) {
            self.0 |= matches!(
                node,
                Node::ReturnNode { .. }
                    | Node::BreakNode { .. }
                    | Node::NextNode { .. }
                    | Node::RedoNode { .. }
                    | Node::RetryNode { .. }
            );
        }
    }
    let mut jumps = Jumps(false);
    walk(&mut jumps, node);
    jumps.0
}
fn predicate_hazard(node: &Node<'_>, name: &[u8]) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Names<'a> {
        name: &'a [u8],
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Names<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            let name = match &node {
                Node::LocalVariableReadNode { .. } => {
                    Some(node.as_local_variable_read_node().unwrap().name())
                }
                Node::LocalVariableWriteNode { .. } => {
                    Some(node.as_local_variable_write_node().unwrap().name())
                }
                Node::LocalVariableTargetNode { .. } => {
                    Some(node.as_local_variable_target_node().unwrap().name())
                }
                Node::LocalVariableAndWriteNode { .. } => {
                    Some(node.as_local_variable_and_write_node().unwrap().name())
                }
                Node::LocalVariableOrWriteNode { .. } => {
                    Some(node.as_local_variable_or_write_node().unwrap().name())
                }
                Node::LocalVariableOperatorWriteNode { .. } => {
                    Some(node.as_local_variable_operator_write_node().unwrap().name())
                }
                _ => None,
            };
            self.found |= name.is_some_and(|n| n.as_slice() == self.name)
                || super::redundant_assignment::observes_locals(&node);
        }
    }
    let mut names = Names { name, found: false };
    walk(&mut names, node);
    names.found
}
// Skipping the self-read changes the expression result from the old local to
// nil. Never change a method/block/assignment/argument's consumed return value.
fn discarded(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    for parent in cx.ancestors() {
        if let Some(body) = parent.as_statements_node() {
            if body
                .body()
                .last()
                .is_some_and(|n| n.location().start_offset() != child.location().start_offset())
            {
                return true;
            }
        } else if parent.as_program_node().is_some()
            || (matches!(
                parent,
                Node::WhileNode { .. }
                    | Node::UntilNode { .. }
                    | Node::ForNode { .. }
                    | Node::EnsureNode { .. }
            ) && child.as_statements_node().is_some())
        {
            return true;
        } else if matches!(
            parent,
            Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::ElseNode { .. }
                | Node::BeginNode { .. }
                | Node::RescueNode { .. }
                | Node::WhenNode { .. }
                | Node::InNode { .. }
                | Node::CaseNode { .. }
                | Node::CaseMatchNode { .. }
        ) && matches!(
            child,
            Node::StatementsNode { .. }
                | Node::ElseNode { .. }
                | Node::RescueNode { .. }
                | Node::WhenNode { .. }
                | Node::InNode { .. }
        ) {
        } else {
            return false;
        }
        child = parent;
    }
    false
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
    use super::RedundantSelfAssignmentBranch as Branch;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn all_pinned_corrections_are_canonical_or_consumed_result_holds() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_self_assignment_branch_pinned.json"
        ))
        .unwrap();
        let mut held = Vec::new();
        let mut canonical = 0;
        for c in cases {
            if let Some(after) = c["correction"].as_str() {
                let before = c["source"].as_str().unwrap();
                if before.starts_with("def ") {
                    let found = correction_offenses(&Branch, before);
                    assert_eq!(found.len(), 1);
                    assert!(found[0].correction.is_none());
                    held.push(c["spec_line"].as_u64().unwrap());
                } else {
                    assert_correction(&Branch, before, after);
                    canonical += 1;
                }
            }
        }
        assert_eq!(canonical, 15);
        assert_eq!(held, [166, 268]);
    }
    #[test]
    fn consumed_values_and_first_predicate_bindings_are_not_changed() {
        for source in [
            "foo = foo ? bar : foo",
            "foo = defined?(foo) ? foo : 1",
            "foo = proc { foo } ? foo : 1",
            "foo = proc { foo = 2 } ? foo : 1",
            "foo = binding.local_variable_defined?(:foo) ? foo : 1",
            "foo = proc { local_variables }.call.include?(:foo) ? foo : 1",
            "foo = (self.local_variables ||= []).include?(:foo) ? foo : 1",
            "foo = (self.local_variables &&= []).include?(:foo) ? foo : 1",
            "foo = (self.local_variables += []).include?(:foo) ? foo : 1",
            "def f(foo); foo = ready ? foo : 3; end",
            "[1].map { |foo| foo = ready ? foo : 3 }",
            "result = begin; foo = ready ? foo : 3; end",
            "def f(foo); if cond; foo = ready ? foo : 3; end; end",
        ] {
            let found = correction_offenses(&Branch, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &Branch,
            "foo = nil; foo = foo ? bar : foo",
            "foo = nil; foo = bar if foo",
        );
        assert_correction(
            &Branch,
            "def f(foo); foo = ready ? foo : 3; foo; end",
            "def f(foo); foo = 3 unless ready; foo; end",
        );
    }
    #[test]
    fn precedence_control_flow_and_statement_scopes() {
        for (before, after) in [
            (
                "foo = if a or b; foo; else; 1; end",
                "foo = 1 unless (a or b)",
            ),
            (
                "foo = if c; foo; else; not bar; end",
                "foo = (not bar) unless c",
            ),
            (
                "foo = if c; foo; else; a and b; end",
                "foo = (a and b) unless c",
            ),
            ("foo = unless c; foo; else; 1; end", "foo = 1 if c"),
            ("foo = if c; foo; end", "foo = nil unless c"),
            (
                "begin; foo = c ? foo : 1; end",
                "begin; foo = 1 unless c; end",
            ),
            (
                "def f(foo); work; ensure; foo = c ? foo : 1; end",
                "def f(foo); work; ensure; foo = 1 unless c; end",
            ),
        ] {
            assert_correction(&Branch, before, after);
        }
        for source in [
            "def f(foo); foo = if c; foo; else; return 1; end; foo; end",
            "foo = if c # important\n foo\nelse\n 1\nend",
            "foo = if c\n foo\nelse\n __LINE__\nend",
            "foo = if c\n foo\nelse\n 1\nend\np __LINE__",
            "foo = c ? foo : DATA.pos\n__END__\npayload\n",
        ] {
            let found = correction_offenses(&Branch, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
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
    fn lazy_bounded_rewrites_do_not_scan_unbounded_expressions() {
        let source = "foo = c ? foo : 1\n".repeat(1000);
        super::REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str(&source)
                .iter()
                .filter(|o| o.cop == "Style/RedundantSelfAssignmentBranch")
                .count(),
            1000
        );
        super::REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        let found = correction_offenses(&Branch, &source);
        assert_eq!(found.len(), 1000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        super::REWRITE_BYTES.with(|n| assert!(n.get() <= source.len()));
        let huge = format!("foo = c ? foo : '{}'", "x".repeat(20000));
        super::REWRITE_BYTES.with(|n| n.set(0));
        assert!(correction_offenses(&Branch, &huge)[0].correction.is_none());
        super::REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
    }
    #[test]
    fn canonical_discarded_value_and_consumed_value_hold() {
        crate::cops::testing::assert_correction(
            &super::RedundantSelfAssignmentBranch,
            "foo = condition ? bar : foo\n",
            "foo = bar if condition\n",
        );
        let found = crate::cops::testing::correction_offenses(
            &super::RedundantSelfAssignmentBranch,
            "def f(foo); foo = condition ? bar : foo; end",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
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
