//! `Style/RedundantSelf` — unnecessary explicit self receivers at fixed Ruby 3.4.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
pub struct RedundantSelf;
const KEYWORDS: &str = "alias and begin break case class def defined? do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield __FILE__ __LINE__ __ENCODING__";
// Kernel singleton methods in the fixed Ruby runtime. Bare lookup may dispatch
// to Kernel rather than the receiver's method, unlike a normal self call.
const KERNEL: &str = "Array Complex Float Hash Integer Rational String __callee__ __dir__ __method__ ` abort at_exit autoload autoload? binding block_given? caller caller_locations catch eval exec exit exit! fail fork format gets global_variables iterator? lambda load local_variables loop open p print printf proc putc puts raise rand readline readlines require require_relative select set_trace_func sleep spawn sprintf srand syscall system test throw trace_var trap untrace_var warn";
fn word(list: &str, name: &[u8]) -> bool {
    list.split_ascii_whitespace().any(|w| w.as_bytes() == name)
}
impl Cop for RedundantSelf {
    fn name(&self) -> &'static str {
        "Style/RedundantSelf"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/ColonMethodCall", "Layout/DotPosition"]
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
        let Some(c) = node.as_call_node() else { return };
        let Some(receiver) = c.receiver().filter(|r| r.as_self_node().is_some()) else {
            return;
        };
        let name = c.name().as_slice();
        if c.is_safe_navigation()
            || c.message_loc().is_none()
            || !regular(name)
            || word(KERNEL, name)
            || it_in_block(&c, cx)
        {
            return;
        }
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        if file
            .self_facts(root)
            .allows(name, c.location().start_offset())
        {
            return;
        }
        let (line, col) = file.line_col(receiver.location().start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                // RuboCop removes the receiver and the dot.
                let dot = c.call_operator_loc()?;
                super::correction::combine(
                    file,
                    vec![
                        Correction {
                            range: receiver.location().start_offset()
                                ..receiver.location().end_offset(),
                            replacement: String::new(),
                        },
                        Correction {
                            range: dot.start_offset()..dot.end_offset(),
                            replacement: String::new(),
                        },
                    ],
                )
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Redundant `self` detected.".into(),
            correction,
        });
    }
}
fn regular(name: &[u8]) -> bool {
    !word(KEYWORDS, name)
        && name
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || *b == b'_' || !b.is_ascii())
        && !name.ends_with(b"=")
}
fn it_in_block(c: &CallNode<'_>, cx: &NodeContext<'_, '_>) -> bool {
    if c.name().as_slice() != b"it" || !ast::call_arguments(c).is_empty() || c.block().is_some() {
        return false;
    }
    cx.ancestors()
        .find_map(|n| {
            n.as_block_node()
                .map(|b| b.parameters())
                .or_else(|| n.as_lambda_node().map(|b| b.parameters()))
        })
        .is_some_and(|parameters| {
            parameters.is_none()
                || parameters.is_some_and(|p| {
                    p.as_numbered_parameters_node().is_some() || p.as_it_parameters_node().is_some()
                })
        })
}
#[derive(Default)]
pub(crate) struct Facts {
    ranges: HashMap<Vec<u8>, Vec<Range<usize>>>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
fn key(n: &Node<'_>) -> (usize, usize) {
    (n.location().start_offset(), n.location().end_offset())
}
fn local_name<'pr>(n: &Node<'pr>) -> Option<&'pr [u8]> {
    macro_rules! name {($($access:ident),*) => {$ (if let Some(v)=n.$access() { return Some(v.name().as_slice()); })*};}
    name!(
        as_local_variable_write_node,
        as_local_variable_target_node,
        as_local_variable_or_write_node,
        as_local_variable_and_write_node,
        as_local_variable_operator_write_node,
        as_block_local_variable_node,
        as_required_parameter_node,
        as_optional_parameter_node,
        as_required_keyword_parameter_node,
        as_optional_keyword_parameter_node
    );
    if let Some(v) = n.as_rest_parameter_node() {
        return v.name().map(|n| n.as_slice());
    }
    if let Some(v) = n.as_keyword_rest_parameter_node() {
        return v.name().map(|n| n.as_slice());
    }
    if let Some(v) = n.as_block_parameter_node() {
        return v.name().map(|n| n.as_slice());
    }
    None
}
impl Facts {
    fn add(&mut self, name: &[u8], range: Range<usize>) {
        self.ranges.entry(name.to_vec()).or_default().push(range);
    }
    pub(crate) fn build(root: &Node<'_>) -> Self {
        // Index conditional declarations once. Each assignment touches only its
        // enclosing predicates, never rescans a subtree for each self receiver.
        #[derive(Default)]
        struct Conditions<'pr> {
            stack: Vec<Node<'pr>>,
            names: HashMap<(usize, usize), HashSet<Vec<u8>>>,
        }
        fn predicate<'pr>(n: &Node<'pr>) -> Option<Node<'pr>> {
            n.as_if_node()
                .map(|n| n.predicate())
                .or_else(|| n.as_unless_node().map(|n| n.predicate()))
                .or_else(|| n.as_while_node().map(|n| n.predicate()))
                .or_else(|| n.as_until_node().map(|n| n.predicate()))
        }
        impl<'pr> NodeVisitor<'pr> for Conditions<'pr> {
            fn enter(&mut self, n: Node<'pr>) {
                let pattern_target = n.as_local_variable_target_node().is_some()
                    && self
                        .stack
                        .iter()
                        .rev()
                        .find(|p| {
                            matches!(
                                p,
                                Node::MultiWriteNode { .. }
                                    | Node::RescueNode { .. }
                                    | Node::ForNode { .. }
                                    | Node::MatchWriteNode { .. }
                                    | Node::InNode { .. }
                                    | Node::MatchRequiredNode { .. }
                                    | Node::MatchPredicateNode { .. }
                            )
                        })
                        .is_some_and(|p| {
                            matches!(
                                p,
                                Node::MatchWriteNode { .. }
                                    | Node::InNode { .. }
                                    | Node::MatchRequiredNode { .. }
                                    | Node::MatchPredicateNode { .. }
                            )
                        });
                if !pattern_target
                    && crate::cops::variables::is_local_assignment(&n)
                    && let Some(name) = local_name(&n)
                {
                    for p in &self.stack {
                        if let Some(condition) = predicate(p) {
                            self.names
                                .entry(key(&condition))
                                .or_default()
                                .insert(name.to_vec());
                        }
                    }
                }
                self.stack.push(n);
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        let mut conditions = Conditions::default();
        walk::walk(&mut conditions, root);
        struct Build<'pr> {
            facts: Facts,
            stack: Vec<(Node<'pr>, Option<Range<usize>>)>,
            conditions: HashMap<(usize, usize), HashSet<Vec<u8>>>,
        }
        impl<'pr> NodeVisitor<'pr> for Build<'pr> {
            fn enter(&mut self, n: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                let mut scope = self.stack.last().and_then(|(_, s)| s.clone());
                if n.as_def_node().is_some()
                    || (n.as_block_node().is_some() && scope.is_none())
                    || n.as_lambda_node().is_some()
                {
                    scope = Some(n.location().start_offset()..n.location().end_offset());
                }
                if let Some(names) = self.conditions.get(&key(&n)) {
                    for name in names {
                        if scope.is_none()
                            && let Some(call) = n
                                .as_call_node()
                                .filter(|c| !ast::call_arguments(c).is_empty())
                        {
                            // Like assignment RHSs, argument-bearing predicates
                            // exempt only their arguments, not the call receiver.
                            for arg in ast::call_arguments(&call) {
                                self.facts.add(
                                    name,
                                    arg.location().start_offset()..arg.location().end_offset(),
                                );
                            }
                        } else {
                            self.facts.add(
                                name,
                                n.location().start_offset()
                                    ..scope.as_ref().map_or(n.location().end_offset(), |s| s.end),
                            );
                        }
                    }
                }
                // For indices and named-regexp captures are not lvasgn
                // callbacks in the pinned detector. Do not confuse their
                // synthetic Prism targets with real assignment/parameter nodes.
                let synthetic_capture = n.as_local_variable_target_node().is_some()
                    && self
                        .stack
                        .iter()
                        .rev()
                        .find(|(p, _)| {
                            matches!(
                                p,
                                Node::MultiWriteNode { .. }
                                    | Node::RescueNode { .. }
                                    | Node::ForNode { .. }
                                    | Node::MatchWriteNode { .. }
                            )
                        })
                        .is_some_and(|(p, _)| {
                            matches!(p, Node::ForNode { .. } | Node::MatchWriteNode { .. })
                        });
                if let Some(name) = local_name(&n).filter(|_| !synthetic_capture) {
                    let pattern = self.stack.iter().rev().find(|(p, _)| {
                        p.as_in_node().is_some()
                            || p.as_match_required_node().is_some()
                            || p.as_match_predicate_node().is_some()
                    });
                    // Parameter callbacks share the argument scope: even names
                    // declared later protect earlier defaults. Do not give local
                    // assignments *inside* defaults this parameter-wide range.
                    let parameter = !crate::cops::variables::is_local_assignment(&n)
                        || (n.as_local_variable_target_node().is_some()
                            && self
                                .stack
                                .iter()
                                .rev()
                                .find(|(p, _)| p.as_multi_target_node().is_none())
                                .is_some_and(|(p, _)| p.as_parameters_node().is_some()));
                    let range = if let Some(s) = &scope {
                        if parameter {
                            let start = self
                                .stack
                                .iter()
                                .rev()
                                .find(|(p, _)| {
                                    p.as_parameters_node().is_some()
                                        || p.as_block_parameters_node().is_some()
                                })
                                .map_or(n.location().start_offset(), |(p, _)| {
                                    p.location().start_offset()
                                });
                            start..s.end
                        } else {
                            n.location().start_offset()..s.end
                        }
                    } else if let Some((p, _)) = pattern {
                        p.location().start_offset()..p.location().end_offset()
                    } else if crate::cops::variables::is_local_assignment(&n) {
                        // At top level upstream only marks the assignment's RHS
                        // (or its call arguments), not subsequent sibling calls.
                        if let Some(w) = n.as_local_variable_write_node() {
                            let rhs = w.value();
                            if let Some(c) = rhs
                                .as_call_node()
                                .filter(|c| !ast::call_arguments(c).is_empty())
                            {
                                for arg in ast::call_arguments(&c) {
                                    self.facts.add(
                                        name,
                                        arg.location().start_offset()..arg.location().end_offset(),
                                    );
                                }
                                0..0
                            } else {
                                n.location().start_offset()..n.location().end_offset()
                            }
                        } else if n.as_local_variable_target_node().is_some() {
                            if let Some(multi) = self
                                .stack
                                .iter()
                                .rev()
                                .find_map(|(p, _)| p.as_multi_write_node())
                            {
                                let rhs = multi.value();
                                if let Some(c) = rhs
                                    .as_call_node()
                                    .filter(|c| !ast::call_arguments(c).is_empty())
                                {
                                    for arg in ast::call_arguments(&c) {
                                        self.facts.add(
                                            name,
                                            arg.location().start_offset()
                                                ..arg.location().end_offset(),
                                        );
                                    }
                                    self.stack.push((n, scope));
                                    return;
                                }
                            }
                            self.stack
                                .iter()
                                .rev()
                                .find(|(p, _)| {
                                    p.as_multi_write_node().is_some()
                                        || p.as_rescue_node().is_some()
                                })
                                .map(|(p, _)| {
                                    p.location().start_offset()..p.location().end_offset()
                                })
                                .unwrap_or(0..0)
                        } else {
                            n.location().start_offset()..n.location().end_offset()
                        }
                    } else {
                        0..0
                    };
                    self.facts.add(name, range);
                }
                self.stack.push((n, scope));
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        let mut b = Build {
            facts: Self::default(),
            stack: Vec::new(),
            conditions: conditions.names,
        };
        walk::walk(&mut b, root);
        for ranges in b.facts.ranges.values_mut() {
            ranges.sort_by_key(|r| (r.start, r.end));
            let mut merged: Vec<Range<usize>> = Vec::new();
            for r in ranges.drain(..) {
                if let Some(last) = merged.last_mut()
                    && r.start <= last.end
                {
                    last.end = last.end.max(r.end);
                } else {
                    merged.push(r);
                }
            }
            *ranges = merged;
        }
        b.facts
    }
    fn allows(&self, name: &[u8], offset: usize) -> bool {
        let Some(ranges) = self.ranges.get(name) else {
            return false;
        };
        let i = ranges.partition_point(|r| r.start <= offset);
        i > 0 && offset < ranges[i - 1].end
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_self_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    #[test]
    fn later_parameters_are_visible_to_earlier_defaults() {
        for source in [
            "def f(a = self.b, b = 1); self.b; end",
            "def f(a: self.b, b: 1); self.b; end",
            "def f(a = self.b, *b); self.b; end",
            "def f(a = self.b, **b); self.b; end",
            "def f(a = self.b, &b); self.b; end",
            "def f(a = self.b, (b,c)); self.b; end",
            "->(a = self.b, b = 1) { self.b }",
            "items.each { |a = self.b, b = 1| self.b }",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&RedundantSelf, source).is_empty(),
                "{source}"
            );
        }
        for (source, col) in [
            ("def f(a = self.b, c = 1); end", 11),
            ("def f(a = self.b, c = (b = 1)); end", 11),
            ("def f(a = self.b); items.each { |b| self.b }; end", 11),
            (
                "def f; self.b; items.each { |a = self.b, b = 1| self.b }; end",
                8,
            ),
        ] {
            let found = crate::cops::testing::correction_offenses(&RedundantSelf, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
        }
    }
    #[test]
    fn incompatible_dot_position_contract_is_enforced_even_before_layout_cop_exists() {
        struct DotPosition;
        impl Cop for DotPosition {
            fn name(&self) -> &'static str {
                "Layout/DotPosition"
            }
        }
        let found = crate::pipeline::lint_str_with_corrections(
            &[&RedundantSelf, &DotPosition],
            "self.object_id\n",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        assert_eq!(
            RedundantSelf.autocorrect_incompatible_with(),
            &["Style/ColonMethodCall", "Layout/DotPosition"]
        );
    }
    #[test]
    fn all_pinned_diagnostics_and_messages() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_self_pinned.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let found = crate::cops::testing::correction_offenses(
                &RedundantSelf,
                case["source"].as_str().unwrap(),
            );
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{case}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(messages), case["messages"], "{case}");
        }
    }
}
