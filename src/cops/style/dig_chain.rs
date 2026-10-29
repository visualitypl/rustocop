//! `Style/DigChain` — suggest collapsing consecutive dig calls (detection only).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

pub struct DigChain;
impl Cop for DigChain {
    fn name(&self) -> &'static str {
        "Style/DigChain"
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
        if call.name().as_slice() != b"dig" || call.call_operator_loc().is_none() {
            return;
        }
        let facts = file.dig_calls(cx.ancestors().next_back().unwrap_or(node));
        if !facts.is_dig(&call) {
            return;
        }
        // Only inspect maximal receiver chains, including those ultimately
        // rejected for forwarding. Upstream ignores their inner nodes too.
        // This makes total chain work linear without mutable per-file state.
        if !literal_block(&call)
            && cx
                .parent()
                .and_then(Node::as_call_node)
                .is_some_and(|parent| {
                    parent.call_operator_loc().is_some()
                        && facts.is_dig(&parent)
                        && parent.receiver().is_some_and(|receiver| {
                            receiver.location().start_offset() == node.location().start_offset()
                                && receiver.location().end_offset() == node.location().end_offset()
                        })
                })
        {
            return;
        }
        let mut groups = vec![ast::call_arguments(&call)];
        let mut receiver = call.receiver();
        let mut start = None;
        while let Some(inner) = receiver
            .filter(|node| facts.dig_expression(node))
            .and_then(|n| n.as_call_node())
        {
            start = inner.message_loc().map(|loc| loc.start_offset());
            groups.push(ast::call_arguments(&inner));
            receiver = inner.receiver();
        }
        let Some(start) = start else { return };
        let arguments: Vec<_> = groups.into_iter().rev().flatten().collect();
        if arguments
            .iter()
            .take(arguments.len().saturating_sub(1))
            .any(|arg| arg.as_forwarding_arguments_node().is_some())
        {
            return;
        }
        let arguments = arguments
            .iter()
            .map(|arg| single_line(arg.location().as_slice()))
            .collect::<Vec<_>>()
            .join(", ");
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `dig({arguments})` instead of chaining."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}

/// RuboCop's DigHelp matcher: at least one argument, no hash or block pass.
/// A literal block wraps the send in RuboCop, but is part of Prism's CallNode.
fn is_dig(call: &CallNode<'_>) -> bool {
    if call.name().as_slice() != b"dig" {
        return false;
    }
    let args = ast::call_arguments(call);
    !args.is_empty()
        && args.iter().all(|arg| {
            !matches!(
                arg,
                Node::HashNode { .. }
                    | Node::KeywordHashNode { .. }
                    | Node::BlockArgumentNode { .. }
            )
        })
}

/// One lazy, shared index of the DigHelp predicate. A wide dig argument list
/// can itself contain many single-argument digs; scanning that parent for each
/// child would be quadratic. Selector offsets uniquely identify sends even
/// when receiver chains share their expression start offset.
#[derive(Default)]
pub(crate) struct Facts {
    selectors: std::collections::HashSet<usize>,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        use crate::cops::walk::{NodeVisitor, walk};
        struct Builder(Facts);
        impl<'pr> NodeVisitor<'pr> for Builder {
            fn enter(&mut self, node: Node<'pr>) {
                if let Some(call) = node.as_call_node().filter(is_dig)
                    && let Some(selector) = call.message_loc()
                {
                    self.0.selectors.insert(selector.start_offset());
                }
            }
        }
        let mut builder = Builder(Self::default());
        walk(&mut builder, root);
        builder.0
    }
    pub(super) fn is_dig(&self, call: &CallNode<'_>) -> bool {
        call.message_loc()
            .is_some_and(|loc| self.selectors.contains(&loc.start_offset()))
    }
    pub(super) fn dig_expression(&self, node: &Node<'_>) -> bool {
        node.as_call_node()
            .is_some_and(|call| self.is_dig(&call) && !literal_block(&call))
    }
}

pub(super) fn literal_block(call: &CallNode<'_>) -> bool {
    call.block()
        .is_some_and(|block| block.as_block_node().is_some())
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `dig_chain_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/DigChain")
            .collect()
    }
    #[test]
    fn all_pinned_examples_match_exact_positions_messages_and_emit_no_corrections() {
        use crate::cops::testing::correction_offenses;
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/dig_chain_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 24);
        for case in cases.as_array().unwrap() {
            let found = correction_offenses(&super::DigChain, case["source"].as_str().unwrap());
            let actual: Vec<_> = found
                .iter()
                .map(|o| (o.line, o.col, o.message.as_str()))
                .collect();
            let expected: Vec<_> = case["positions"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["messages"].as_array().unwrap())
                .map(|(p, m)| {
                    (
                        p[0].as_u64().unwrap() as u32,
                        p[1].as_u64().unwrap() as u32,
                        m.as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(actual, expected, "{case}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn literal_blocks_grouping_and_arguments_are_not_receiver_chain_edges() {
        for source in [
            "(x.dig(:a)).dig(:b)",
            "x.dig(:a) { :v }.dig(:b)",
            "x.dig(y.dig(:a))",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("x.dig(:a).dig(:b) { :v }.dig(:c).dig(:d)");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].message, "Use `dig(:a, :b)` instead of chaining.");
        assert_eq!(found[1].message, "Use `dig(:c, :d)` instead of chaining.");
        let found = offenses("x.dig(:a).dig(:b, x.dig(:c).dig(:d))");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn reports_only_the_outer_chain_with_exact_selector_position_and_message() {
        for (source, col, replacement) in [
            (
                "x.dig(:foo).dig(:bar).dig(:baz)",
                3,
                "dig(:foo, :bar, :baz)",
            ),
            (
                "x.y.z.dig(:foo).dig(:bar).dig(:baz)",
                7,
                "dig(:foo, :bar, :baz)",
            ),
            (
                "x.dig(:foo, :bar).dig(:baz, :quux)",
                3,
                "dig(:foo, :bar, :baz, :quux)",
            ),
            (
                "x.dig(:foo, :bar)&.dig(:baz, :quux)",
                3,
                "dig(:foo, :bar, :baz, :quux)",
            ),
            (
                "x&.dig(:foo, :bar)&.dig(:baz, :quux)",
                4,
                "dig(:foo, :bar, :baz, :quux)",
            ),
            (
                "dig(:foo, :bar).dig(:baz, :quux)",
                1,
                "dig(:foo, :bar, :baz, :quux)",
            ),
            (
                "x.dig(:foo, :bar)::dig(:baz, :quux)",
                3,
                "dig(:foo, :bar, :baz, :quux)",
            ),
            (
                "x.dig(:foo).dig(*names).dig(:bar)",
                3,
                "dig(:foo, *names, :bar)",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("Use `{replacement}` instead of chaining.")
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn ignores_nonchains_hashes_and_argumentless_calls() {
        for source in [
            "dig",
            "X::dig(:foo, :bar)",
            "x.dig(:foo).bar",
            "x.bar.dig(:foo)",
            "x.dig(:foo).first.dig(:bar)",
            "x.dig",
            "x.dig(:foo, :bar).dig",
            "x.dig.dig(:foo, :bar)",
            "x.dig({foo: 1}).dig({baz: 2})",
            "x.dig(foo: 1).dig(baz: 2)",
            "x.dig(:foo, &block).dig(:bar)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn forwarding_only_at_end_and_anonymous_splat_are_supported() {
        for (source, replacement) in [
            ("def f(...); x.dig(:foo).dig(...); end", "dig(:foo, ...)"),
            ("def f(*); x.dig(*).dig(*); end", "dig(*, *)"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                found[0].message,
                format!("Use `{replacement}` instead of chaining.")
            );
        }
        for source in [
            "def f(...); x.dig(...).dig(:foo); end",
            "def f(...); x.dig(...).dig(...); end",
            "def f(&); x.dig(&).dig(:foo); end",
            "def f(**); x.dig(**).dig(:foo); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn multiline_comments_and_indentation_keep_full_offense_span() {
        let found = offenses(
            "def f\n  x.dig(:foo) # first\n   .dig(:bar) # second\n   .dig(:baz) # third\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 5, 4));
        assert_eq!(
            found[0].message,
            "Use `dig(:foo, :bar, :baz)` instead of chaining."
        );
    }
}
