//! `Lint/DuplicateElsifCondition` — flags an `elsif` condition that repeats
//! the condition of an earlier branch of the same `if`.
//!
//! Matches RuboCop's semantics for `if`/`elsif` chains: conditions are
//! compared structurally (see [`crate::cops::ast`]). Like RuboCop, the chain
//! continues into an `else` whose only statement is another `if` (plain or
//! modifier): the parser gem makes that `if` the else branch itself.

use ruby_prism::{IfNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast::fingerprint_for_duplicate_condition},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/DuplicateElsifCondition` cop.
pub struct DuplicateElsifCondition;

impl Cop for DuplicateElsifCondition {
    fn name(&self) -> &'static str {
        "Lint/DuplicateElsifCondition"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_if_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(head) = node.as_if_node() else {
            return;
        };
        // Start at the head of a chain: `elsif` nodes are walked from their
        // `if`, and ternaries have no keyword.
        if head.if_keyword_loc().map(|loc| loc.as_slice()) != Some(b"if") {
            return;
        }
        // Upstream uses Array#include? (AST ==), not Set/eql?: Complex
        // components compare across numeric kinds without changing hash keys.
        let mut seen = Vec::new();
        let mut current = Some(head);
        while let Some(branch) = current {
            let condition = branch.predicate();
            let key = fingerprint_for_duplicate_condition(&condition, file);
            if seen
                .iter()
                .any(|previous: &crate::cops::ast::Fingerprint<'_>| {
                    previous.source_buffer_scope_eq(&key, file)
                })
            {
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    condition.location().start_offset(),
                    file,
                );
                // A nested chain is walked from the outer `if` too; report
                // each condition once (RuboCop keeps one offense per range).
                let reported = out
                    .iter()
                    .any(|o| o.cop == self.name() && o.line == line && o.col == col);
                if !reported {
                    out.push(Offense {
                        correction: None,
                        cop: self.name(),
                        severity: self.severity(),
                        message: "Duplicate `elsif` condition detected.".to_string(),
                        line,
                        end_line: file.last_line_of(condition.location()),
                        col,
                    });
                }
            }
            seen.push(key);
            current = next_branch(&branch);
        }
    }
}

/// The branch after `branch`: an `elsif`, or the lone `if` statement of an
/// `else`.
fn next_branch<'pr>(branch: &IfNode<'pr>) -> Option<IfNode<'pr>> {
    let next = branch.subsequent()?;
    match next {
        Node::IfNode { .. } => next.as_if_node(),
        Node::ElseNode { .. } => lone_if(&next.as_else_node()?.statements()?.body()),
        _ => None,
    }
}

/// The `if` (keyword `if`, possibly modifier) that is the only statement.
fn lone_if<'pr>(body: &ruby_prism::NodeList<'pr>) -> Option<IfNode<'pr>> {
    let mut statements = body.iter();
    let only = statements.next()?;
    if statements.next().is_some() {
        return None;
    }
    let inner = only.as_if_node()?;
    (inner.if_keyword_loc()?.as_slice() == b"if").then_some(inner)
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/duplicate_elsif_condition_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateElsifCondition")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_repeated_elsif_condition_with_message() {
        let found = offenses("if x == 1\nelsif x == 2\nelsif x == 1\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 7));
        assert_eq!(found[0].message, "Duplicate `elsif` condition detected.");
    }

    #[test]
    fn flags_subsequent_repeated_elsif_condition() {
        assert_eq!(
            positions("if x == 1\nelsif x == 2\nelsif x == 2\nend\n"),
            vec![(3, 7)]
        );
    }

    #[test]
    fn flags_multiple_repeated_elsif_conditions_once_each() {
        assert_eq!(
            positions("if x == 1\nelsif x == 2\nelsif x == 1\nelsif x == 2\nend\n"),
            vec![(3, 7), (4, 7)]
        );
    }

    #[test]
    fn flags_structurally_equal_conditions() {
        assert_eq!(
            positions("if foo(a, 'b')\n  1\nelsif foo a, \"b\"\n  2\nend\n"),
            vec![(3, 7)]
        );
    }

    #[test]
    fn flags_inside_nested_bodies() {
        let source = "def m\n  if a\n    if b\n    elsif b\n    end\n  elsif c\n  end\nend\n";
        assert_eq!(positions(source), vec![(4, 11)]);
    }

    #[test]
    fn follows_else_whose_only_statement_is_an_if() {
        // RuboCop walks these: the parser gem makes the inner `if` the else
        // branch, and the inner branch can never run.
        assert_eq!(positions("if a\n  x\nelse\n  y if a\nend\n"), vec![(4, 8)]);
        assert_eq!(
            positions("if a\n  x\nelsif b\n  z\nelse\n  y if b\nend\n"),
            vec![(6, 8)]
        );
        assert_eq!(positions("if a\nelse\n  if a\n  end\nend\n"), vec![(3, 6)]);
        assert_eq!(
            positions("if a\nelsif b\nelse\n  if b\n  end\nend\n"),
            vec![(4, 6)]
        );
    }

    #[test]
    fn reports_once_when_nested_chain_is_walked_twice() {
        // The inner chain is walked from the outer `if` and on its own.
        let source = "if a\nelse\n  if b\n  elsif b\n  elsif a\n  end\nend\n";
        assert_eq!(positions(source), vec![(4, 9), (5, 9)]);
    }

    #[test]
    fn stops_at_else_that_is_not_a_lone_if() {
        for source in [
            "if a\nelse\n  y if a\n  z\nend\n",
            "if a\nelse\n  unless a\n  end\nend\n",
            "if a\nelse\n  a ? 1 : 2\nend\n",
            "if a\nelse\n  (y if a)\nend\n",
            "if a\nelse\n  y unless a\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_non_repeated_elsif_conditions() {
        assert!(offenses("if x == 1\nelsif x == 2\nelse\nend\n").is_empty());
    }

    #[test]
    fn accepts_partially_repeated_elsif_conditions() {
        assert!(offenses("if x == 1\nelsif x == 1 && x == 2\nend\n").is_empty());
    }

    #[test]
    fn compares_parser_literal_kinds_and_evaluated_source_lines() {
        assert!(offenses("if :\"\"; elsif :\"\\u{}\"; end").is_empty());
        assert_eq!(positions("if __LINE__; elsif __LINE__; end"), vec![(1, 20)]);
        assert!(offenses("if __LINE__;\nelsif __LINE__; end").is_empty());
    }

    #[test]
    fn uses_array_equality_for_complex_values_and_implicit_regexp_options() {
        for source in [
            "if 1i; elsif 1.0i; end",
            "if 1i; elsif 1ri; end",
            "if [1i]; elsif [1ri]; end",
            "if 0.0i; elsif -0.0i; end",
            "if [0.0i]; elsif [-0.0i]; end",
            "if /x/im; elsif /x/mi; end",
            "if /x/im && a; elsif /x/mi && a; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("if /x/i; elsif /x/m; end").is_empty());
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "if a\nend\nif a\nend\n",
            "foo if a\nbar if a\n",
            "a ? b : c\n",
            "unless a\n  1\nelse\n  2\nend\n",
            "if a\nelsif b\nend\nif b\nend\n",
            "if a\n  if a\n  end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
