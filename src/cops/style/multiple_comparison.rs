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
            correct(&group, range.clone())
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
fn correct(group: &Group<'_>, range: std::ops::Range<usize>) -> Option<Correction> {
    // RuboCop's `skipped_within_range?`: a comparison it leaves out sits
    // between the values.
    if group.omitted.iter().any(|n| {
        n.location().start_offset() >= range.start && n.location().end_offset() <= range.end
    }) {
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
    Some(Correction {
        range,
        replacement: format!("[{elements}].include?({arg})"),
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/multiple_comparison_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
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
}
