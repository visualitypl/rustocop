//! `Style/SafeNavigationChainLength` — fixed maximum of two safe calls.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

pub struct SafeNavigationChainLength;

#[cfg(test)]
thread_local! {
    static CHAIN_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SEND_END_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Cop for SafeNavigationChainLength {
    fn name(&self) -> &'static str {
        "Style/SafeNavigationChainLength"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !is_safe_call(node) {
            return;
        }
        // RuboCop wraps a literal-block call in a block node, and has no
        // ArgumentsNode. Only the outermost csend in each component reports.
        let parent = cx.ancestors().find(|n| n.as_arguments_node().is_none());
        if node.as_call_node().is_some()
            && !has_literal_block(node)
            && parent.is_some_and(|p| {
                is_safe_call(p)
                    && (p.as_call_node().is_some()
                        || node.location().end_offset() <= safe_send_end(p))
            })
        {
            return;
        }
        if !has_safe_descendant(node, 2) {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid safe navigation chains longer than 2 calls.".into(),
            line,
            col,
            end_line: file.line_of(safe_send_end(node) - 1),
            correction: None,
        });
    }
}

fn is_safe_call(node: &Node<'_>) -> bool {
    match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().is_safe_navigation(),
        Node::CallTargetNode { .. } => node.as_call_target_node().unwrap().is_safe_navigation(),
        Node::CallAndWriteNode { .. } => {
            node.as_call_and_write_node().unwrap().is_safe_navigation()
        }
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node().unwrap().is_safe_navigation(),
        Node::CallOperatorWriteNode { .. } => node
            .as_call_operator_write_node()
            .unwrap()
            .is_safe_navigation(),
        _ => false,
    }
}

// Compound writes contain a synthetic reader csend in RuboCop. Its receiver
// belongs to the chain, but the assignment wrapper and RHS do not.
fn safe_send_end(node: &Node<'_>) -> usize {
    #[cfg(test)]
    SEND_END_WORK.with(|work| {
        work.set(
            work.get()
                + 1
                + node
                    .as_call_node()
                    .and_then(|c| c.arguments())
                    .map_or(0, |a| a.arguments().len()),
        );
    });
    let message = match node {
        Node::CallAndWriteNode { .. } => node.as_call_and_write_node().unwrap().message_loc(),
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node().unwrap().message_loc(),
        Node::CallOperatorWriteNode { .. } => {
            node.as_call_operator_write_node().unwrap().message_loc()
        }
        _ => return ast::source_end(node),
    };
    message.map_or_else(|| node.location().end_offset(), |loc| loc.end_offset())
}

fn has_literal_block(node: &Node<'_>) -> bool {
    node.as_call_node()
        .and_then(|c| c.block())
        .is_some_and(|b| b.as_block_node().is_some())
}

// Look only two csend edges down from a component root, not through arbitrary
// subtrees. In RuboCop, direct call arguments count too (not just receivers).
// Each component is examined once; blocks, grouping and ordinary sends break
// it. No reparsing, full-source scans or work proportional to chain length.
fn has_safe_descendant(node: &Node<'_>, depth: usize) -> bool {
    #[cfg(test)]
    CHAIN_WORK.with(|work| work.set(work.get() + 1));
    let receiver = match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().receiver(),
        Node::CallTargetNode { .. } => Some(node.as_call_target_node().unwrap().receiver()),
        Node::CallAndWriteNode { .. } => node.as_call_and_write_node().unwrap().receiver(),
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node().unwrap().receiver(),
        Node::CallOperatorWriteNode { .. } => {
            node.as_call_operator_write_node().unwrap().receiver()
        }
        _ => return false,
    };
    receiver
        .into_iter()
        .chain(
            node.as_call_node()
                .and_then(|c| c.arguments())
                .into_iter()
                .flat_map(|a| a.arguments().iter()),
        )
        .any(|child| {
            #[cfg(test)]
            CHAIN_WORK.with(|work| work.set(work.get() + 1));
            child.as_call_node().is_some()
                && is_safe_call(&child)
                && !has_literal_block(&child)
                && (depth == 1 || has_safe_descendant(&child, depth - 1))
        })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/safe_navigation_chain_length_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SafeNavigationChainLength")
            .collect()
    }

    #[test]
    fn pinned_max_two_offenses() {
        for source in [
            "x&.foo&.bar&.baz",
            "x.foo&.bar&.baz&.zoo",
            "x.foo&.bar&.baz&.zoo.nil?",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!(
                found[0].message,
                "Avoid safe navigation chains longer than 2 calls."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn reports_once_per_connected_component_even_for_long_or_branching_chains() {
        for source in [
            "x&.a&.b&.c&.d&.e",
            "x&.a(y&.b(z&.c))",
            "x&.a&.b&.c(y&.a&.b&.c, z&.a&.b&.c)",
            "x&.a&.b&.c(&block)",
            "x&.a&.b&.c = value",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1), "{source}");
        }
    }

    #[test]
    fn compound_assignment_counts_the_synthetic_reader_not_its_rhs() {
        for op in ["+=", "||=", "&&="] {
            let source = format!("x&.a&.b&.c {op}\n  y&.a&.b&.c");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!((found[1].line, found[1].col), (2, 3));
            assert!(offenses(&format!("x&.a&.b {op} y&.c")).is_empty());
        }
    }

    #[test]
    fn for_and_rescue_assignment_targets_count_the_setter_chain() {
        for (source, col) in [
            ("for x&.a&.b&.c in items; body; end", 5),
            ("begin; a; rescue => x&.a&.b&.c; end", 21),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert!(offenses(&source.replace("&.c", "")).is_empty(), "{source}");
        }
    }

    #[test]
    fn grouping_regular_calls_literals_and_blocks_break_components() {
        for source in [
            "(x&.a)&.b&.c",
            "x&.a.foo&.b&.c",
            "x&.a&.b(y&.c)",
            "x&.a([y&.b(z&.c)])",
            "x&.a(k: y&.b(z&.c))",
            "x&.a&.b(&y&.c)",
            "x&.a {}&.b&.c",
            "x&.a&.b {}&.c&.d",
            "x&.a { y&.b&.c }",
            "x&.a&.b { y&.c }",
            "x&.a&.b do |y| y&.c end",
            "x&.a(-> { y&.b&.c })",
            "# x&.a&.b&.c\n'x&.a&.b&.c'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn block_bodies_and_literal_block_sends_report_their_own_chains() {
        for (source, line, col, end_line) in [
            ("x&.a&.b {y&.c&.d&.e}", 1, 10, 1),
            ("x&.a&.b&.c {\n  body\n}", 1, 1, 1),
            ("x&.a&.b&.c do\n  body\nend", 1, 1, 1),
            ("x&.a&.b&.c {}&.d&.e&.f", 1, 1, 1),
            ("x&.a(-> { y&.b&.c&.d })", 1, 11, 1),
        ] {
            let found = offenses(source);
            // The fourth source has two components, both beginning at col 1.
            assert_eq!(
                found.len(),
                if source.contains("&.f") { 2 } else { 1 },
                "{source}"
            );
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line),
                "{source}"
            );
        }
    }

    #[test]
    fn exact_multiline_positions_and_end_line_exclude_a_trailing_ordinary_call() {
        let found = offenses("def f(x)\n  x&.a\n    &.b\n    &.c\n    .nil?\nend");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
    }

    #[test]
    fn receiver_type_method_names_and_lexical_binding_do_not_change_syntax_policy() {
        for receiver in [
            "nil",
            "false",
            "self",
            "Box.new",
            "[].freeze",
            "\"x\"",
            "Custom::Box",
            "value",
        ] {
            let source =
                format!("module M\n def f(value)\n  {receiver}&.nil?&.freeze&.new\n end\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 3), "{source}");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn chain_inspection_is_linear_across_components_and_stops_after_two_edges() {
        for length in [3, 10, 50] {
            let source = format!("x{}\n", "&.a".repeat(length));
            CHAIN_WORK.with(|work| work.set(0));
            assert_eq!(offenses(&source).len(), 1);
            assert_eq!(CHAIN_WORK.with(|work| work.get()), 4);
        }
        CHAIN_WORK.with(|work| work.set(0));
        assert_eq!(offenses(&"x&.a&.b&.c\n".repeat(1000)).len(), 1000);
        assert_eq!(CHAIN_WORK.with(|work| work.get()), 4000);

        let source = format!("x&.a({})\n", vec!["y&.a"; 1000].join(", "));
        CHAIN_WORK.with(|work| work.set(0));
        SEND_END_WORK.with(|work| work.set(0));
        assert!(offenses(&source).is_empty());
        assert_eq!(CHAIN_WORK.with(|work| work.get()), 3002);
        assert_eq!(SEND_END_WORK.with(|work| work.get()), 0);
    }

    #[test]
    fn pinned_max_two_clean_examples() {
        for source in ["x&.foo&.bar", "x.foo.bar"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
