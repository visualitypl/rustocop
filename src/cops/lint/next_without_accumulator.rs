//! `Lint/NextWithoutAccumulator` — bare next in reduce/inject blocks.

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

/// Detection only: choosing an accumulator would change program behavior.
pub struct NextWithoutAccumulator;
impl Cop for NextWithoutAccumulator {
    fn name(&self) -> &'static str {
        "Lint/NextWithoutAccumulator"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_block_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(block) = node.as_block_node() else {
            return;
        };
        let Some(call) = cx.parent().and_then(Node::as_call_node) else {
            return;
        };
        if !matches!(call.name().as_slice(), b"reduce" | b"inject") {
            return;
        }
        let args = ast::call_arguments(&call);
        let [arg] = args.as_slice() else { return };
        if arg.as_symbol_node().is_some() {
            return;
        }
        let Some(body) = block.body() else { return };
        if !begin_body(&body) {
            return;
        }
        let mut search = BareNext::default();
        walk::walk(&mut search, &body);
        let Some(offset) = search.first else { return };
        let (line, col) = file.reference_line_col(offset);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            message: "Use `next` with an accumulator argument in a `reduce`.".into(),
            line,
            col,
            end_line: line,
        });
    }
}

// RuboCop's begin is either multiple statements or explicit parentheses,
// NOT begin/end (kwbegin) or an implicit rescue/ensure body.
fn begin_body(node: &Node<'_>) -> bool {
    if let Some(statements) = node.as_statements_node() {
        let body = statements.body();
        return body.len() > 1
            || (body.len() == 1
                && body
                    .iter()
                    .next()
                    .is_some_and(|n| n.as_parentheses_node().is_some()));
    }
    node.as_parentheses_node().is_some()
}

#[derive(Default)]
struct BareNext {
    barriers: Vec<bool>,
    nested: usize,
    first: Option<usize>,
}
impl<'pr> NodeVisitor<'pr> for BareNext {
    fn enter(&mut self, node: Node<'pr>) {
        // A RuboCop block encloses its send (receiver and arguments included).
        // Prism puts the block below its call/super instead. Count that whole
        // subtree, not only BlockNode, as the nearest nested-block boundary.
        let boundary = node.as_lambda_node().is_some()
            || node.as_block_node().is_some()
            || node
                .as_call_node()
                .is_some_and(|n| n.block().is_some_and(|b| b.as_block_node().is_some()))
            || node
                .as_super_node()
                .is_some_and(|n| n.block().is_some_and(|b| b.as_block_node().is_some()))
            || node
                .as_forwarding_super_node()
                .is_some_and(|n| n.block().is_some());
        self.barriers.push(boundary);
        self.nested += usize::from(boundary);
        if self.nested == 0
            && self.first.is_none()
            && node.as_next_node().is_some_and(|n| n.arguments().is_none())
        {
            self.first = Some(node.location().start_offset());
        }
    }
    fn leave(&mut self) {
        self.nested -= usize::from(self.barriers.pop().expect("balanced walk"));
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/next_without_accumulator_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NextWithoutAccumulator")
            .collect()
    }
    #[test]
    fn reports_only_first_bare_next_in_each_eligible_block() {
        let source = "xs.reduce(0) do |acc, x|\n  next acc if a\n  next if b\n  next if c\n  xs.inject(0) { next if d; acc }\n  acc\nend\n";
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 3), (5, 18)]
        );
    }

    #[test]
    fn exact_arity_and_begin_body_not_general_reduce_control_flow() {
        for source in [
            "xs.reduce { next if test; acc }",
            "xs.reduce(:sum) { next if test; acc }",
            "xs.reduce(0, :sum) { next if test; acc }",
            "xs.reduce(0) { next if test }",
            "xs.reduce(0) { begin; next if test; acc; end }",
            "xs.reduce(0) { next if test; acc; rescue; nil }",
            "xs.reduce(0) { next(); acc }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "xs.reduce(0) { (next) }",
            "xs.inject(:\"#{name}\") { next if test; acc }",
            "xs.reduce(*args) { next if test; acc }",
            "xs.reduce(key: 0) { next if test; acc }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn nearest_block_includes_callee_arguments_and_lambda_parameters() {
        for source in [
            "xs.reduce(0) { foo((next if test)) { value }; acc }",
            "xs.reduce(0) { ->(x = (next if test)) { x }; acc }",
            "xs.reduce(0) { foo { next; acc }; acc }",
            "xs.reduce(0) { foo { it; next; acc }; acc }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_explicit_numbered_and_it_examples() {
        for method in ["reduce", "inject"] {
            for op in [".", "&."] {
                for (params, predicate, value) in [
                    ("|acc, i|", "i", "acc + i"),
                    ("", "_2", "_1 + i"),
                    ("", "it", "it + 1"),
                ] {
                    let source = format!(
                        "(1..4){op}{method}(0) do {params}\n  next if {predicate}.odd?\n  {value}\nend\n"
                    );
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert_eq!(
                        found[0].message,
                        "Use `next` with an accumulator argument in a `reduce`."
                    );
                    assert!(found[0].correction.is_none());
                }
            }
            for body in [
                "next acc if i.odd?; acc + i",
                "elems.each_with_index { |elem, i| next if i == 1; acc << elem }; acc",
            ] {
                assert!(offenses(&format!("items.{method}(0) {{ |acc, i| {body} }}")).is_empty());
            }
        }
        assert!(offenses("items.foo(0) { |acc, i| next if i.odd?; acc + i }").is_empty());
    }
}
