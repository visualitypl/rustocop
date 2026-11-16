//! `Style/ParallelAssignment` — simple equal-arity parallel writes.
//!
//! Detection follows the pinned dependency-cycle exemptions. Corrections are
//! deliberately stricter: only infallible, non-dispatching RHS expressions and
//! plain local targets in discarded-value contexts can be interleaved safely.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::{BTreeSet, HashMap, HashSet};
pub struct ParallelAssignment;
#[cfg(test)]
thread_local! { static READ_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; static EDGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
impl Cop for ParallelAssignment {
    fn name(&self) -> &'static str {
        "Style/ParallelAssignment"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_multi_write_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_multi_write_node().is_none() {
            return;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        let Some(parts) = candidate(node, root, file) else {
            return;
        };
        // Like upstream ignore_node, an outer reported assignment owns nested
        // assignments (including lambda bodies) for this pass.
        if cx
            .ancestors()
            .any(|n| n.as_multi_write_node().is_some() && candidate(n, root, file).is_some())
        {
            return;
        }
        let start = node.location().start_offset();
        let end = parts.rhs.location().end_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use parallel assignment.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, &parts, cx, file))
                .flatten(),
        });
    }
}
struct Parts<'pr> {
    lhs: Vec<Node<'pr>>,
    values: Vec<Node<'pr>>,
    rhs: Node<'pr>,
    order: Vec<usize>,
}
fn candidate<'pr>(node: &Node<'pr>, root: &Node<'pr>, file: &SourceFile) -> Option<Parts<'pr>> {
    let n = node.as_multi_write_node()?;
    let mut lhs = Vec::new();
    for target in n.lefts().iter().chain(n.rest()).chain(n.rights().iter()) {
        flatten_target(target, &mut lhs)?;
    }
    if lhs.len() < 2 {
        return None;
    }
    let value = n.value();
    let rhs = value
        .as_rescue_modifier_node()
        .map_or(value, |r| r.expression());
    let array = rhs.as_array_node()?;
    let values: Vec<_> = array.elements().iter().collect();
    if values.len() != lhs.len()
        || values.iter().any(|n| n.as_splat_node().is_some())
        || file.contains_heredoc(root, &rhs)
    {
        return None;
    }
    let order = order(&lhs, &values, file)?;
    Some(Parts {
        lhs,
        values,
        rhs,
        order,
    })
}
fn flatten_target<'pr>(node: Node<'pr>, out: &mut Vec<Node<'pr>>) -> Option<()> {
    if let Some(n) = node.as_multi_target_node() {
        for target in n.lefts().iter().chain(n.rest()).chain(n.rights().iter()) {
            flatten_target(target, out)?;
        }
    } else if let Some(n) = node.as_splat_node() {
        flatten_target(n.expression()?, out)?;
    } else if node.as_implicit_rest_node().is_none() {
        out.push(node);
    }
    Some(())
}

#[derive(Hash, PartialEq, Eq)]
enum Receiver<'pr> {
    SelfValue,
    Other(ast::Fingerprint<'pr>),
}
fn receiver<'pr>(n: Node<'pr>, file: &SourceFile) -> Receiver<'pr> {
    if n.as_self_node().is_some() {
        Receiver::SelfValue
    } else {
        Receiver::Other(ast::fingerprint_for_detection(&n, file))
    }
}
#[derive(Hash, PartialEq, Eq)]
enum Key<'pr> {
    Variable(&'pr [u8]),
    Attribute(Receiver<'pr>, Vec<u8>),
    Index(Receiver<'pr>, Vec<ast::Fingerprint<'pr>>),
}
fn target<'pr>(n: &Node<'pr>, file: &SourceFile) -> Option<Key<'pr>> {
    Some(match n {
        Node::LocalVariableTargetNode { .. } => {
            Key::Variable(n.as_local_variable_target_node()?.name().as_slice())
        }
        Node::InstanceVariableTargetNode { .. } => {
            Key::Variable(n.as_instance_variable_target_node()?.name().as_slice())
        }
        Node::ClassVariableTargetNode { .. } => {
            Key::Variable(n.as_class_variable_target_node()?.name().as_slice())
        }
        Node::GlobalVariableTargetNode { .. } => {
            Key::Variable(n.as_global_variable_target_node()?.name().as_slice())
        }
        Node::ConstantTargetNode { .. } => {
            Key::Variable(n.as_constant_target_node()?.name().as_slice())
        }
        Node::ConstantPathTargetNode { .. } => {
            Key::Variable(n.as_constant_path_target_node()?.name()?.as_slice())
        }
        Node::CallTargetNode { .. } => {
            let c = n.as_call_target_node()?;
            Key::Attribute(
                receiver(c.receiver(), file),
                c.name()
                    .as_slice()
                    .strip_suffix(b"=")
                    .unwrap_or(c.name().as_slice())
                    .to_vec(),
            )
        }
        Node::IndexTargetNode { .. } => {
            let c = n.as_index_target_node()?;
            Key::Index(
                receiver(c.receiver(), file),
                c.arguments()
                    .map(|a| {
                        a.arguments()
                            .iter()
                            .map(|n| ast::fingerprint_for_detection(&n, file))
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        }
        _ => return None,
    })
}
fn read_key<'pr>(n: &Node<'pr>, file: &SourceFile) -> Option<Key<'pr>> {
    Some(match n {
        Node::LocalVariableReadNode { .. } => {
            Key::Variable(n.as_local_variable_read_node()?.name().as_slice())
        }
        Node::InstanceVariableReadNode { .. } => {
            Key::Variable(n.as_instance_variable_read_node()?.name().as_slice())
        }
        Node::ClassVariableReadNode { .. } => {
            Key::Variable(n.as_class_variable_read_node()?.name().as_slice())
        }
        Node::GlobalVariableReadNode { .. } => {
            Key::Variable(n.as_global_variable_read_node()?.name().as_slice())
        }
        Node::ConstantReadNode { .. } => {
            Key::Variable(n.as_constant_read_node()?.name().as_slice())
        }
        Node::ConstantPathNode { .. } => {
            Key::Variable(n.as_constant_path_node()?.name()?.as_slice())
        }
        Node::CallNode { .. } => {
            let c = n.as_call_node()?;
            if c.is_safe_navigation() {
                return None;
            }
            let recv = c
                .receiver()
                .map_or(Receiver::SelfValue, |n| receiver(n, file));
            if c.name().as_slice() == b"[]" {
                Key::Index(
                    recv,
                    ast::call_arguments(&c)
                        .iter()
                        .map(|n| ast::fingerprint_for_detection(n, file))
                        .collect(),
                )
            } else {
                Key::Attribute(recv, c.name().as_slice().to_vec())
            }
        }
        _ => return None,
    })
}
struct Reads<'a, 'pr> {
    file: &'a SourceFile,
    keys: &'a mut HashSet<Key<'pr>>,
    top: std::ops::Range<usize>,
}
impl<'pr> NodeVisitor<'pr> for Reads<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let n = &node;
        #[cfg(test)]
        READ_VISITS.with(|v| v.set(v.get() + 1));
        // Upstream normalizes implicit-self getters only at the RHS root.
        if n.as_call_node().is_some_and(|c| {
            c.receiver().is_none()
                && ((n.location().start_offset()..n.location().end_offset()) != self.top
                    || c.arguments().is_some()
                    || c.block().is_some())
        }) {
            return;
        }
        if let Some(k) = read_key(n, self.file) {
            self.keys.insert(k);
        }
    }
}
#[derive(Hash, PartialEq, Eq)]
enum PairValue<'pr> {
    Getter(Key<'pr>),
    Other(ast::Fingerprint<'pr>),
}
fn pair_value<'pr>(node: &Node<'pr>, file: &SourceFile) -> PairValue<'pr> {
    if node.as_call_node().is_some_and(|c| {
        !c.is_safe_navigation() && ast::call_arguments(&c).is_empty() && c.block().is_none()
    }) && let Some(key) = read_key(node, file)
    {
        return PairValue::Getter(key);
    }
    PairValue::Other(ast::fingerprint_for_detection(node, file))
}
fn order(lhs: &[Node<'_>], rhs: &[Node<'_>], file: &SourceFile) -> Option<Vec<usize>> {
    // Use the same detection equality for pair coalescing and dependency keys.
    // These normalizations do not prove correction safety; correction separately
    // rejects duplicate targets and observable/effectful expressions.
    let mut pairs = HashSet::new();
    let active: Vec<_> = (0..lhs.len())
        .filter(|&i| {
            pairs.insert((
                ast::fingerprint_for_detection(&lhs[i], file),
                pair_value(&rhs[i], file),
            ))
        })
        .collect();
    let mut writes: HashMap<_, Vec<usize>> = HashMap::new();
    let mut reads: HashMap<_, Vec<usize>> = HashMap::new();
    for &i in &active {
        if let Some(k) = target(&lhs[i], file) {
            writes.entry(k).or_default().push(i);
        }
        let mut keys = HashSet::new();
        walk::walk(
            &mut Reads {
                file,
                keys: &mut keys,
                top: rhs[i].location().start_offset()..rhs[i].location().end_offset(),
            },
            &rhs[i],
        );
        for k in keys {
            reads.entry(k).or_default().push(i);
        }
    }
    let mut after = vec![Vec::new(); lhs.len()];
    let mut degrees = vec![0usize; lhs.len()];
    for (key, writers) in writes {
        let Some(readers) = reads.get(&key) else {
            continue;
        };
        let reader_set: HashSet<_> = readers.iter().copied().collect();
        let mut overlap = writers.iter().copied().filter(|i| reader_set.contains(i));
        let own = overlap.next();
        // Two assignments each reading this same target necessarily form a cycle.
        if overlap.next().is_some() {
            return None;
        }
        // A virtual fan-out vertex avoids a reader×writer cross product for
        // repeated targets. Self reads must not introduce artificial cycles.
        let gate = after.len();
        after.push(Vec::new());
        degrees.push(0);
        for &i in readers {
            if Some(i) != own {
                after[i].push(gate);
                degrees[gate] += 1;
            }
        }
        for j in writers {
            after[gate].push(j);
            degrees[j] += 1;
            if let Some(i) = own.filter(|i| *i != j) {
                after[i].push(j);
                degrees[j] += 1;
            }
        }
    }
    #[cfg(test)]
    EDGES.with(|v| v.set(v.get() + after.iter().map(Vec::len).sum::<usize>()));
    // Release virtual vertices eagerly, then choose the first source-order
    // assignment, exactly as upstream's stable Kahn ordering does.
    let priority = |i| (i < lhs.len(), i);
    let mut ready: BTreeSet<_> = active
        .iter()
        .copied()
        .chain(lhs.len()..after.len())
        .filter(|&i| degrees[i] == 0)
        .map(priority)
        .collect();
    let mut result = Vec::new();
    while let Some((_, i)) = ready.pop_first() {
        if i < lhs.len() {
            result.push(i);
        }
        for &j in &after[i] {
            degrees[j] -= 1;
            if degrees[j] == 0 {
                ready.insert(priority(j));
            }
        }
    }
    (result.len() == active.len()).then_some(result)
}

/// RuboCop's correctors: one assignment per line at the column of the
/// statement, in an order that keeps each value's reads; under a modifier
/// the modifier becomes a block, under `rescue` a `begin` block (in a
/// method body, a `rescue` clause of it).
fn correction(
    node: &Node<'_>,
    p: &Parts<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    let column = file.line_col(loc.start_offset()).1 as usize - 1;
    let offset = " ".repeat(column);
    let indentation = " ".repeat(column + 2);
    let mut assignments = Vec::new();
    for &i in &p.order {
        let lhs = std::str::from_utf8(p.lhs[i].location().as_slice()).ok()?;
        assignments.push(format!("{lhs} = {}", value_source(&p.values[i], file)?));
    }
    // RuboCop's parent: Prism's list of a lone statement is no node of its AST.
    let lone = cx
        .parent()
        .and_then(|n| n.as_statements_node())
        .is_some_and(|s| s.body().len() == 1);
    let parent = if lone {
        cx.ancestors().nth(1)
    } else {
        cx.parent()
    };
    if let Some(guard) = parent.filter(|g| modifier(g).is_some()) {
        let (keyword, _) = modifier(guard)?;
        let head =
            std::str::from_utf8(&file.bytes[keyword.start_offset()..guard.location().end_offset()])
                .ok()?;
        return Some(Correction {
            range: guard.location().start_offset()..guard.location().end_offset(),
            replacement: format!(
                "{head}\n{indentation}{}\n{offset}end",
                assignments.join(&format!("\n{indentation}"))
            ),
        });
    }
    if let Some(rescue) = node
        .as_multi_write_node()?
        .value()
        .as_rescue_modifier_node()
    {
        let result = std::str::from_utf8(rescue.rescue_expression().location().as_slice()).ok()?;
        let in_def = lone && parent.is_some_and(|n| n.as_def_node().is_some());
        let replacement = if in_def {
            format!(
                "{}\nrescue\n{offset}{result}",
                assignments.join(&format!("\n{offset}"))
            )
        } else {
            format!(
                "begin\n{indentation}{}\n{offset}rescue\n{indentation}{result}\n{offset}end",
                assignments.join(&format!("\n{indentation}"))
            )
        };
        return Some(Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement,
        });
    }
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: assignments.join(&format!("\n{offset}")),
    })
}
/// RuboCop's `source`: a word of a percent array as a quoted string or a
/// symbol's `inspect`, anything else as its source.
fn value_source(n: &Node<'_>, file: &SourceFile) -> Option<String> {
    if let Some(s) = n.as_string_node().filter(|s| s.opening_loc().is_none()) {
        let text = std::str::from_utf8(s.unescaped()).ok()?;
        return Some(format!(
            "'{}'",
            text.replace('\\', "\\\\").replace('\'', "\\'")
        ));
    }
    if let Some(s) = n.as_symbol_node().filter(|s| s.opening_loc().is_none()) {
        let value = s.unescaped();
        let inspected = super::super::lint::symbol_conversion::inspect_symbol(value, file, false);
        return String::from_utf8(inspected).ok();
    }
    Some(std::str::from_utf8(n.location().as_slice()).ok()?.into())
}
fn modifier<'pr>(node: &Node<'pr>) -> Option<(ruby_prism::Location<'pr>, Node<'pr>)> {
    let (keyword, predicate) = match node {
        Node::IfNode { .. } => {
            let n = node.as_if_node()?;
            (n.if_keyword_loc()?, n.predicate())
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node()?;
            (n.keyword_loc(), n.predicate())
        }
        Node::WhileNode { .. } => {
            let n = node.as_while_node()?;
            if n.is_begin_modifier() {
                return None;
            }
            (n.keyword_loc(), n.predicate())
        }
        Node::UntilNode { .. } => {
            let n = node.as_until_node()?;
            if n.is_begin_modifier() {
                return None;
            }
            (n.keyword_loc(), n.predicate())
        }
        _ => return None,
    };
    (keyword.start_offset() > node.location().start_offset()).then_some((keyword, predicate))
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop parallel_assignment_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/ParallelAssignment")
            .collect()
    }
    #[test]
    fn pinned_diagnostics_are_complete_and_exact() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/parallel_assignment_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 86);
        for c in cases {
            let found = offenses(c["source"].as_str().unwrap());
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(
                serde_json::to_value(positions).unwrap(),
                c["positions"],
                "spec {}: {}",
                c["spec_line"],
                c["source"]
            );
            let messages: Vec<_> = found.iter().map(|o| o.message.as_str()).collect();
            assert_eq!(serde_json::to_value(messages).unwrap(), c["messages"]);
        }
    }
    #[test]
    fn canonical_modifiers_and_literal_hashes() {
        for keyword in ["if", "unless", "while", "until"] {
            crate::cops::testing::assert_correction(
                &ParallelAssignment,
                &format!("a,b=1,2 {keyword} foo\n"),
                &format!("{keyword} foo\n  a = 1\n  b = 2\nend\n"),
            );
        }
        crate::cops::testing::assert_correction(
            &ParallelAssignment,
            "a,b={a: 1},{b: 2}\n",
            "a = {a: 1}\nb = {b: 2}\n",
        );
    }
    #[test]
    fn later_heredocs_are_not_lost_from_the_shared_token_index() {
        assert_eq!(offenses("a,b=1,2\nc,d=<<~A,2\n text\nA\n").len(), 1);
        assert_eq!(offenses("a,b=1,2\nc,d=foo(<<~A),2\n text\nA\n").len(), 1);
    }
    #[test]
    fn every_variable_kind_has_dependency_cycle_exemptions() {
        for (a, b) in [
            ("a", "b"),
            ("@a", "@b"),
            ("@@a", "@@b"),
            ("$a", "$b"),
            ("A", "B"),
            ("C::A", "C::B"),
            ("o.a", "o.b"),
            ("o[0]", "o[1]"),
        ] {
            assert_eq!(offenses(&format!("{a},{b}=1,2\n")).len(), 1);
            assert!(offenses(&format!("{a},{b}={b},{a}\n")).is_empty());
        }
    }
    #[test]
    fn dependency_cycles_normalize_call_spelling_and_evaluated_line_values() {
        for source in [
            "o[0].a,o[1].b=o.[](1).b,o.[](0).a\n",
            "o[__LINE__],o[2]=o[2],o[1]\n",
            "o[1r],o[2]=o[2],o[1.0r]\n",
            "o[foo[0]],o[2]=o[2],o[foo.[](0)]\n",
            "\no[__LINE__],o[3]=o[3],o[2]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn detection_equality_preserves_distinct_values_and_safe_navigation() {
        for source in [
            "a,a=[a,1r],[a,1.000000000000000000001r]\n",
            "a,a=[a,__LINE__],\n[a,__LINE__]\n",
            "o[0].a,o[0].a=[o[0].a,1],[o[0].a,2]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "o[0].a,o[1].b=o&.[](1).b,o&.[](0).a\n",
            "o[__LINE__],o[2]=o[2],o[3]\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1), "{source}");
            assert_eq!(found[0].message, "Do not use parallel assignment.");
        }
    }
    #[test]
    fn canonical_literals() {
        crate::cops::testing::assert_correction(
            &ParallelAssignment,
            "a,b,c=1,2,3\n",
            "a = 1\nb = 2\nc = 3\n",
        );
    }
}
