//! `Style/RedundantAssignment` — local assignment immediately before a method's final read.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantAssignment;
impl Cop for RedundantAssignment {
    fn name(&self) -> &'static str {
        "Style/RedundantAssignment"
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
        let Some((assignment, reference)) = pair(node) else {
            return;
        };
        if !method_tail(node, cx) {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| correction(&assignment, &reference, cx, file))
            .flatten();
        super::negated_if::report(
            self,
            &assignment,
            file,
            "Redundant assignment before returning detected.".into(),
            correction,
            out,
        );
    }
}
fn pair<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>)> {
    let body = node.as_statements_node()?;
    let mut previous = None;
    let mut last = None;
    for n in body.body().iter() {
        previous = last;
        last = Some(n);
    }
    let (assignment, reference) = (previous?, last?);
    let write = assignment.as_local_variable_write_node()?;
    let read = reference.as_local_variable_read_node()?;
    (write.name().as_slice() == read.name().as_slice() && write.depth() == read.depth())
        .then_some((assignment, reference))
}
fn correction(
    assignment: &Node<'_>,
    reference: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let start = assignment.location().start_offset();
    let end = reference.location().end_offset();
    if end - start > 16 * 1024
        || super::correction::contains_comment(
            file,
            file.line_start(file.line_of(start))..file.line_start(file.line_of(end) + 1),
        )
        || !file
            .redundant_assignment_safety(root)
            .removable
            .contains(&start)
        || file.contains_heredoc(root, assignment)
        || file.heredoc_pending_at(root, start)
    {
        return None;
    }
    let value = assignment.as_local_variable_write_node()?.value();
    let text = std::str::from_utf8(value.location().as_slice()).ok()?;
    let gap = std::str::from_utf8(
        &file.bytes[assignment.location().end_offset()..reference.location().start_offset()],
    )
    .ok()?;
    super::negated_if::finish(
        Correction {
            range: start..end,
            replacement: format!("{text}{gap}"),
        },
        cx,
        file,
    )
}
/// Compound attribute writes invoke a getter that is not a `CallNode` child.
pub(super) fn observes_locals(node: &Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|n| n.name().as_slice() == b"local_variables")
        || node
            .as_call_or_write_node()
            .is_some_and(|n| n.read_name().as_slice() == b"local_variables")
        || node
            .as_call_and_write_node()
            .is_some_and(|n| n.read_name().as_slice() == b"local_variables")
        || node
            .as_call_operator_write_node()
            .is_some_and(|n| n.read_name().as_slice() == b"local_variables")
}
#[cfg(test)]
thread_local! { static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
/// One lazy whole-file scope proof. Every reference must be a paired terminal
/// read, so a remaining branch's read always has its own preceding write.
/// Captures, reflection and other reads keep the finding but not the edit.
#[derive(Default)]
pub(crate) struct Safety {
    removable: std::collections::HashSet<usize>,
    pub(super) dynamic_writes: std::collections::HashSet<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn of(root: &Node<'_>) -> Self {
        #[cfg(test)]
        BUILDS.with(|n| n.set(n.get() + 1));
        use crate::cops::{
            variables::{ReferenceKind, ScopeAnalysis},
            walk::{NodeVisitor, walk},
        };
        #[derive(Default)]
        struct Pairs {
            reads: std::collections::HashSet<usize>,
            observes_locals: bool,
            visits: usize,
        }
        impl<'pr> NodeVisitor<'pr> for Pairs {
            fn enter(&mut self, node: Node<'pr>) {
                self.visits += 1;
                self.observes_locals |= observes_locals(&node);
                if let Some((_, reference)) = pair(&node) {
                    self.reads.insert(reference.location().start_offset());
                }
            }
        }
        let mut pairs = Pairs::default();
        walk(&mut pairs, root);
        let mut result = Self::default();
        #[cfg(test)]
        {
            result.visits = pairs.visits;
        }
        let Some(program) = root.as_program_node() else {
            return result;
        };
        let analysis = ScopeAnalysis::of(&program);
        for scope in analysis.scopes_in_exit_order() {
            for id in analysis.scope(scope).variables() {
                let var = analysis.variable(id);
                if analysis.may_be_dynamically_mutated(id) {
                    for id in &var.assignments {
                        result
                            .dynamic_writes
                            .insert(analysis.assignment(*id).node.location().start_offset());
                    }
                }
                if pairs.observes_locals
                    || var.captured_by_block
                    || var.has_pattern_bindings
                    || analysis.may_be_dynamically_mutated(id)
                    || var.references.iter().any(|r| {
                        let r = analysis.reference(*r);
                        r.kind != ReferenceKind::Read || !pairs.reads.contains(&r.range.start)
                    })
                {
                    continue;
                }
                for id in &var.assignments {
                    result
                        .removable
                        .insert(analysis.assignment(*id).node.location().start_offset());
                }
            }
        }
        result
    }
}
fn method_tail(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    for parent in cx.ancestors() {
        match parent {
            Node::DefNode { .. } => return true,
            Node::StatementsNode { .. } => {
                if !parent
                    .as_statements_node()
                    .unwrap()
                    .body()
                    .last()
                    .is_some_and(|n| n.location().start_offset() == child.location().start_offset())
                {
                    return false;
                }
            }
            Node::IfNode { .. } => {
                let n = parent.as_if_node().unwrap();
                if n.if_keyword_loc().is_none()
                    || n.end_keyword_loc().is_none()
                    || n.predicate().location().start_offset() == child.location().start_offset()
                {
                    return false;
                }
            }
            Node::UnlessNode { .. } => {
                let n = parent.as_unless_node().unwrap();
                if n.end_keyword_loc().is_none()
                    || n.predicate().location().start_offset() == child.location().start_offset()
                {
                    return false;
                }
            }
            Node::BeginNode { .. } => {
                if parent.as_begin_node().unwrap().ensure_clause().is_some()
                    && child.as_ensure_node().is_none()
                {
                    return false;
                }
            }
            Node::CaseNode { .. } | Node::CaseMatchNode { .. } => {
                if !matches!(
                    child,
                    Node::WhenNode { .. } | Node::InNode { .. } | Node::ElseNode { .. }
                ) {
                    return false;
                }
            }
            Node::WhenNode { .. } | Node::InNode { .. } | Node::RescueNode { .. } => {
                if !matches!(child, Node::StatementsNode { .. } | Node::RescueNode { .. }) {
                    return false;
                }
            }
            // Either operand supplies the terminal rescue expression's value.
            Node::RescueModifierNode { .. }
            | Node::ElseNode { .. }
            | Node::EnsureNode { .. }
            | Node::ParenthesesNode { .. } => {}
            _ => return false,
        }
        child = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_assignment_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::RedundantAssignment;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn scope_reflection_and_captured_values_keep_their_assignments() {
        for source in [
            "def f; x = nil; capture = -> { x }; x = capture; x; end",
            "def f; x = -> { x }; x; end",
            "def f; b = binding; x = 1; x; end",
            "def f; x = 1; saved = -> { x }; x = 2; x; end",
            "def f; x = 1; x; ensure; p x; end",
            "def f; x = 1; x; rescue; p x; end",
            "def f; x = defined?(x); x; end",
            "def f; x = local_variables; x; end",
            "def f; x = proc { local_variables }; x; end",
            "def f; x = (self.local_variables ||= []); x; end",
            "def f; x = (self.local_variables &&= []); x; end",
            "def f; x = (self.local_variables += []); x; end",
            "def f; x = (self&.local_variables ||= []); x; end",
        ] {
            let found = correction_offenses(&RedundantAssignment, source);
            // A method-level ensure suppresses the main-body finding upstream.
            if source.contains("ensure") {
                assert!(found.is_empty());
            } else {
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn only_method_tail_branches_not_predicates_loops_or_nonlocals() {
        for source in [
            "x = 1; x",
            "def f; x = 1; x; work; end",
            "def f; 1.times { x = 1; x }; end",
            "def f; while cond; x = 1; x; end; end",
            "def f; case (x = 1; x); when 1; work; end; end",
            "def f; case 1; when (x = 1; x); work; end; end",
            "def f; if (x = 1; x); work; end; end",
            "def f; @x = 1; @x; end",
            "def f; @@x = 1; @@x; end",
            "def f; $x = 1; $x; end",
            "def f; self.x = 1; x; end",
            "def f; a,b = 1,2; a; end",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Style/RedundantAssignment"),
                "{source}"
            );
        }
        for (source, after) in [
            ("def self.f; x = 1; x; end", "def self.f; 1; ; end"),
            ("def f; (x = 1; x); end", "def f; (1; ); end"),
            (
                "def f; unless cond; x = 1; x; end; end",
                "def f; unless cond; 1; ; end; end",
            ),
            (
                "def f; work; ensure; x = 1; x; end",
                "def f; work; ensure; 1; ; end",
            ),
        ] {
            assert_correction(&RedundantAssignment, source, after);
        }
    }
    #[test]
    fn terminal_rescue_modifiers_check_both_value_branches() {
        for (source, after, col) in [
            (
                "def f; work rescue (x = 1; x); end",
                "def f; work rescue (1; ); end",
                21,
            ),
            (
                "def f; (x = 1; x) rescue work; end",
                "def f; (1; ) rescue work; end",
                9,
            ),
            (
                "def f; work rescue (other rescue (x = 1; x)); end",
                "def f; work rescue (other rescue (1; )); end",
                35,
            ),
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/RedundantAssignment")
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                "Redundant assignment before returning detected."
            );
            assert_correction(&RedundantAssignment, source, after);
        }
        for source in [
            "def f; work rescue (x = 1; x); more; end",
            "def f; consume(work rescue (x = 1; x)); end",
            "def f; work rescue (x = 1; x); ensure; cleanup; end",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Style/RedundantAssignment"),
                "{source}"
            );
        }
    }
    #[test]
    fn comments_heredocs_and_source_observers_withhold_edits() {
        for source in [
            "def f\n x = 1 # note\n x\nend",
            "def f\n x = 1\n # note\n x\nend",
            "def f\n x = <<~TEXT\n value\nTEXT\n x\nend",
            "def f\n x =\n __LINE__\n x\nend",
            "def f; x = DATA.pos; x; end\n__END__\npayload\n",
        ] {
            let found = correction_offenses(&RedundantAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn lazy_scope_index_and_statement_scans_have_bounded_work() {
        let source = "def f; x=1; x; end\n".repeat(1000);
        super::BUILDS.with(|n| n.set(0));
        assert_eq!(
            lint_str(&source)
                .iter()
                .filter(|o| o.cop == "Style/RedundantAssignment")
                .count(),
            1000
        );
        super::BUILDS.with(|n| assert_eq!(n.get(), 0));
        let file = crate::source::SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let result = ruby_prism::parse(source.as_bytes());
        let root = result.node();
        // Main-parse correction evidence is built once and queried by offset.
        let facts = file.redundant_assignment_safety(&root);
        assert_eq!(facts.removable.len(), 1000);
        assert!(facts.visits < source.len());
        assert!(std::ptr::eq(facts, file.redundant_assignment_safety(&root)));
        let found = correction_offenses(&RedundantAssignment, &source);
        assert!(found.iter().all(|o| o.correction.is_some()));
        let huge = format!("def f; x='{}'; x; end", "x".repeat(20000));
        assert!(
            correction_offenses(&RedundantAssignment, &huge)[0]
                .correction
                .is_none()
        );
    }
    #[test]
    fn every_pinned_correction_is_canonical() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_assignment_pinned.json"
        ))
        .unwrap();
        for c in cases {
            if let Some(after) = c["correction"].as_str() {
                crate::cops::testing::assert_correction(
                    &super::RedundantAssignment,
                    c["source"].as_str().unwrap(),
                    after,
                );
            }
        }
    }
    #[test]
    fn every_pinned_example_has_exact_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_assignment_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 12);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/RedundantAssignment")
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
