//! `Lint/NoReturnInBeginEndBlocks` — returns in assignment begin/end contexts.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NoReturnInBeginEndBlocks;
impl Cop for NoReturnInBeginEndBlocks {
    fn name(&self) -> &'static str {
        "Lint/NoReturnInBeginEndBlocks"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_return_node().is_none() || !assignment_begin(node, cx) {
            return;
        }
        let loc = node.location();
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(loc.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not `return` in `begin..end` blocks in assignment contexts.".into(),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction: None,
        });
    }
}

// Reverse the upstream assignment -> kwbegin -> return search. Each return is
// visited once by the shared typed-child-complete walker; no subtree rescans.
// A scope barrier only invalidates begins ABOVE it: an inner begin below a def
// can still belong to an outer assignment's traversal.
fn assignment_begin(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut eligible_begin = false;
    let mut scope_barrier = false;
    let mut child = node;
    for ancestor in cx.ancestors() {
        if ancestor
            .as_begin_node()
            .is_some_and(|b| b.begin_keyword_loc().is_some())
            && !scope_barrier
        {
            eligible_begin = true;
        }
        if eligible_begin && assignment(ancestor, child) {
            return true;
        }
        // RuboCop's block wraps BOTH send and body, whereas Prism nests the
        // block under its call. A lambda block is a barrier for its receiver
        // and arguments too, but a lambda-named call WITHOUT a block is not.
        scope_barrier |= matches!(ancestor, Node::DefNode { .. } | Node::LambdaNode { .. })
            || ancestor.as_call_node().is_some_and(|c| {
                c.name().as_slice() == b"lambda"
                    && c.block().is_some_and(|b| b.as_block_node().is_some())
            });
        child = ancestor;
    }
    false
}

fn assignment(node: &Node<'_>, child: &Node<'_>) -> bool {
    // &&= itself has no callback, but its casgn target does. Only the
    // constant qualifier is under that target; never the RHS.
    if let Some(n) = node.as_constant_path_and_write_node() {
        return matches!((child, n.target().as_node()),
            (Node::ConstantPathNode { pointer: a, .. }, Node::ConstantPathNode { pointer: b, .. }) if *a == b);
    }
    matches!(
        node,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::LocalVariableOperatorWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::ClassVariableOperatorWriteNode { .. }
            | Node::GlobalVariableOperatorWriteNode { .. }
            | Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::CallOperatorWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::ConstantPathTargetNode { .. }
    )
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/no_return_in_begin_end_blocks_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NoReturnInBeginEndBlocks")
            .collect()
    }
    #[test]
    fn pinned_assignment_operators_and_variable_kinds() {
        for operator in ["=", "+=", "-=", "*=", "/=", "**=", "||="] {
            for lhs in [
                "some_value",
                "@some_value",
                "@@some_value",
                "$some_value",
                "CONST",
            ] {
                let source =
                    format!("{lhs} {operator} begin\n  return 1 if rand(1..2).odd?\n  2\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    "Do not `return` in `begin..end` blocks in assignment contexts."
                );
                assert!(found[0].correction.is_none());
                let source = format!(
                    "{lhs} {operator} begin\n if rand(1..2).odd?\n 'odd number'\n else\n 'even number'\n end\nend\n"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn exact_callback_kinds_not_generic_assignment_contexts() {
        for (source, count) in [
            ("x &&= begin; return 1 if test; value; end", 0),
            ("obj.x = begin; return 1 if test; value; end", 0),
            ("obj[0] = begin; return 1 if test; value; end", 0),
            ("x, y = begin; return 1 if test; value; end", 0),
            ("obj.x ||= begin; return 1 if test; value; end", 1),
            ("obj[0] += begin; return 1 if test; value; end", 1),
            ("(begin; return 1 if test; object; end)::X &&= value", 1),
            ("X &&= begin; return 1 if test; value; end", 0),
            ("(begin; return 1 if test; object; end)::X, y = values", 1),
            ("x = if test; return 1; else; value; end", 0),
        ] {
            assert_eq!(offenses(source).len(), count, "{source}");
        }
    }
    #[test]
    fn scope_barriers_apply_only_between_return_and_selected_begin() {
        for (source, count) in [
            ("x = begin; def f; return 1; end; end", 0),
            ("x = def f; begin; return 1; end; end", 1),
            ("x = begin; def f; begin; return 1; end; end; end", 1),
            ("x = begin; proc { return 1 }; value; end", 1),
            ("x = begin; foo { it; return 1 }; value; end", 1),
            ("x = begin; -> { return 1 }; value; end", 0),
            ("x = begin; obj&.lambda { return 1 }; value; end", 0),
            (
                "x = begin; lambda((return 1 if test)) { _1 }; value; end",
                0,
            ),
            ("x = begin; lambda((return 1 if test)); value; end", 1),
            ("x = lambda { begin; return 1; end }", 1),
            ("x = begin; y = begin; return 1 if test; value; end; end", 1),
        ] {
            assert_eq!(offenses(source).len(), count, "{source}");
        }
    }
    #[test]
    fn return_highlight_ends_with_arguments_not_conditional_or_begin() {
        let found = offenses("x = begin\n return foo(\n  1,\n  2\n ) if test\n value\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 2, 5));
    }
    #[test]
    fn visits_rescue_ensure_and_heredoc_interpolations_without_duplicates() {
        let source = "x = begin\n return 1 if test\n value\nrescue\n return 2 if test\n value\nensure\n return 3 if test\nend\n";
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 2), (5, 2), (8, 2)]
        );
        let found =
            offenses("x = begin; consume(<<TEXT) { -> { return 2 } }; end\n#{return 1}\nTEXT\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 3));
        assert!(
            offenses("consume(<<TEXT) { x = begin; value; end }\n#{return 1}\nTEXT\n").is_empty()
        );
    }
    #[test]
    fn inner_method_and_lambda_returns_are_not_assignment_returns() {
        for body in [
            "def foo; return 1; end",
            "handler = -> { return 1 }; handler.call",
        ] {
            assert!(offenses(&format!("x = begin; {body}; end")).is_empty());
        }
    }
}
