//! `Style/MultipleComparison` — repeated equality comparisons in a disjunction.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MultipleComparison;
impl Cop for MultipleComparison {
    fn name(&self) -> &'static str {
        "Style/MultipleComparison"
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
        if node.as_or_node().is_none() || cx.parent().is_some_and(|p| p.as_or_node().is_some()) {
            return;
        }
        let Some(group) = collect(node, file) else {
            return;
        };
        let first = group.comparisons.first().unwrap().location();
        let last = group.comparisons.last().unwrap().location();
        let range = first.start_offset()..last.end_offset();
        let (line, col) = file.line_col(range.start);
        let correction = if cx.corrections_requested() {
            correct(&group, range.clone(), cx, file)
        } else {
            None
        };
        out.push(Offense { cop: self.name(), severity: self.severity(), line, col, end_line: file.line_of(range.end - 1), message: "Avoid comparing a variable with multiple items in a conditional, use `Array#include?` instead.".into(), correction });
    }
}
struct Group<'pr> {
    variable: Node<'pr>,
    values: Vec<Node<'pr>>,
    comparisons: Vec<Node<'pr>>,
    omitted: Vec<Node<'pr>>,
}
fn variable(node: &Node<'_>) -> bool {
    node.as_local_variable_read_node().is_some() || is_send(node)
}
fn is_send(node: &Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|c| c.block().is_none_or(|b| b.as_block_node().is_none()))
}
fn comparison<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>)> {
    let c = node.as_call_node()?;
    if c.name().as_slice() != b"==" || c.is_safe_navigation() || !is_send(node) {
        return None;
    }
    let lhs = c.receiver()?;
    let args = ast::call_arguments(&c);
    if args.len() != 1 {
        return None;
    }
    let rhs = args.into_iter().next()?;
    if variable(&lhs) {
        Some((lhs, rhs))
    } else if variable(&rhs) {
        Some((rhs, lhs))
    } else {
        None
    }
}
fn collect<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Group<'pr>> {
    let mut pending = vec![node.as_or_node()?.as_node()];
    let mut first = None;
    let mut fingerprint = None;
    let mut different = false;
    let mut values = Vec::new();
    let mut comparisons = Vec::new();
    let mut omitted = Vec::new();
    let mut skipped = Vec::new();
    while let Some(n) = pending.pop() {
        if let Some(or) = n.as_or_node() {
            pending.push(or.right());
            pending.push(or.left());
            continue;
        }
        let (var, value) = comparison(&n)?;
        if var.as_local_variable_read_node().is_some()
            && value.as_local_variable_read_node().is_some()
        {
            omitted.push(n);
            continue;
        }
        if is_send(&value) {
            skipped.push(n);
            continue;
        }
        // Detection matches RuboCop's value-based numeric AST equality;
        // correction proofs below still require a closed Integer local.
        let key = ast::fingerprint_with_integer_source_lines(&var, file);
        if first.is_none() {
            first = Some(var);
            fingerprint = Some(key.clone());
        }
        different |= fingerprint.as_ref() != Some(&key);
        // Match the pinned first-variable collection policy, even when a
        // later unrelated variable appears: never resume after that boundary.
        if !different {
            values.push(value);
            comparisons.push(n);
        }
    }
    if values.len() < 2 {
        return None;
    }
    let start = comparisons.first()?.location().start_offset();
    let end = comparisons.last()?.location().end_offset();
    if skipped
        .iter()
        .any(|n| n.location().start_offset() >= start && n.location().end_offset() <= end)
    {
        return None;
    }
    Some(Group {
        variable: first?,
        values,
        comparisons,
        omitted,
    })
}
fn small_integer(n: &Node<'_>) -> bool {
    n.as_integer_node().is_some()
        && super::even_odd::integer(n, None).is_some_and(|i| (-(1 << 30)..(1 << 30)).contains(&i))
}
fn correct(
    group: &Group<'_>,
    range: std::ops::Range<usize>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    group.variable.as_local_variable_read_node()?;
    if !group.values.iter().all(small_integer)
        || group.omitted.iter().any(|n| {
            n.location().start_offset() >= range.start && n.location().end_offset() <= range.end
        })
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.multiple_comparison_safety(root);
    if !facts
        .integer_reads
        .contains(&group.variable.location().start_offset())
        || facts.changed
        || facts.first_effect.is_some_and(|end| end <= range.start)
        || super::correction::contains_comment(file, range.clone())
        || super::correction::touches_directive(file, range.clone())
        || file.heredoc_pending_at(root, range.start)
    {
        return None;
    }
    let elements = group
        .values
        .iter()
        .map(|n| std::str::from_utf8(n.location().as_slice()))
        .collect::<Result<Vec<_>, _>>()
        .ok()?
        .join(", ");
    let arg = std::str::from_utf8(group.variable.location().as_slice()).ok()?;
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range,
            replacement: format!("[{elements}].include?({arg})"),
        },
    )
}
/// Correction-only owned summary of the shared VariableForce analysis. A single
/// unconditional top-level immediate-Integer assignment must dominate each read;
/// captures, rebinding, dynamic mutation and unknown/deferred dispatch invalidate
/// the proof. This is not inferred purity for arbitrary locals or calls.
#[derive(Default)]
pub(crate) struct Safety {
    integer_reads: std::collections::HashSet<usize>,
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        use crate::cops::variables::ScopeAnalysis;
        let mut facts = Self::default();
        if let Some(program) = root.as_program_node() {
            let analysis = ScopeAnalysis::of(&program);
            let writes: std::collections::HashSet<_> = program
                .statements()
                .body()
                .iter()
                .filter(|n| n.as_local_variable_write_node().is_some())
                .map(|n| n.location().start_offset())
                .collect();
            for scope in analysis.scopes_in_exit_order() {
                for id in analysis.scope(scope).variables() {
                    let var = analysis.variable(id);
                    if var.is_parameter()
                        || var.captured_by_block
                        || var.has_pattern_bindings
                        || analysis.may_be_dynamically_mutated(id)
                        || var.assignments.len() != 1
                    {
                        continue;
                    }
                    let assignment = analysis.assignment(var.assignments[0]);
                    let Some(write) = assignment.node.as_local_variable_write_node() else {
                        continue;
                    };
                    if !writes.contains(&write.location().start_offset())
                        || !small_integer(&write.value())
                    {
                        continue;
                    }
                    for reference in &var.references {
                        let r = analysis.reference(*reference);
                        if r.is_explicit() && r.range.start >= write.location().end_offset() {
                            facts.integer_reads.insert(r.range.start);
                        }
                    }
                }
            }
        }
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
    fn integer(&self, n: &Node<'_>) -> bool {
        small_integer(n)
            || (n.as_local_variable_read_node().is_some()
                && self.integer_reads.contains(&n.location().start_offset()))
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        if let Some(c) = node.as_call_node() {
            let args = ast::call_arguments(&c);
            let trusted = !c.is_safe_navigation()
                && c.block().is_none()
                && args.len() == 1
                && c.receiver().is_some_and(|lhs| {
                    (c.name().as_slice() == b"==" && self.integer(&lhs) && self.integer(&args[0]))
                        || (c.name().as_slice() == b"include?"
                            && lhs
                                .as_array_node()
                                .is_some_and(|a| a.elements().iter().all(|n| small_integer(&n)))
                            && self.integer(&args[0]))
                });
            if !trusted {
                self.changed |= c.block().is_some();
                let end = node.location().end_offset();
                self.first_effect = Some(self.first_effect.map_or(end, |old| old.min(end)));
            }
        } else {
            self.changed |= !matches!(
                node,
                Node::ProgramNode { .. }
                    | Node::StatementsNode { .. }
                    | Node::ArgumentsNode { .. }
                    | Node::IntegerNode { .. }
                    | Node::LocalVariableReadNode { .. }
                    | Node::LocalVariableWriteNode { .. }
                    | Node::OrNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::ArrayNode { .. }
                    | Node::SourceLineNode { .. }
            );
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/multiple_comparison_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == MultipleComparison.name())
            .collect()
    }
    #[test]
    fn repeated_variables_and_method_receivers() {
        for expression in [
            "a == 1 || a == 2",
            "1 == a || 2 == a",
            "a == 1 || 2 == a || a == 3",
            "a[:key] == 'a' || a[:key] == 'b'",
            "'a' == a&.work || 'b' == a&.work",
        ] {
            let s = format!("a = 1\nif {expression}\nend");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, 4));
            assert_eq!(
                o[0].message,
                "Avoid comparing a variable with multiple items in a conditional, use `Array#include?` instead."
            );
        }
    }
    #[test]
    fn skipped_method_comparisons_and_ranges() {
        for (s, col) in [
            ("a = 1; a == foo || a == 1 || a == 2", 20),
            ("a = 1; a == 1 || a == 2 || a == foo", 8),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, col));
        }
        for s in [
            "a=1; a == 1 || a == foo || a == 2",
            "a=1; a == before.column || a == after.column",
            "a=1; a == before&.column || a == after&.column",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn lookalikes_and_entire_or_chain_eligibility() {
        for s in [
            "a=1; a == 1",
            "1 == 1 || 1 == 2",
            "'a' == 'a' || 'a' == 'b'",
            "a=1; b=2; a == 1 || b == 2",
            "a=1; b=2; a == b || b == a",
            "a=1; b=2; a == b || a == b",
            "a=1; a == 1 || a == 2 || work",
            "a=1; a != 1 || a != 2",
            "a=1; a == 1 && a == 2",
            "a=1; [1,2].include?(a)",
            "def f(foo, bar); bar.work == bar || foo == :sym; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn block_passes_are_send_arguments_not_literal_block_wrappers() {
        for s in [
            "foo(&b) == 1 || foo(&b) == 2",
            "foo(&:bar) == 1 || foo(&:bar) == 2",
            "foo(&factory()) == 1 || foo(&factory()) == 2",
            "a=1; a.==(&b) || a.==(&c)",
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(
                correction_offenses(&MultipleComparison, s)[0]
                    .correction
                    .is_none()
            );
        }
        for s in [
            "foo { work } == 1 || foo { work } == 2",
            "foo(&b) == 1 || foo(&c) == 2",
            "a=1; a.==(1) { work } || a == 2",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }

    #[test]
    fn detection_compares_numeric_arguments_by_ruby_ast_value() {
        for s in [
            "foo(-0.0) == 1 || foo(0.0) == 2",
            "foo(__LINE__) == 1 || foo(1) == 2",
        ] {
            let o = correction_offenses(&MultipleComparison, s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 1));
            assert_eq!(
                o[0].message,
                "Avoid comparing a variable with multiple items in a conditional, use `Array#include?` instead."
            );
            assert!(o[0].correction.is_none(), "{s}");
        }
        for s in [
            "foo(-0.5) == 1 || foo(0.5) == 2",
            "foo(__LINE__) == 1 || foo(2) == 2",
            "foo(__LINE__) == 1 ||\nfoo(__LINE__) == 2",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }

    #[test]
    fn canonical_closed_local_integer_corrections() {
        for (s, after) in [
            ("a = 1; a == 1 || a == 2", "a = 1; [1, 2].include?(a)"),
            (
                "a = 1; 1 == a || 2 == a || 3 == a",
                "a = 1; [1, 2, 3].include?(a)",
            ),
            ("a = 1; a == -1 or a == 0", "a = 1; [-1, 0].include?(a)"),
        ] {
            assert_correction(&MultipleComparison, s, after);
        }
    }
    #[test]
    fn unknown_dispatch_truthy_comparisons_and_effects_remain_reported() {
        for s in [
            "def f(a); a == 1 || a == 2; end",
            "a = unknown; a == 1 || a == 2",
            "a=1; a == (x = 1) || a == (x = 2)",
            "a=1; a == C || a == D",
            "foo == 1 || foo == 2",
            "a[:key] == 1 || a[:key] == 2",
            "a&.work == 1 || a&.work == 2",
            "a=1; a == 'a' || a == 'b'",
            "a=1; effect; a == 1 || a == 2",
            "a=1; a=unknown; a == 1 || a == 2",
            "class Integer; def ==(x); :truthy; end; end; a=1; a == 1 || a == 2",
            "a=1; b=2; a == 1 || a == b || a == 2",
        ] {
            let o = correction_offenses(&MultipleComparison, s);
            assert!(!o.is_empty(), "{s}");
            assert!(o.iter().all(|o| o.correction.is_none()), "{s}");
        }
    }
}
