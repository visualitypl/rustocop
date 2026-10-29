//! `Lint/EnsureReturn` — flags `return` inside an `ensure` clause, which
//! silently discards any exception in flight.
//!
//! Matches RuboCop's semantics: every `return` in the `ensure` body is
//! reported, including inside plain blocks and procs (they return from the
//! method too), but not inside a nested method definition or a lambda
//! (`-> {}`, `lambda {}`), which return from themselves.

use ruby_prism::{CallNode, DefNode, EnsureNode, LambdaNode, Node, ReturnNode, Visit};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/EnsureReturn` cop.
pub struct EnsureReturn;

impl Cop for EnsureReturn {
    fn name(&self) -> &'static str {
        "Lint/EnsureReturn"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(statements) = node.as_ensure_node().and_then(|ensure| ensure.statements()) else {
            return;
        };
        let mut search = ReturnSearch {
            cop: self,
            file,
            out,
        };
        search.visit_statements_node(&statements);
    }
}

/// Reports the `return`s that leave the method from an `ensure` body.
struct ReturnSearch<'a> {
    cop: &'a EnsureReturn,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
}

impl<'pr> Visit<'pr> for ReturnSearch<'_> {
    fn visit_return_node(&mut self, node: &ReturnNode<'pr>) {
        let (line, col) = self.file.line_col(node.location().start_offset());
        self.out.push(Offense {
            correction: None,
            cop: self.cop.name(),
            severity: self.cop.severity(),
            message: "Do not return from an `ensure` block.".to_string(),
            line,
            end_line: self.file.last_line_of(node.location()),
            col,
        });
        ruby_prism::visit_return_node(self, node);
    }

    // Inner scopes: a `return` there leaves the method or lambda itself.
    fn visit_def_node(&mut self, _node: &DefNode<'pr>) {}

    fn visit_lambda_node(&mut self, _node: &LambdaNode<'pr>) {}

    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        // `lambda { }` (any receiver, as RuboCop's `BlockNode#lambda?`). The
        // parser gem's block wraps the whole call, so skip all of it.
        let lambda_block = node.name().as_slice() == b"lambda"
            && node
                .block()
                .is_some_and(|block| block.as_block_node().is_some());
        if !lambda_block {
            ruby_prism::visit_call_node(self, node);
        }
    }

    // A nested `ensure` reports its own returns.
    fn visit_ensure_node(&mut self, _node: &EnsureNode<'pr>) {}
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/ensure_return_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EnsureReturn")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_return_in_ensure_with_message() {
        let found = offenses("begin\n  something\nensure\n  file.close\n  return\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (5, 3));
        assert_eq!(found[0].message, "Do not return from an `ensure` block.");
    }

    #[test]
    fn flags_return_with_values_in_ensure() {
        assert_eq!(
            positions("begin\n  foo\nensure\n  return baz\nend\n"),
            vec![(4, 3)]
        );
        assert_eq!(
            positions("begin\n  something\nensure\n  do_something\n  return foo, bar\nend\n"),
            vec![(5, 3)]
        );
    }

    #[test]
    fn flags_return_inside_a_block_in_ensure() {
        assert_eq!(
            positions("def foo\n  do_something\nensure\n  [1, 2].each { return }\nend\n"),
            vec![(4, 17)]
        );
        assert_eq!(
            positions("def foo\nensure\n  handler = proc { return 1 }\nend\n"),
            vec![(3, 20)]
        );
    }

    #[test]
    fn flags_nested_and_conditional_returns() {
        assert_eq!(
            positions("def foo\nensure\n  if a\n    return\n  end\n  b && (return 1)\nend\n"),
            vec![(4, 5), (6, 9)]
        );
        // A return in a nested `ensure` is reported once.
        assert_eq!(
            positions("begin\nensure\n  begin\n    a\n  ensure\n    return\n  end\nend\n"),
            vec![(6, 5)]
        );
        // The body of a nested `begin` is still the outer `ensure`'s.
        assert_eq!(
            positions("begin\nensure\n  begin\n    return\n  ensure\n    a\n  end\nend\n"),
            vec![(4, 5)]
        );
    }

    #[test]
    fn accepts_return_outside_ensure() {
        assert!(offenses("begin\n  something\n  return\nensure\n  file.close\nend\n").is_empty());
        assert!(
            offenses("begin\n  a\nrescue\n  return\nelse\n  return\nensure\n  b\nend\n").is_empty()
        );
    }

    #[test]
    fn accepts_empty_ensure() {
        assert!(offenses("begin\n  something\nensure\nend\n").is_empty());
    }

    #[test]
    fn accepts_return_inside_a_lambda_in_ensure() {
        assert!(
            offenses(
                "def foo\n  do_something\nensure\n  handler = -> { return 42 }\n  handler.call\nend\n"
            )
            .is_empty()
        );
        assert!(offenses("def foo\nensure\n  handler = lambda { return 42 }\nend\n").is_empty());
        assert!(
            offenses("def foo\nensure\n  handler = lambda do |x|\n    return x\n  end\nend\n")
                .is_empty()
        );
    }

    #[test]
    fn accepts_return_inside_a_method_definition_in_ensure() {
        assert!(
            offenses("def foo\n  do_something\nensure\n  def bar\n    return 1\n  end\nend\n")
                .is_empty()
        );
        assert!(offenses("def foo\nensure\n  def self.bar\n    return 1\n  end\nend\n").is_empty());
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "def foo\nensure\n  next\nend\n",
            "def foo\nensure\n  break_out\nend\n",
            "def foo\nensure\n  returns\nend\n",
            "def foo\n  return\nensure\n  b\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
