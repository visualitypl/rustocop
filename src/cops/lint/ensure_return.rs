//! `Lint/EnsureReturn` — flags `return` inside an `ensure` clause, which
//! silently discards any exception in flight.
//!
//! Matches RuboCop's semantics: every `return` in the `ensure` body is
//! reported, including inside plain blocks and procs (they return from the
//! method too), but not inside a nested method definition or a lambda
//! (`-> {}`, `lambda {}`), which return from themselves.

use ruby_prism::Node;

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

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_return_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_return_node().is_none() {
            return;
        }
        for ancestor in cx.ancestors() {
            if ancestor.as_def_node().is_some()
                || ancestor.as_lambda_node().is_some()
                || ancestor.as_call_node().is_some_and(|call| {
                    // The parser AST's lambda block wraps the callee too.
                    call.name().as_slice() == b"lambda"
                        && call.block().is_some_and(|b| b.as_block_node().is_some())
                })
            {
                return;
            }
            if ancestor.as_ensure_node().is_some() {
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    node.location().start_offset(),
                    file,
                );
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Do not return from an `ensure` block.".to_string(),
                    line,
                    end_line: file.last_line_of(node.location()),
                    col,
                });
                return;
            }
        }
    }
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
    fn inner_ensure_has_its_own_scope_even_in_lambda() {
        assert_eq!(
            positions("def f\nensure\n  -> { begin; ensure; return 1; end }\nend\n"),
            vec![(3, 23)]
        );
        assert!(offenses("def f\nensure\n  lambda(return) { x }\nend\n").is_empty());
        assert_eq!(
            positions("def f\nensure\n  begin; return; ensure; x; end\nend\n"),
            vec![(3, 10)]
        );
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
