//! `Style/EachWithObject` — mutable accumulators returned by inject/reduce.
use super::correction::{combine, contains_comment, touches_directive};
use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::{AssignmentKind, ScopeAnalysis},
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockNode, Node};
use std::{collections::HashMap, ops::Range};
pub struct EachWithObject;
impl Cop for EachWithObject {
    fn name(&self) -> &'static str {
        "Style/EachWithObject"
    }
    fn safe_fixes(&self) -> bool {
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
        if !matches!(call.name().as_slice(), b"inject" | b"reduce") {
            return;
        }
        let args = ast::call_arguments(&call);
        let [initial] = args.as_slice() else { return };
        if ast::is_basic_literal(initial) {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        let Some(body) = block.body().and_then(|b| b.as_statements_node()) else {
            return;
        };
        let Some(mut last) = body.body().iter().last() else {
            return;
        };
        // A single grouped body is RuboCop's begin node. Peel exactly its
        // last child, not arbitrary nested grouping or a grouped final sibling.
        if body.body().len() == 1
            && let Some(paren) = last.as_parentheses_node()
        {
            let Some(group) = paren.body().and_then(|n| n.as_statements_node()) else {
                return;
            };
            let Some(value) = group.body().iter().last() else {
                return;
            };
            last = value;
        }
        let Some(returned) = last.as_local_variable_read_node() else {
            return;
        };
        let numbered = block
            .parameters()
            .and_then(|p| p.as_numbered_parameters_node())
            .is_some_and(|p| p.maximum() == 2);
        let params = explicit_arguments(&block);
        let accumulator = if numbered {
            b"_1".as_slice()
        } else {
            let Some(p) = &params else { return };
            let Some(name) = parameter_name(&p[0]) else {
                return;
            };
            name
        };
        if returned.name().as_slice() != accumulator {
            return;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        let facts = file.each_with_object_facts(root);
        let block_loc = block.location();
        let Some(span) = facts
            .spans
            .get(&(block_loc.start_offset(), block_loc.end_offset()))
        else {
            return;
        };
        let body_loc = body.location();
        let Some(body_span) = facts
            .spans
            .get(&(body_loc.start_offset(), body_loc.end_offset()))
        else {
            return;
        };
        // RuboCop checks the body's descendants, not parameter defaults.
        if !numbered && facts.assigned(accumulator, body_span) {
            return;
        }
        let selector = call.message_loc().unwrap();
        let (line, col) = file.line_col(selector.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Unknown/lazy/custom enumerators can change arity, return value,
                // visibility and evaluation. Keep their diagnostics, not their edits.
                if !call.receiver().is_some_and(|r| r.as_array_node().is_some())
                    || !(initial.as_hash_node().is_some() || initial.as_array_node().is_some())
                    || facts.custom_dispatch
                    || facts.risky(span)
                    || contains_comment(
                        file,
                        block.location().start_offset()..block.location().end_offset(),
                    )
                    || touches_directive(
                        file,
                        call.location().start_offset()..call.location().end_offset(),
                    )
                {
                    return None;
                }
                let mut edits = vec![Correction {
                    range: selector.start_offset()..selector.end_offset(),
                    replacement: "each_with_object".into(),
                }];
                if numbered {
                    // Nested blocks are individually withheld: numbered identities
                    // are scoped, and swapping by spelling across them is unsafe.
                    if Facts::has(&facts.nested_blocks, &(span.start + 1..span.end)) {
                        return None;
                    }
                    for name in [b"_1".as_slice(), b"_2".as_slice()] {
                        if let Some(reads) = facts.numbered_reads.get(name) {
                            let first = reads.partition_point(|(p, _)| *p < span.start);
                            for (_, range) in
                                reads[first..].iter().take_while(|(p, _)| *p < span.end)
                            {
                                edits.push(Correction {
                                    range: range.clone(),
                                    replacement: if name == b"_1" { "_2" } else { "_1" }.into(),
                                });
                            }
                        }
                    }
                } else {
                    let params = params.as_ref()?;
                    // Only ordinary two-required-argument headers are safely swappable.
                    let p = block.parameters()?.as_block_parameters_node()?;
                    let ordinary = p.parameters()?;
                    if ordinary.requireds().len() != 2
                        || !p.locals().is_empty()
                        // Ruby allows duplicate underscore-prefixed parameters;
                        // swapping their spelling cannot swap their bindings.
                        || parameter_name(&params[0]) == parameter_name(&params[1])
                        || params
                            .iter()
                            .any(|p| p.as_required_parameter_node().is_none())
                    {
                        return None;
                    }
                    edits.push(Correction {
                        range: params[0].location().start_offset()
                            ..params[0].location().end_offset(),
                        replacement: std::str::from_utf8(params[1].location().as_slice())
                            .ok()?
                            .into(),
                    });
                    edits.push(Correction {
                        range: params[1].location().start_offset()
                            ..params[1].location().end_offset(),
                        replacement: std::str::from_utf8(params[0].location().as_slice())
                            .ok()?
                            .into(),
                    });
                    let start = file.line_start(file.line_of(last.location().start_offset()));
                    let end = file.line_start(
                        file.line_of(last.location().end_offset().saturating_sub(1)) + 1,
                    );
                    let whole_line = file.bytes[start..end]
                        .iter()
                        .filter(|b| !b.is_ascii_whitespace())
                        .copied()
                        .eq(last.location().as_slice().iter().copied());
                    let range = if whole_line {
                        start..end
                    } else {
                        last.location().start_offset()..last.location().end_offset()
                    };
                    edits.push(Correction {
                        range,
                        replacement: String::new(),
                    });
                }
                let edit = combine(file, edits)?;
                Some(edit)
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `each_with_object` instead of `{}`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}
fn explicit_arguments<'pr>(block: &BlockNode<'pr>) -> Option<[Node<'pr>; 2]> {
    let p = block.parameters()?.as_block_parameters_node()?;
    let mut args = Vec::new();
    if let Some(p) = p.parameters() {
        args.extend(
            p.requireds()
                .iter()
                .chain(p.optionals().iter())
                .chain(p.rest().filter(|n| n.as_implicit_rest_node().is_none()))
                .chain(p.posts().iter())
                .chain(p.keywords().iter())
                .chain(p.keyword_rest())
                .chain(p.block().map(|b| b.as_node())),
        );
    }
    args.extend(p.locals().iter());
    args.try_into().ok()
}
fn parameter_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    if let Some(n) = node.as_required_parameter_node() {
        return Some(n.name().as_slice());
    }
    if let Some(n) = node.as_optional_parameter_node() {
        return Some(n.name().as_slice());
    }
    if let Some(n) = node.as_rest_parameter_node() {
        return n.name().map(|n| n.as_slice());
    }
    if let Some(n) = node.as_block_parameter_node() {
        return n.name().map(|n| n.as_slice());
    }
    if let Some(n) = node.as_required_keyword_parameter_node() {
        return Some(n.name().as_slice());
    }
    if let Some(n) = node.as_optional_keyword_parameter_node() {
        return Some(n.name().as_slice());
    }
    if let Some(n) = node.as_keyword_rest_parameter_node() {
        return n.name().map(|n| n.as_slice());
    }
    node.as_block_local_variable_node()
        .map(|n| n.name().as_slice())
}

/// One lazy main-AST index, shared with EachForSimpleLoop's correction proof.
/// AST preorder intervals handle heredocs outside their parent's byte span.
#[derive(Default)]
pub(crate) struct Facts {
    spans: HashMap<(usize, usize), Range<usize>>,
    writes: HashMap<Vec<u8>, Vec<usize>>,
    risks: Vec<usize>,
    nested_blocks: Vec<usize>,
    numbered_reads: HashMap<Vec<u8>, Vec<(usize, Range<usize>)>>,
    pub(super) custom_dispatch: bool,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    fn has(positions: &[usize], span: &Range<usize>) -> bool {
        let i = positions.partition_point(|p| *p < span.start);
        positions.get(i).is_some_and(|p| *p < span.end)
    }
    fn assigned(&self, name: &[u8], span: &Range<usize>) -> bool {
        self.writes.get(name).is_some_and(|p| Self::has(p, span))
    }
    fn risky(&self, span: &Range<usize>) -> bool {
        Self::has(&self.risks, span)
    }
    pub(crate) fn build(root: &Node<'_>) -> Self {
        struct Builder {
            facts: Facts,
            stack: Vec<((usize, usize), usize, bool)>,
            positions: HashMap<(usize, usize), usize>,
            count: usize,
            needs_assignments: bool,
        }
        impl<'pr> NodeVisitor<'pr> for Builder {
            fn enter(&mut self, node: Node<'pr>) {
                let p = self.count;
                self.count += 1;
                let loc = node.location();
                self.positions
                    .insert((loc.start_offset(), loc.end_offset()), p);
                self.stack.push((
                    (loc.start_offset(), loc.end_offset()),
                    p,
                    matches!(node, Node::BlockNode { .. } | Node::StatementsNode { .. }),
                ));
                let methods: &[&[u8]] =
                    &[b"each", b"times", b"inject", b"reduce", b"each_with_object"];
                self.facts.custom_dispatch |= node
                    .as_def_node()
                    .is_some_and(|d| methods.contains(&d.name().as_slice()))
                    || node.as_alias_method_node().is_some()
                    || node.as_undef_node().is_some();
                if matches!(node, Node::BlockNode { .. } | Node::LambdaNode { .. }) {
                    self.facts.nested_blocks.push(p);
                }
                if matches!(
                    node,
                    Node::NextNode { .. }
                        | Node::BreakNode { .. }
                        | Node::ReturnNode { .. }
                        | Node::RedoNode { .. }
                        | Node::RetryNode { .. }
                        | Node::DefNode { .. }
                        | Node::RescueNode { .. }
                        | Node::EnsureNode { .. }
                        | Node::ImplicitNode { .. }
                        | Node::MatchWriteNode { .. }
                        | Node::MatchRequiredNode { .. }
                        | Node::MatchPredicateNode { .. }
                        | Node::InNode { .. }
                ) {
                    self.facts.risks.push(p);
                }
                if let Some(call) = node.as_call_node() {
                    self.needs_assignments |=
                        matches!(call.name().as_slice(), b"inject" | b"reduce")
                            && call.block().is_some();
                    self.facts.custom_dispatch |= super::file_api::binds_method(&call, methods)
                        || matches!(
                            call.name().as_slice(),
                            b"eval"
                                | b"class_eval"
                                | b"module_eval"
                                | b"instance_eval"
                                | b"using"
                                | b"refine"
                                | b"private"
                                | b"protected"
                                | b"public"
                                | b"private_class_method"
                                | b"public_class_method"
                                | b"send"
                                | b"__send__"
                                | b"public_send"
                        );
                    if matches!(
                        call.name().as_slice(),
                        b"binding"
                            | b"local_variables"
                            | b"eval"
                            | b"local_variable_get"
                            | b"local_variable_set"
                    ) {
                        self.facts.risks.push(p);
                    }
                }
                if let Some(read) = node.as_local_variable_read_node()
                    && matches!(read.name().as_slice(), b"_1" | b"_2")
                {
                    self.facts
                        .numbered_reads
                        .entry(read.name().as_slice().to_vec())
                        .or_default()
                        .push((p, loc.start_offset()..loc.end_offset()));
                }
            }
            fn leave(&mut self) {
                let (loc, p, indexed) = self.stack.pop().unwrap();
                if indexed {
                    self.facts.spans.insert(loc, p..self.count);
                }
            }
        }
        let mut b = Builder {
            facts: Self::default(),
            stack: vec![],
            positions: HashMap::new(),
            count: 0,
            needs_assignments: false,
        };
        walk(&mut b, root);
        if b.needs_assignments
            && let Some(program) = root.as_program_node()
        {
            let analysis = ScopeAnalysis::of(&program);
            for scope in analysis.scopes_in_exit_order() {
                for id in analysis.scope(scope).variables() {
                    let var = analysis.variable(id);
                    for &assignment in &var.assignments {
                        let a = analysis.assignment(assignment);
                        // The pinned syntactic assignment predicate excludes
                        // regexp captures. They still block corrections above.
                        if a.kind == AssignmentKind::NamedCapture {
                            continue;
                        }
                        let loc = a.node.location();
                        if let Some(&pos) = b.positions.get(&(loc.start_offset(), loc.end_offset()))
                        {
                            b.facts
                                .writes
                                .entry(var.name.to_vec())
                                .or_default()
                                .push(pos);
                        }
                    }
                }
            }
            for writes in b.facts.writes.values_mut() {
                writes.sort_unstable();
            }
        }
        #[cfg(test)]
        {
            b.facts.visits = b.count;
        }
        b.facts
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop each_with_object_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == EachWithObject.name())
            .collect()
    }
    #[test]
    fn pinned_diagnostics_and_canonical_corrections() {
        for (s, after, col) in [
            (
                "[].inject({}) { |a, e| a }",
                "[].each_with_object({}) { |e, a|  }",
                4,
            ),
            (
                "[]&.inject({}) { |a, e| a }",
                "[]&.each_with_object({}) { |e, a|  }",
                5,
            ),
            (
                "[].reduce({}) do |a, e|\n  a[e] = 1\n  a[e] = 1\n  a\nend",
                "[].each_with_object({}) do |e, a|\n  a[e] = 1\n  a[e] = 1\nend",
                4,
            ),
            (
                "[1, 2, 3].inject({}) do |h, i|\n  h[i] = i\n  h\nend",
                "[1, 2, 3].each_with_object({}) do |i, h|\n  h[i] = i\nend",
                11,
            ),
            (
                "[1, 2, 3].inject({}) do |h, i|\n  h\nend",
                "[1, 2, 3].each_with_object({}) do |i, h|\nend",
                11,
            ),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, col, 1));
            let name = if s.contains("reduce") {
                "reduce"
            } else {
                "inject"
            };
            assert_eq!(
                o[0].message,
                format!("Use `each_with_object` instead of `{name}`.")
            );
            assert_correction(&EachWithObject, s, after);
        }
    }
    #[test]
    fn pinned_numbered_corrections() {
        for dot in [".", "&."] {
            assert_correction(
                &EachWithObject,
                &format!("[]{dot}reduce({{}}) do\n  _1[_2] = 1\n  _1\nend"),
                &format!("[]{dot}each_with_object({{}}) do\n  _2[_1] = 1\n  _2\nend"),
            );
        }
    }
    #[test]
    fn pinned_negative_lookalikes() {
        for s in [
            "[].inject({}) { $GLOBAL[rand] = rand; $GLOBAL }",
            "[].reduce({}) { |h| h[rand] = rand; h }",
            "[].inject({}) { |a,e| a + e }",
            "[].reduce({}) { |a,e| my_method e,a }",
            "[].inject({}) { |a,e| }",
            "[].inject({}) { |a,e| a=e if e }",
            "[].inject(:+)",
            "[1,2,3].inject { |a,e| a+e }",
            "[].reduce({}) { |memo,item| memo += item; memo }",
            "array.reduce(0) { |a,e| a }",
            "[].inject({}) { it[foo]=bar; it }",
            "[].reduce({}) { _1 }",
            "[].reduce({}) { _3; _1 }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn parenthesized_body_has_one_upstream_begin_layer() {
        for body in ["(a)", "(work; a)"] {
            let s = format!("[].reduce({{}}) {{ |a,e| {body} }}");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 4));
            assert_correction(
                &EachWithObject,
                &s,
                &s.replace("reduce", "each_with_object")
                    .replace("|a,e|", "|e,a|")
                    .replace("a)", ")"),
            );
        }
        for body in ["((a))", "work; (a)", "begin a end"] {
            assert!(offenses(&format!("[].reduce({{}}) {{ |a,e| {body} }}")).is_empty());
        }
    }
    #[test]
    fn assignments_captures_and_parallel_targets() {
        for body in [
            "a = e; a",
            "a ||= e; a",
            "a &&= e; a",
            "a += e; a",
            "a,x = e; a",
            "for a in e; end; a",
            "proc { a = e }; a",
        ] {
            assert!(
                offenses(&format!("[].inject({{}}) {{ |a,e| {body} }}")).is_empty(),
                "{body}"
            );
        }
        assert_eq!(
            offenses("[].inject({}) { |a,e| proc { a[e]=e }; a }").len(),
            1
        );
    }
    #[test]
    fn duplicate_underscore_parameters_keep_offenses_without_corrections() {
        for name in ["_", "_memo"] {
            let s = format!("[1,2].reduce({{}}) {{ |{name}, {name}| {name}[:key] = 1; {name} }}");
            let o = correction_offenses(&EachWithObject, &s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 7));
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn assignments_in_parameter_defaults_do_not_suppress_body_offenses() {
        for header in ["a, e=(a=1)", "a, e: (a=1)"] {
            let s = format!("[1].reduce({{}}) {{ |{header}| a }}");
            let o = correction_offenses(&EachWithObject, &s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 5, 1));
            assert_eq!(o[0].message, "Use `each_with_object` instead of `reduce`.");
            assert!(o[0].correction.is_none(), "{s}");
            let assigned_body = format!("[1].reduce({{}}) {{ |{header}| a = e; a }}");
            assert!(offenses(&assigned_body).is_empty(), "{assigned_body}");
        }
    }
    #[test]
    fn pattern_and_regexp_binding_offenses_are_not_unsafe_corrections() {
        for body in [
            "e => a; a",
            "e in a; a",
            "case e; in a; end; a",
            "/(?<a>x)/ =~ e; a",
        ] {
            let s = format!("[\"x\"].reduce({{}}) {{ |a,e| {body} }}");
            let o = correction_offenses(&EachWithObject, &s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn numbered_shorthand_and_nested_scopes_never_rename_keys_or_other_bindings() {
        for s in [
            "[1].reduce({}) { _2; {_1:}; _1 }",
            "[1].reduce({}) { _2; proc { |x| x }; _1 }",
            "[1].reduce({}) { _2; -> { 1 }; _1 }",
        ] {
            let o = correction_offenses(&EachWithObject, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn heredoc_assignments_and_captures_follow_ast_not_source_containment() {
        let s = "[].reduce({}) { |a,e| puts <<~TEXT; a }\n#{a = e}\nTEXT\n";
        assert!(offenses(s).is_empty());
        let s = "[1].reduce({}) { |a,e| puts <<~TEXT; a }\n#{a[e] = e}\nTEXT\n";
        assert_eq!(offenses(s).len(), 1);
    }
}
