//! `Style/HashEachMethods` — unsafe, fixed-policy Hash iteration suggestions.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::{ReferenceKind, ScopeAnalysis},
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockNode, CallNode, Node};
use std::{collections::HashMap, ops::Range};
pub struct HashEachMethods;
impl Cop for HashEachMethods {
    fn name(&self) -> &'static str {
        "Style/HashEachMethods"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"each"
            || call.arguments().is_some_and(|a| !a.arguments().is_empty())
        {
            return;
        }
        let Some(block) = call.block() else { return };
        let literal = block.as_block_node();
        let symbol_pass = block
            .as_block_argument_node()
            .and_then(|b| b.expression())
            .is_some_and(|n| n.as_symbol_node().is_some());
        if literal.is_none() && !symbol_pass {
            return;
        }
        let kv = call.receiver().and_then(|r| r.as_call_node()).filter(|r| {
            matches!(r.name().as_slice(), b"keys" | b"values")
                && ast::call_arguments(r).is_empty()
                && r.block().is_none()
        });
        let args = literal.as_ref().and_then(two_arguments);
        let kv = kv.filter(|kv| kv.receiver().is_some_and(|r| !allowed_receiver(&r)));
        if kv.is_none() && args.is_none() {
            return;
        }
        if literal.is_some()
            && !file
                .hash_each_facts(cx.ancestors().next_back().unwrap())
                .handleable(&call)
        {
            return;
        }
        if let Some(kv) = kv {
            let start = kv.message_loc().unwrap().start_offset();
            let end = call.message_loc().unwrap().end_offset();
            // The literal-block matcher includes explicit parentheses in the
            // message, but its diagnostic ends at the outer selector.
            let current_end = if symbol_pass {
                end
            } else {
                ast::source_end(node)
            };
            let current = String::from_utf8_lossy(&file.bytes[start..current_end]);
            let prefer = if kv.name().as_slice() == b"keys" {
                "each_key"
            } else {
                "each_value"
            };
            // correct_key_value_each: `hash.each_key`, keeping the dot.
            let correction = if symbol_pass {
                Some(Correction {
                    range: start..end,
                    replacement: prefer.into(),
                })
            } else {
                kv.receiver()
                    .zip(call.call_operator_loc())
                    .map(|(hash, dot)| Correction {
                        range: node.location().start_offset()..ast::source_end(node),
                        replacement: format!(
                            "{}{}{prefer}",
                            String::from_utf8_lossy(hash.location().as_slice()),
                            String::from_utf8_lossy(dot.as_slice())
                        ),
                    })
            };
            report(
                self,
                file,
                start..end,
                format!("Use `{prefer}` instead of `{current}`."),
                correction.filter(|_| cx.corrections_requested()),
                out,
            );
            return;
        }
        let Some(block) = literal else { return };
        let Some(body) = block.body() else { return };
        let Some([key, value]) = args else { return };
        let facts = file.hash_each_facts(cx.ancestors().next_back().unwrap());
        if !facts.handleable(&call) {
            return;
        }
        let used_key = parameter_used(&key, &body, facts);
        let used_value = parameter_used(&value, &body, facts);
        if used_key == used_value {
            return;
        }
        let (prefer, unused, removal) = if used_key {
            (
                "each_key",
                &value,
                key.location().end_offset()..value.location().end_offset(),
            )
        } else {
            (
                "each_value",
                &key,
                key.location().start_offset()..value.location().start_offset(),
            )
        };
        let unused = String::from_utf8_lossy(unused.location().as_slice());
        let correction = call
            .message_loc()
            .filter(|_| cx.corrections_requested())
            .and_then(|selector| {
                Correction::from_edits(
                    &file.bytes,
                    vec![
                        (
                            selector.start_offset()..selector.end_offset(),
                            prefer.into(),
                        ),
                        (removal, String::new()),
                    ],
                )
            });
        report(
            self,
            file,
            call.location().start_offset()..call.location().end_offset(),
            format!(
                "Use `{prefer}` instead of `each` and remove the unused `{unused}` block argument."
            ),
            correction,
            out,
        );
    }
}

fn two_arguments<'pr>(block: &BlockNode<'pr>) -> Option<[Node<'pr>; 2]> {
    let params = block.parameters()?.as_block_parameters_node()?;
    let mut args = Vec::new();
    if let Some(p) = params.parameters() {
        args.extend(
            p.requireds()
                .iter()
                .chain(p.optionals().iter())
                // A trailing comma is an implicit rest, not a rest argument.
                .chain(p.rest().filter(|n| n.as_implicit_rest_node().is_none()))
                .chain(p.posts().iter())
                .chain(p.keywords().iter())
                .chain(p.keyword_rest())
                .chain(p.block().map(|b| b.as_node())),
        );
    }
    // shadowarg block locals count even without ordinary parameters.
    args.extend(params.locals().iter());
    args.try_into().ok()
}

fn report(
    cop: &HashEachMethods,
    file: &SourceFile,
    range: Range<usize>,
    message: String,
    correction: Option<Correction>,
    out: &mut Vec<Offense>,
) {
    let (line, col) = file.line_col(range.start);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message,
        line,
        col,
        end_line: file.line_of(range.end.saturating_sub(1)),
        correction,
    });
}

// Pinned AllowedReceivers compares a name assembled up to the first constant
// receiver, not the exact source and not resolved constant/variable identities.
fn allowed_receiver(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    let Some(receiver) = call.receiver() else {
        return false;
    };
    if !ast::is_const(&receiver) {
        return allowed_receiver(&receiver);
    }
    !call.is_safe_navigation()
        && !call.block().is_some_and(|b| b.as_block_node().is_some())
        && call.name().as_slice() == b"current"
        && receiver.location().as_slice() == b"Thread"
}

fn parameter_used(param: &Node<'_>, body: &Node<'_>, facts: &Facts) -> bool {
    if let Some(multi) = param.as_multi_target_node() {
        return multi
            .lefts()
            .iter()
            .chain(multi.rest())
            .chain(multi.rights().iter())
            .any(|n| parameter_used(&n, body, facts));
    }
    if let Some(splat) = param.as_splat_node() {
        return splat
            .expression()
            .is_some_and(|n| parameter_used(&n, body, facts));
    }
    if body.as_statements_node().is_some_and(|s| {
        s.body().len() == 1
            && s.body()
                .iter()
                .next()
                .is_some_and(|n| n.as_local_variable_read_node().is_some())
    }) {
        return false;
    }
    let name = param.location().as_slice();
    let name = name.strip_prefix(b"*").unwrap_or(name);
    let Some(reads) = facts.reads.get(name) else {
        return false;
    };
    let Some(subtree) = facts.bodies.get(&source_key(body)) else {
        return false;
    };
    let i = reads.partition_point(|&p| p < subtree.start);
    reads.get(i).is_some_and(|&p| p < subtree.end)
}

fn source_key(node: &Node<'_>) -> (usize, usize) {
    (node.location().start_offset(), node.location().end_offset())
}

/// Lazy main-parse indexes. One complete walk computes receiver roots bottom-up;
/// mutation/read preorder positions are indexed once, never rescanned per block.
/// Subtree intervals follow AST edges, not byte containment: heredoc descendants
/// can occur outside the source span of their enclosing block or body. The
/// source-name lookup deliberately preserves the pinned cop's lexical policy:
/// even reads of shadowed identities count, but writes/binding exposure do not.
#[derive(Default)]
pub(crate) struct Facts {
    handled: HashMap<(usize, usize), bool>,
    reads: HashMap<Vec<u8>, Vec<usize>>,
    bodies: HashMap<(usize, usize), Range<usize>>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    fn handleable(&self, call: &CallNode<'_>) -> bool {
        self.handled
            .get(&(call.location().start_offset(), call.location().end_offset()))
            .copied()
            .unwrap_or(false)
    }
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Builder<'pr> {
            stack: Vec<(Node<'pr>, usize)>,
            subtrees: HashMap<(usize, usize, std::mem::Discriminant<Node<'pr>>), Range<usize>>,
            read_positions: HashMap<(usize, usize), usize>,
            roots: Vec<Node<'pr>>,
            root_ids: HashMap<(usize, usize), usize>,
            calls: HashMap<(usize, usize), usize>,
            candidates: Vec<(CallNode<'pr>, Option<usize>, Range<usize>)>,
            mutations: Vec<(Node<'pr>, usize)>,
            visits: usize,
            needs_reads: bool,
        }
        fn key<'pr>(n: &Node<'pr>) -> (usize, usize, std::mem::Discriminant<Node<'pr>>) {
            let (start, end) = source_key(n);
            (start, end, std::mem::discriminant(n))
        }
        impl<'pr> NodeVisitor<'pr> for Builder<'pr> {
            fn enter(&mut self, n: Node<'pr>) {
                if n.as_local_variable_read_node().is_some() {
                    self.read_positions.insert(source_key(&n), self.visits);
                }
                self.stack.push((n, self.visits));
                self.visits += 1;
            }
            fn leave(&mut self) {
                let (node, position) = self.stack.pop().unwrap();
                self.subtrees.insert(key(&node), position..self.visits);
                if let Some(target) = node.as_index_target_node()
                    && !target.is_safe_navigation()
                {
                    self.mutations.push((target.receiver(), position));
                }
                let Some(call) = node.as_call_node() else {
                    return;
                };
                let root_id = call.receiver().map(|receiver| {
                    let k = source_key(&receiver);
                    if let Some(&id) = self.calls.get(&k) {
                        return id;
                    }
                    *self.root_ids.entry(k).or_insert_with(|| {
                        let id = self.roots.len();
                        self.roots.push(receiver);
                        id
                    })
                });
                if let Some(id) = root_id {
                    self.calls.insert(source_key(&node), id);
                }
                if call.name().as_slice() == b"[]="
                    && !call.is_safe_navigation()
                    && let Some(r) = call.receiver()
                {
                    self.mutations.push((r, position));
                }
                if call.name().as_slice() == b"each"
                    && call.block().is_some_and(|b| b.as_block_node().is_some())
                {
                    self.needs_reads |= call
                        .block()
                        .and_then(|b| b.as_block_node())
                        .is_some_and(|b| b.body().is_some() && two_arguments(&b).is_some());
                    self.candidates.push((call, root_id, position..self.visits));
                }
            }
        }
        let mut b = Builder::default();
        walk(&mut b, root);
        let mut out = Self::default();
        #[cfg(test)]
        {
            out.visits = b.visits;
        }
        let mut mutations: HashMap<ast::Fingerprint<'_>, Vec<usize>> = HashMap::new();
        for (receiver, pos) in b.mutations {
            mutations
                .entry(ast::fingerprint(&receiver))
                .or_default()
                .push(pos);
        }
        for positions in mutations.values_mut() {
            positions.sort_unstable();
        }
        let mut root_mutations = Vec::new();
        for root in &b.roots {
            root_mutations.push(if mutations.is_empty() {
                None
            } else {
                mutations.get(&ast::fingerprint(root))
            });
        }
        for (call, root_id, subtree) in b.candidates {
            if let Some(body) = call
                .block()
                .and_then(|n| n.as_block_node())
                .and_then(|n| n.body())
            {
                out.bodies
                    .insert(source_key(&body), b.subtrees[&key(&body)].clone());
            }
            let converter = call
                .receiver()
                .and_then(|r| r.as_call_node())
                .is_some_and(|r| {
                    matches!(
                        r.name().as_slice(),
                        b"assoc"
                            | b"chunk"
                            | b"flatten"
                            | b"rassoc"
                            | b"sort"
                            | b"sort_by"
                            | b"to_a"
                    )
                });
            let handled = !converter
                && root_id.is_some_and(|id| {
                    let root = &b.roots[id];
                    let literal_ok = !ast::is_literal(root) || root.as_hash_node().is_some();
                    let mutated = root_mutations[id].is_some_and(|positions| {
                        let i = positions.partition_point(|&p| p < subtree.start);
                        positions.get(i).is_some_and(|&p| p < subtree.end)
                    });
                    literal_ok && !mutated
                });
            out.handled.insert(
                (call.location().start_offset(), call.location().end_offset()),
                handled,
            );
        }
        if b.needs_reads
            && let Some(program) = root.as_program_node()
        {
            let analysis = ScopeAnalysis::of(&program);
            for scope in analysis.scopes_in_exit_order() {
                for id in analysis.scope(scope).variables() {
                    let variable = analysis.variable(id);
                    for &reference in &variable.references {
                        let r = analysis.reference(reference);
                        if r.kind == ReferenceKind::Read
                            && let Some(&position) =
                                b.read_positions.get(&(r.range.start, r.range.end))
                        {
                            out.reads
                                .entry(variable.name.to_vec())
                                .or_default()
                                .push(position);
                        }
                    }
                }
            }
            for reads in out.reads.values_mut() {
                reads.sort_unstable();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/hash_each_methods_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/HashEachMethods")
            .collect()
    }
    fn one(s: &str, line: usize, col: usize, message: &str) {
        let o = offenses(s);
        assert_eq!(o.len(), 1, "{s}: {o:?}");
        assert_eq!((o[0].line as usize, o[0].col as usize), (line, col), "{s}");
        assert_eq!(o[0].message, message, "{s}");
        assert!(o[0].correction.is_none());
    }
    #[test]
    fn keys_values_and_navigation_exact_ranges() {
        for receiver in ["foo", "{}", "{hash: :literal}"] {
            for (method, prefer) in [("keys", "each_key"), ("values", "each_value")] {
                for dot in [".", "&."] {
                    for block in [" { |k| p k }", " { p _1 }", " { p it }", "(&:bar)"] {
                        let s = format!("  {receiver}{dot}{method}{dot}each{block}");
                        one(
                            &s,
                            1,
                            3 + receiver.len() + dot.len(),
                            &format!("Use `{prefer}` instead of `{method}{dot}each`."),
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn unused_parameters_and_destructuring() {
        for (params, body, unused, prefer) in [
            ("k, unused_value", "p k", "unused_value", "each_key"),
            ("unused_key, v", "p v", "unused_key", "each_value"),
            (
                "k, (_, unused_value)",
                "p k",
                "(_, unused_value)",
                "each_key",
            ),
            ("(_, unused_key), v", "p v", "(_, unused_key)", "each_value"),
            ("k, *v", "p(*v)", "k", "each_value"),
            ("*k, v", "p(*k)", "v", "each_key"),
        ] {
            for dot in [".", "&."] {
                one(
                    &format!("\n  foo{dot}each {{ |{params}| {body} }}"),
                    2,
                    3,
                    &format!(
                        "Use `{prefer}` instead of `each` and remove the unused `{unused}` block argument."
                    ),
                );
            }
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for s in [
            "foo.each { |k, v| p(k, v) }",
            "foo.each { |k, (_, v)| p(k, v) }",
            "foo.each { |(_, k), v| p(k, v) }",
            "foo.each { |k, (_, *v)| p(k, *v) }",
            "foo.each { |(_, *k), v| p(*k, v) }",
            "foo.each { |e| p e }",
            "foo.each { |(k,v)| p e }",
            "foo.each { |k,*v| p(k,*v) }",
            "foo.each { |*k,v| p(*k,v) }",
            "foo.each { |k,v| p 1 }",
            "foo.each { |k,v| }",
            "[[1,2],[3,4]].each { |k,v| p k }",
            "foo.each_key { |k| p k }",
            "{}.each_value { |v| p v }",
            "keys.each { |k| p k }",
            "values.each(&:bar)",
            "each { |k,v| p k }",
            "foo.each_pair { |k,v| p k }",
            "foo.keys.each",
            "foo.values.each(&callback)",
            "foo.keys.each(&nil)",
            "foo.keys.each(1) { p it }",
            "foo.keys(1).each { p it }",
            "foo.keys.each(k: 1) { p it }",
            "foo.keys.each({k: 1}) { p it }",
            "foo.each(1) { |k,v| p k }",
            "foo.each { p _2 }",
            "foo.each { p it }",
            "foo.each { |k,v,w| p k }",
            "foo.each { |k,v; local| p k }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn converters_and_literal_roots() {
        for method in [
            "assoc(key)",
            "chunk { |i| i.foo }",
            "flatten",
            "rassoc(v)",
            "sort",
            "sort_by { _2 }",
            "to_a",
        ] {
            assert!(
                offenses(&format!("foo.{method}.each {{ |k,v| p v }}")).is_empty(),
                "{method}"
            );
        }
        for root in ["[]", "42", "'str'", ":sym", "nil", "true", "/x/", "1..2"] {
            assert!(
                offenses(&format!("{root}.each {{ |k,v| p k }}")).is_empty(),
                "{root}"
            );
        }
    }
    #[test]
    fn mutation_exemption_and_distinct_receiver() {
        for method in ["keys", "values"] {
            assert!(
                offenses(&format!("foo.{method}.each {{ |key| foo[key] = value }}")).is_empty()
            );
            one(
                &format!("foo.{method}.each {{ |key| bar[key] = value }}"),
                1,
                5,
                &format!(
                    "Use `each_{}` instead of `{method}.each`.",
                    &method[..method.len() - 1]
                ),
            );
        }
        assert!(offenses("foo.each { |k,v| foo[k] = k }").is_empty());
    }
    #[test]
    fn kv_exemptions_fall_through_to_unused_parameters() {
        for receiver in [
            "Thread.current.keys",
            "Thread.current.values",
            "keys",
            "values",
        ] {
            one(
                &format!("{receiver}.each {{ |k,v| p k }}"),
                1,
                1,
                "Use `each_key` instead of `each` and remove the unused `v` block argument.",
            );
        }
        one(
            "Thread.current { p 1 }.keys.each { p it }",
            1,
            24,
            "Use `each_key` instead of `keys.each`.",
        );
    }
    #[test]
    fn body_itself_is_not_a_descendant_and_multiple_index_targets_mutate() {
        for s in [
            "foo.each { |k,v| k }",
            "foo.each { |k,v| v }",
            "foo.keys.each { |k| foo[k],x = values; p k }",
            "foo.each { |k,v| foo[k],x = values; p k }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn chained_blocks_have_independent_handling_ranges() {
        let s = "foo.to_a.each { |k,v| p k }.each { |k,v| p k }";
        one(
            s,
            1,
            1,
            "Use `each_key` instead of `each` and remove the unused `v` block argument.",
        );
        assert_eq!(offenses(s)[0].end_line, 1);
    }
    #[test]
    fn closures_shadows_and_closed_scopes_follow_pinned_source_name_policy() {
        for body in [
            "p k",
            "proc { p k }",
            "proc { |k| p k }",
            "def m; k=1; p k; end",
            "k=1; p k",
            "p({k:})",
            "p(\"#{k}\")",
            "def m(k); k; end",
            "k = v; p k",
        ] {
            let s = format!("foo.each {{ |k,v| {body} }}");
            if body == "k = v; p k" {
                assert!(offenses(&s).is_empty());
            } else {
                one(
                    &s,
                    1,
                    1,
                    "Use `each_key` instead of `each` and remove the unused `v` block argument.",
                );
            }
        }
        for body in ["binding", "k=1", "k += 1", "p(\"k\")"] {
            assert!(
                offenses(&format!("foo.each {{ |k,v| {body} }}")).is_empty(),
                "{body}"
            );
        }
    }
    #[test]
    fn optional_keyword_and_block_parameters_are_matched_by_source_not_value_flow() {
        for unused in ["v=1", "v:", "v: 1", "**v", "&v", "*"] {
            let s = format!("foo.each {{ |k,{unused}| p(k, v) }}");
            one(
                &s,
                1,
                1,
                &format!(
                    "Use `each_key` instead of `each` and remove the unused `{unused}` block argument."
                ),
            );
        }
        for params in ["(k,x),v", "k,(*x,v)", "k,v; local", "k,v,w", "(k,v)"] {
            let s = format!("foo.each {{ |{params}| p(k,v) }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
    #[test]
    fn symbol_pass_has_its_own_literal_and_converter_policy() {
        for receiver in ["[]", "42", "foo.to_a", "foo.sort_by { _2 }"] {
            let s = format!("{receiver}.keys.each(&:bar)");
            one(
                &s,
                1,
                receiver.len() + 2,
                "Use `each_key` instead of `keys.each`.",
            );
        }
        for receiver in [
            "Thread.current(1)",
            "Thread.current.foo",
            "Thread.current.foo { p it }",
        ] {
            assert!(offenses(&format!("{receiver}.keys.each(&:bar)")).is_empty());
        }
    }
    #[test]
    fn control_flow_reads_are_syntactic_and_do_not_include_implicit_operator_reads() {
        for body in [
            "defined?(k)",
            "(k)",
            "begin k end",
            "next k",
            "break k",
            "return k",
            "k; k",
        ] {
            one(
                &format!("foo.each {{ |k,v| {body} }}"),
                1,
                1,
                "Use `each_key` instead of `each` and remove the unused `v` block argument.",
            );
        }
        one(
            "foo.each { |k,v| k += v }",
            1,
            1,
            "Use `each_value` instead of `each` and remove the unused `k` block argument.",
        );
        one(
            "foo.each { |k;v| p k }",
            1,
            1,
            "Use `each_key` instead of `each` and remove the unused `v` block argument.",
        );
    }
    #[test]
    fn heredoc_descendants_count_as_reads_and_mutations() {
        for (read, prefer, unused) in [("k", "each_key", "v"), ("v", "each_value", "k")] {
            one(
                &format!("h.each {{ |k,v| puts <<~TEXT }}\n#{{{read}}}\nTEXT\n"),
                1,
                1,
                &format!(
                    "Use `{prefer}` instead of `each` and remove the unused `{unused}` block argument."
                ),
            );
        }
        for body in ["h[k] = 1", "h[k], x = values"] {
            let s = format!("h.keys.each {{ |k| puts <<~TEXT }}\n#{{{body}}}\nTEXT\n");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        // Heredoc work in siblings must not leak into this block's subtree.
        one(
            "h.keys.each { |k| puts <<~TEXT }; h[k] = 1\n#{k}\nTEXT\n",
            1,
            3,
            "Use `each_key` instead of `keys.each`.",
        );
        assert!(offenses("h.each { |k,v| puts <<~TEXT }; p k\nplain\nTEXT\n").is_empty());
    }
    #[test]
    fn trailing_commas_are_not_parameters() {
        assert!(offenses("h.each { |k,| p k }").is_empty());
        for (body, prefer, unused) in [("p k", "each_key", "v"), ("p v", "each_value", "k")] {
            one(
                &format!("h.each {{ |k,v,| {body} }}"),
                1,
                1,
                &format!(
                    "Use `{prefer}` instead of `each` and remove the unused `{unused}` block argument."
                ),
            );
        }
        assert!(offenses("h.each { |k,v,| p k,v }").is_empty());
    }
    #[test]
    fn shadow_only_parameters_follow_pinned_two_argument_policy() {
        for (body, prefer, unused) in [("p k", "each_key", "v"), ("p v", "each_value", "k")] {
            one(
                &format!("h.each {{ |;k,v| {body} }}"),
                1,
                1,
                &format!(
                    "Use `{prefer}` instead of `each` and remove the unused `{unused}` block argument."
                ),
            );
        }
        for s in [
            "h.each { |;k,v| p k,v }",
            "h.each { |;k| p k }",
            "h.each { |;k,v,x| p k }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn fixed_allowed_receiver() {
        assert!(offenses("Thread.current.keys.each { |k| p k }").is_empty());
        assert!(offenses("Thread.current.values.each(&:bar)").is_empty());
        one(
            "execute(sql).values.each { p it }",
            1,
            14,
            "Use `each_value` instead of `values.each`.",
        );
        // AllowedReceivers applies to the keys/values forms, not unused args.
        one(
            "Thread.current.each { |k,v| p k }",
            1,
            1,
            "Use `each_key` instead of `each` and remove the unused `v` block argument.",
        );
    }
}
