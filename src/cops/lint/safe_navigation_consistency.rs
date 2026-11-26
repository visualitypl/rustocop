//! `Lint/SafeNavigationConsistency` — pinned boolean operand/receiver consistency.
//! Unsafe upstream corrections are deliberately not exposed.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

pub struct SafeNavigationConsistency;

#[derive(Clone, Copy)]
struct Parts {
    first: [Option<usize>; 4], // csend AND, csend OR, non-nilable send AND/OR
    end: usize,
}
impl Parts {
    fn merge(&mut self, other: Self) {
        for (a, b) in self.first.iter_mut().zip(other.first) {
            *a = match (*a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        self.end = self.end.max(other.end);
    }
    fn decision(self) -> Option<(bool, usize)> {
        let [ca, co, sa, so] = self.first;
        if ca.zip(co).is_some_and(|(a, o)| a < o) {
            return None;
        }
        if let Some(a) = ca {
            Some((false, sa.map_or(a, |s| s.min(a)) + 1))
        } else if let Some((s, o)) = so.zip(co) {
            Some(if s < o { (false, s + 1) } else { (true, o + 1) })
        } else if let Some((s, o)) = sa.zip(co) {
            (s < o).then_some((false, o))
        } else {
            None
        }
    }
}
#[derive(Default)]
struct Group<'pr> {
    calls: Vec<CallNode<'pr>>,
    // Half-open operand intervals, mode, preorder priority. Keeping ranges
    // rather than re-emitting every suffix avoids quadratic long AND chains.
    intervals: Vec<(usize, usize, bool, usize)>,
}
fn boolean<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>, bool)> {
    if let Some(n) = node.as_and_node() {
        Some((n.left(), n.right(), true))
    } else {
        node.as_or_node().map(|n| (n.left(), n.right(), false))
    }
}
impl Cop for SafeNavigationConsistency {
    fn name(&self) -> &'static str {
        "Lint/SafeNavigationConsistency"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if boolean(node).is_none() || cx.parent().is_some_and(|n| boolean(n).is_some()) {
            return;
        }
        // One connected operator component, not a Program rescan. Parentheses,
        // kwbegin, assignments and synthetic ordinary/numbered/it blocks stop
        // collect_operands exactly as they do in the pinned parser AST.
        let mut groups: Vec<Group<'pr>> = Vec::new();
        let mut keys: HashMap<Vec<u8>, usize> = HashMap::new();
        let root = node
            .as_and_node()
            .map(|n| n.as_node())
            .or_else(|| node.as_or_node().map(|n| n.as_node()))
            .unwrap();
        let mut stack = vec![(root, None, false, 0)];
        let mut results: Vec<HashMap<usize, Parts>> = Vec::new();
        let mut priority = 0;
        while let Some((n, parent_and, leaving, order)) = stack.pop() {
            if leaving {
                let mut right = results.pop().unwrap();
                let mut left = results.pop().unwrap();
                // Small-to-large merging bounds work for many distinct receiver
                // groups as well as a long sequence on a single receiver.
                if left.len() < right.len() {
                    std::mem::swap(&mut left, &mut right);
                }
                for (id, parts) in right {
                    if let Some(existing) = left.get_mut(&id) {
                        existing.merge(parts);
                        if let Some((safe, start)) = existing.decision()
                            && start < existing.end
                        {
                            groups[id]
                                .intervals
                                .push((start, existing.end, safe, order));
                        }
                    } else {
                        left.insert(id, parts);
                    }
                }
                results.push(left);
            } else if let Some((left, right, and)) = boolean(&n) {
                let order = priority;
                priority += 1;
                stack.push((n, parent_and, true, order));
                stack.push((right, Some(and), false, 0));
                stack.push((left, Some(and), false, 0));
            } else {
                let mut result = HashMap::new();
                if let Some(call) = n
                    .as_call_node()
                    .filter(|c| !c.block().is_some_and(|b| b.as_block_node().is_some()))
                {
                    let key = call.receiver().map_or_else(Vec::new, |r| {
                        let end = if r
                            .as_call_node()
                            .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()))
                        {
                            r.location().end_offset()
                        } else {
                            ast::source_end(&r)
                        };
                        let source = file
                            .match_pattern_source(&file.bytes[r.location().start_offset()..end]);
                        // receiver.source comes from the reference parser's
                        // CRLF-normalized buffer, after encoding projection.
                        // Normalize only this comparison key: raw source and
                        // diagnostic offsets stay untouched; bare CR is data.
                        source
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &byte)| {
                                (byte != b'\r' || source.get(i + 1) != Some(&b'\n')).then_some(byte)
                            })
                            .collect()
                    });
                    let id = *keys.entry(key).or_insert_with(|| {
                        groups.push(Group::default());
                        groups.len() - 1
                    });
                    let group = &mut groups[id];
                    let index = group.calls.len();
                    let mut parts = Parts {
                        first: [None; 4],
                        end: index + 1,
                    };
                    let kind = if call.is_safe_navigation() {
                        Some(0)
                    } else if !nil_method(call.name().as_slice()) {
                        Some(2)
                    } else {
                        None
                    };
                    if let Some(kind) = kind {
                        parts.first[kind + usize::from(parent_and == Some(false))] = Some(index);
                    }
                    group.calls.push(call);
                    result.insert(id, parts);
                }
                results.push(result);
            }
        }
        for mut group in groups {
            group.intervals.sort_unstable();
            let mut pending = group.intervals.iter().peekable();
            let mut heaps: [BinaryHeap<Reverse<(usize, usize)>>; 2] = Default::default();
            for (index, call) in group.calls.iter().enumerate() {
                while let Some(&&(start, end, safe, order)) = pending.peek() {
                    if start > index {
                        break;
                    }
                    heaps[usize::from(safe)].push(Reverse((order, end)));
                    pending.next();
                }
                let operator = ast::is_operator_method(call.name().as_slice());
                let dot = call.call_operator_loc();
                let mut selected = None;
                for (mode, heap) in heaps.iter_mut().enumerate() {
                    while heap.peek().is_some_and(|Reverse((_, end))| *end <= index) {
                        heap.pop();
                    }
                    let appropriate = if mode == 1 {
                        call.is_safe_navigation()
                    } else {
                        operator || dot.as_ref().is_some_and(|l| l.as_slice() == b".")
                    };
                    if !appropriate
                        && let Some(&Reverse((order, _))) = heap.peek()
                        && selected.is_none_or(|(earlier, _)| order < earlier)
                    {
                        selected = Some((order, mode));
                    }
                }
                let Some((_, mode)) = selected else {
                    continue;
                };
                let (start, end) = if operator {
                    (
                        call.location().start_offset(),
                        ast::source_end(&call.as_node()),
                    )
                } else if let Some(dot) = dot {
                    (dot.start_offset(), dot.end_offset())
                } else {
                    continue;
                };
                let (line, col) = file.reference_line_col(start);
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    line,
                    col,
                    end_line: file.line_of(end.saturating_sub(1)),
                    message: if mode == 0 {
                        "Use `.` instead of unnecessary `&.`."
                    } else {
                        "Use `&.` for consistency with safe navigation."
                    }
                    .into(),
                    correction: None,
                });
            }
        }
    }
}
fn nil_method(name: &[u8]) -> bool {
    super::redundant_safe_navigation::nil_method(name)
        || matches!(
            name,
            b"to_d" | b"present?" | b"blank?" | b"presence" | b"try" | b"try!"
        )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/safe_navigation_consistency_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/SafeNavigationConsistency")
            .collect()
    }
    #[test]
    fn pinned_operator_ranges_and_mixed_parent_early_return() {
        for (s, col) in [
            ("foo&.bar && foo&.baz || foo&.qux", 16),
            ("foo.bar && foo.baz || foo&.qux", 26),
            ("foo&.zero? || foo > 5", 15),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{s}");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn component_root_accounts_for_each_subexpression_callback_once() {
        let source = "x&.foo && x&.bar || x&.baz";
        let parsed = crate::parser::parse(source.as_bytes());
        let program = parsed.node().as_program_node().unwrap();
        let root = program.statements().body().iter().next().unwrap();
        let left = root.as_or_node().unwrap().left();
        let file = SourceFile::new("case.rb".into(), source.as_bytes().to_vec());
        let reports = std::cell::RefCell::default();
        let context = NodeContext {
            ancestors: &[],
            corrections_requested: false,
            require_order_reports: &reports,
        };
        let mut whole = Vec::new();
        SafeNavigationConsistency.check_node(&root, &context, &file, &mut whole);
        assert_eq!(whole.len(), 1); // root early return must NOT lose inner AND
        let mut subtree = Vec::new();
        SafeNavigationConsistency.check_node(&left, &context, &file, &mut subtree);
        let positions = |items: &[Offense]| {
            items
                .iter()
                .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(positions(&subtree), positions(&whole));
        let parents = [root];
        let child_context = NodeContext {
            ancestors: &parents,
            ..context
        };
        SafeNavigationConsistency.check_node(&left, &child_context, &file, &mut whole);
        assert_eq!(whole.len(), 1); // already processed by the component root
        assert_eq!(positions(&offenses(source)), positions(&subtree));
    }
    #[test]
    fn five_allowed_methods_not_chain_six() {
        for method in [
            "present?", "blank?", "presence", "try", "try!", "nil?", "to_d", "to_json", "to_yaml",
        ] {
            assert!(
                offenses(&format!("foo&.bar || foo.{method}")).is_empty(),
                "{method}"
            );
        }
        assert_eq!(offenses("foo&.bar || foo.in?(x)").len(), 1);
    }
    #[test]
    fn mixed_newline_receiver_keys_both_orders_and_four_boolean_patterns() {
        for (left, op, right, message) in [
            ("&.", "&&", "&.", "Use `.` instead of unnecessary `&.`."),
            (
                "&.",
                "||",
                ".",
                "Use `&.` for consistency with safe navigation.",
            ),
            (".", "||", "&.", "Use `.` instead of unnecessary `&.`."),
            (".", "&&", "&.", "Use `.` instead of unnecessary `&.`."),
        ] {
            for (a, b) in [
                ("\n", "\r\n"),
                ("\r\n", "\n"),
                ("\n", "\n"),
                ("\r\n", "\r\n"),
            ] {
                let source = format!("build({a}1,{a}2){left}foo {op} build({b}1,{b}2){right}bar\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source:?}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (5, 3, 5));
                assert_eq!(found[0].message, message);
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn receiver_key_keeps_source_format_spelling_and_bare_cr_distinct() {
        for right in [
            "build(\r\n 1,\r\n2)",
            "build(\r\n0x1,\r\n2)",
            "build(\r\n1, # comment\r\n2)",
            "build(\r1,\r2)",
            "(build(\r\n1,\r\n2))",
        ] {
            let source = format!("build(\n1,\n2)&.foo && {right}&.bar\n");
            assert!(offenses(&source).is_empty(), "{source:?}");
        }
        assert!(offenses("build(\"a\rb\")&.foo || build(\"a\nb\").bar\n").is_empty());
        assert_eq!(
            offenses("build(\r1,\r2)&.foo && build(\r1,\r2)&.bar\n").len(),
            1
        );
    }
    #[test]
    fn bounded_boolean_components_and_siblings() {
        let s = std::iter::repeat_n("foo&.bar", 55)
            .collect::<Vec<_>>()
            .join(" && ");
        let found = crate::cops::testing::correction_offenses(&SafeNavigationConsistency, &s);
        assert_eq!(found.len(), 54);
        let s = (0..55)
            .map(|i| format!("v{i}&.bar"))
            .collect::<Vec<_>>()
            .join(" && ");
        assert!(
            crate::cops::testing::correction_offenses(&SafeNavigationConsistency, &s).is_empty()
        );
        assert_eq!(
            crate::cops::testing::correction_offenses(
                &SafeNavigationConsistency,
                &"each { x&.foo || x.bar }\n".repeat(2000)
            )
            .len(),
            2000
        );
    }
}
