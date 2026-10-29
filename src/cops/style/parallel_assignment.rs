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
    rescue: bool,
    destructuring: bool,
}
fn candidate<'pr>(node: &Node<'pr>, root: &Node<'pr>, file: &SourceFile) -> Option<Parts<'pr>> {
    let n = node.as_multi_write_node()?;
    let mut lhs = Vec::new();
    let mut destructuring = false;
    for target in n.lefts().iter().chain(n.rest()).chain(n.rights().iter()) {
        flatten_target(target, &mut lhs, &mut destructuring)?;
    }
    if lhs.len() < 2 {
        return None;
    }
    let value = n.value();
    let rescue = value.as_rescue_modifier_node().is_some();
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
        rescue,
        destructuring,
    })
}
fn flatten_target<'pr>(
    node: Node<'pr>,
    out: &mut Vec<Node<'pr>>,
    destructuring: &mut bool,
) -> Option<()> {
    if let Some(n) = node.as_multi_target_node() {
        *destructuring = true;
        for target in n.lefts().iter().chain(n.rest()).chain(n.rights().iter()) {
            flatten_target(target, out, destructuring)?;
        }
    } else if let Some(n) = node.as_splat_node() {
        *destructuring = true;
        flatten_target(n.expression()?, out, destructuring)?;
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

fn correction(
    node: &Node<'_>,
    p: &Parts<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    if p.rescue
        || p.destructuring
        || loc.as_slice().len() > 16 * 1024
        || !discarded(node, cx)
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
    {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|v| v.set(v.get() + loc.as_slice().len()));
    let root = cx.ancestors().next_back()?;
    let mut modifiers = cx
        .ancestors()
        .take_while(|n| n.as_program_node().is_none() && n.as_def_node().is_none())
        .filter(|n| modifier(n).is_some());
    let guard = modifiers.next();
    if modifiers.next().is_some() {
        return None;
    }
    let envelope = guard.unwrap_or(node);
    // A modifier wrapping a larger if/begin/loop is not this assignment's
    // modifier: replacing its whole envelope would discard unrelated code.
    if guard.is_some_and(|g| !direct_modifier_body(g, node))
        || envelope.location().as_slice().len() > 16 * 1024
    {
        return None;
    }
    if file.heredoc_pending_at(root, loc.start_offset())
        || file.contains_heredoc(root, envelope)
        || super::correction::contains_comment(
            file,
            envelope.location().start_offset()..envelope.location().end_offset(),
        )
    {
        return None;
    }
    if let Some(guard) = guard {
        let (_, predicate) = modifier(guard)?;
        if file.contains_keyword_block(root, &predicate)
            || !super::if_unless_modifier::expansion_binding_safe(node, &predicate)
        {
            return None;
        }
    }
    // Do not move declarations around implicit block-local/captured scopes or
    // method results. The shared scope index proves reads retain local binding.
    let locals = file.visible_locals(root);
    let mut targets = HashSet::new();
    for n in &p.lhs {
        let n = n.as_local_variable_target_node()?;
        // Reordering duplicate stores can change which value wins.
        // Let TrailingUnderscoreVariable retain forced destructuring while
        // removing discard targets; do not turn them into standalone `_ =`.
        if n.name().as_slice() == b"_" || !targets.insert((n.name().as_slice(), n.depth())) {
            return None;
        }
    }
    for (i, n) in p.values.iter().enumerate() {
        let mut safe = Pure {
            safe: true,
            hashes: Vec::new(),
            hash_depth: 0,
            check_read: &mut |read: &Node<'_>| {
                let r = read.as_local_variable_read_node().unwrap();
                let l = p.lhs[i].as_local_variable_target_node().unwrap();
                (r.name().as_slice() == l.name().as_slice() && r.depth() == l.depth())
                    || loc.start_offset().checked_sub(1).is_some_and(|offset| {
                        locals.declared_before(r.name().as_slice(), node, cx, offset)
                    })
            },
        };
        walk::walk(&mut safe, n);
        if !safe.safe {
            return None;
        }
    }
    let line = file.line_of(loc.start_offset());
    let mut prefix = &file.bytes[file.line_start(line)..loc.start_offset()];
    if prefix.starts_with(b"\xef\xbb\xbf") {
        prefix = &prefix[3..];
    }
    if !prefix.iter().all(|b| matches!(b, b' ' | b'\t')) {
        return None;
    }
    let indent = std::str::from_utf8(prefix).ok()?;
    let newline = if file.bytes[file.line_start(line)..file.line_start(line + 1)].ends_with(b"\r\n")
    {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines = Vec::new();
    for &i in &p.order {
        let lhs = std::str::from_utf8(p.lhs[i].location().as_slice()).ok()?;
        let rhs = value_source(&p.values[i], &p.rhs)?;
        lines.push(format!("{lhs} = {rhs}"));
    }
    let replacement = if let Some(guard) = guard {
        let (keyword, predicate) = modifier(guard)?;
        let keyword = std::str::from_utf8(keyword.as_slice()).ok()?;
        let predicate = std::str::from_utf8(predicate.location().as_slice()).ok()?;
        let body = lines.join(&format!("{newline}{indent}  "));
        format!("{keyword} {predicate}{newline}{indent}  {body}{newline}{indent}end")
    } else {
        lines.join(&format!("{newline}{indent}"))
    };
    super::negated_if::finish(
        Correction {
            range: loc.start_offset()..envelope.location().end_offset(),
            replacement,
        },
        cx,
        file,
    )
}
struct Pure<'a> {
    safe: bool,
    hashes: Vec<bool>,
    hash_depth: usize,
    check_read: &'a mut dyn FnMut(&Node<'_>) -> bool,
}
impl<'pr> NodeVisitor<'pr> for Pure<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        let n = &node;
        let hash = n.as_hash_node().is_some();
        self.hashes.push(hash);
        self.hash_depth += usize::from(hash);
        self.safe &= match n {
            Node::LocalVariableReadNode { .. } => self.hash_depth == 0 && (self.check_read)(n),
            Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::HashNode { .. } => true,
            Node::AssocNode { .. } => {
                let key = n.as_assoc_node().unwrap().key();
                matches!(key, Node::SymbolNode { .. } | Node::StringNode { .. })
            }
            Node::ArrayNode { .. }
            | Node::SourceFileNode { .. }
            | Node::SourceEncodingNode { .. } => true,
            _ => false,
        };
    }
    fn leave(&mut self) {
        self.hash_depth -= usize::from(self.hashes.pop().unwrap());
    }
}
fn value_source(n: &Node<'_>, rhs: &Node<'_>) -> Option<String> {
    // Percent-array elements do not own delimiters. Emit decoded values rather
    // than copying bare words (which would turn them into method calls).
    let percent = rhs
        .as_array_node()?
        .opening_loc()
        .is_some_and(|l| l.as_slice().starts_with(b"%"));
    if percent {
        // Uppercase arrays can force a different encoding via Unicode escapes.
        // Keep findings, but do not guess per-element encoding provenance.
        let opening = rhs.as_array_node()?.opening_loc()?;
        if !matches!(opening.as_slice().get(1), Some(b'w' | b'i')) {
            return None;
        }
        if let Some(s) = n.as_string_node() {
            let text = std::str::from_utf8(s.unescaped()).ok()?;
            if text.bytes().any(|b| b.is_ascii_control()) {
                return None;
            }
            return Some(format!(
                "'{}'",
                text.replace('\\', "\\\\").replace('\'', "\\'")
            ));
        }
        if let Some(s) = n.as_symbol_node() {
            let text = std::str::from_utf8(s.unescaped()).ok()?;
            if text.bytes().any(|b| b.is_ascii_control()) {
                return None;
            }
            if !text.is_empty()
                && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !text.as_bytes()[0].is_ascii_digit()
            {
                return Some(format!(":{text}"));
            }
            return Some(format!(
                ":\"{}\"",
                text.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('#', "\\#")
            ));
        }
        return None;
    }
    Some(std::str::from_utf8(n.location().as_slice()).ok()?.into())
}
fn direct_modifier_body(guard: &Node<'_>, node: &Node<'_>) -> bool {
    let body = match guard {
        Node::IfNode { .. } => guard.as_if_node().and_then(|n| n.statements()),
        Node::UnlessNode { .. } => guard.as_unless_node().and_then(|n| n.statements()),
        Node::WhileNode { .. } => guard.as_while_node().and_then(|n| n.statements()),
        Node::UntilNode { .. } => guard.as_until_node().and_then(|n| n.statements()),
        _ => None,
    };
    body.is_some_and(|b| {
        b.body().len() == 1
            && b.body().first().is_some_and(|n| {
                n.location().start_offset() == node.location().start_offset()
                    && n.location().end_offset() == node.location().end_offset()
            })
    })
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
            || (matches!(parent, Node::WhileNode { .. } | Node::UntilNode { .. })
                && child.as_statements_node().is_some())
        {
            return true;
        } else if matches!(
            parent,
            Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::ElseNode { .. }
                | Node::BeginNode { .. }
        ) && matches!(child, Node::StatementsNode { .. } | Node::ElseNode { .. })
        { /* value propagates */
        } else if !modifier(parent)
            .is_some_and(|(k, _)| child.location().end_offset() <= k.start_offset())
        {
            return false;
        }
        child = parent;
    }
    false
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
    fn reorders_only_infallible_already_bound_local_reads() {
        crate::cops::testing::assert_correction(
            &ParallelAssignment,
            "a = 4\na,b=1,a\n",
            "a = 4\nb = a\na = 1\n",
        );
        crate::cops::testing::assert_correction(&ParallelAssignment, "a,b=a,b\n", "a = a\nb = b\n");
        for source in [
            "a,b=1,a\n",
            "a,b=b,1\n",
            "a,b=1,a if a\n",
            "a,b=1,2 if b\n",
            "a,b=1,2 if (c=3)\n",
            "if true\n a,b=1,2\n more\nend if foo\n",
            "a,b=1,2 if foo do\n true\nend\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&ParallelAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn consumed_results_and_effectful_or_observable_writes_are_held() {
        for source in [
            "def f; a,b=1,2; end\n",
            "f { a,b=1,2 }\n",
            "x=(a,b=1,2)\n",
            "p((a,b=1,2))\n",
            "a,b=foo(),bar()\n",
            "a,b=1,raise\n",
            "a,b=1,C\n",
            "a,b=1,@a\n",
            "a,b=1,$a\n",
            "a,b=1,@@a\n",
            "@a,@b=1,2\n",
            "@@a,@@b=1,2\n",
            "$a,$b=1,2\n",
            "A,B=1,2\n",
            "C::A,C::B=1,2\n",
            "o.a,o.b=1,2\n",
            "o[0],o[1]=1,2\n",
            "a,b=1,2 rescue nil\n",
            "a,b={key=>1},2\n",
            "key=Object.new; a,b={key=>1},2\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&ParallelAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
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
    fn identical_assignment_pairs_coalesce_numeric_and_evaluated_line_values() {
        for (source, line) in [
            ("a,a=[a,1r],[a,1.0r]\n", 1),
            ("a,a=[a,__LINE__],[a,1]\n", 1),
            ("o[0].a,o.[](0).a=[o[0].a,1],[o.[](0).a,1]\n", 1),
            ("a,a=[a,-0.0],[a,0.0]\n", 1),
            ("a,a=[a,o[0]],[a,o.[](0)]\n", 1),
            ("\na,a=[a,__LINE__],[a,2]\n", 2),
        ] {
            let found = crate::cops::testing::correction_offenses(&ParallelAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, 1), "{source}");
            assert_eq!(found[0].message, "Do not use parallel assignment.");
            assert!(found[0].correction.is_none(), "{source}");
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
    fn duplicate_targets_keep_the_winning_store_and_getter_normalization_is_root_only() {
        for source in [
            "a,_=1,2\n",
            "a,a=1,a\n",
            "a,a=a,a\n",
            "self.a,self.b=b.foo,a\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&ParallelAssignment, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn work_is_lazy_and_bounded_for_wide_assignments_and_siblings() {
        let source = "a,b=1,2\n".repeat(2000);
        REWRITE_BYTES.with(|v| v.set(0));
        READ_VISITS.with(|v| v.set(0));
        assert_eq!(offenses(&source).len(), 2000);
        assert_eq!(REWRITE_BYTES.with(|v| v.get()), 0);
        assert_eq!(READ_VISITS.with(|v| v.get()), 4000);
        let fixed = crate::cops::testing::correction_offenses(&ParallelAssignment, &source);
        assert!(fixed.iter().all(|o| o.correction.is_some()));
        assert!(REWRITE_BYTES.with(|v| v.get()) <= source.len());
        // Thousands of writes to the same target must not construct a dense
        // reader×writer graph. Distinct RHS pairs keep the test nontrivial.
        let wide = format!(
            "{}={}\n",
            vec!["a"; 3000].join(","),
            (0..3000)
                .map(|i| format!("[a,{i}]"))
                .collect::<Vec<_>>()
                .join(",")
        );
        EDGES.with(|v| v.set(0));
        assert!(offenses(&wide).is_empty()); // a genuine dependency cycle
        assert!(EDGES.with(|v| v.get()) < 12000);
        // Acyclic many-readers/many-writers fan-out must actually build and
        // consume the compressed graph, not merely hit the early cycle exit.
        let targets = std::iter::repeat_n("a".to_owned(), 1500)
            .chain((0..1500).map(|i| format!("b{i}")))
            .collect::<Vec<_>>()
            .join(",");
        let values = (0..1500)
            .map(|i| i.to_string())
            .chain((0..1500).map(|i| format!("[a,{i}]")))
            .collect::<Vec<_>>()
            .join(",");
        EDGES.with(|v| v.set(0));
        assert_eq!(offenses(&format!("{targets}={values}\n")).len(), 1);
        assert_eq!(EDGES.with(|v| v.get()), 3000);
        let huge = format!("a,b='{}',2\n", "x".repeat(20000));
        REWRITE_BYTES.with(|v| v.set(0));
        let found = crate::cops::testing::correction_offenses(&ParallelAssignment, &huge);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        assert_eq!(REWRITE_BYTES.with(|v| v.get()), 0);
        let huge_predicate = format!("a,b=1,2 if foo('{}')\n", "x".repeat(20000));
        let found = crate::cops::testing::correction_offenses(&ParallelAssignment, &huge_predicate);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn nested_and_named_rest_targets_use_upstream_flattened_arity_but_never_expand() {
        for s in [
            "a,*b=1,2\n",
            "*a,b=1,2\n",
            "(a,b),c=1,2,3\n",
            "a,(b,c)=1,2,3\n",
            "self.a,self.a=a,self.a\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&ParallelAssignment, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
        for s in ["(a,b),c=[1,2],3\n", "a,*,b=1,2\n", "a,*b=1,2,3\n"] {
            assert!(offenses(s).is_empty(), "{s}");
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
