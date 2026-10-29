//! `Style/IfWithBooleanLiteralBranches` — detection only: predicate methods
//! and comparison operators are not guaranteed to return actual booleans.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};
pub struct IfWithBooleanLiteralBranches;
impl Cop for IfWithBooleanLiteralBranches {
    fn name(&self) -> &'static str {
        "Style/IfWithBooleanLiteralBranches"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (predicate, body, other, keyword) = if let Some(n) = node.as_if_node() {
            (
                n.predicate(),
                n.statements(),
                n.subsequent(),
                n.if_keyword_loc(),
            )
        } else if let Some(n) = node.as_unless_node() {
            (
                n.predicate(),
                n.statements(),
                n.else_clause().map(|n| n.as_node()),
                Some(n.keyword_loc()),
            )
        } else {
            return;
        };
        let Some(a) = boolean_branch(body) else {
            return;
        };
        let Some(b) = other
            .and_then(|n| n.as_else_node())
            .and_then(|n| boolean_branch(n.statements()))
        else {
            return;
        };
        let predicate_end = predicate.location().end_offset();
        if a == b || !returns_boolean(predicate) {
            return;
        }
        // RuboCop has no ElseNode or single-child StatementsNode. Its
        // immediate semantic parent also governs nested elsif branch bodies.
        if cx
            .ancestors()
            .find(|p| {
                p.as_else_node().is_none()
                    && !p.as_statements_node().is_some_and(|s| s.body().len() == 1)
            })
            .and_then(|p| p.as_if_node())
            .and_then(|p| p.if_keyword_loc())
            .is_some_and(|l| l.as_slice() == b"elsif")
        {
            return;
        }
        let (start, end, message) = if let Some(keyword) = keyword {
            let word = std::str::from_utf8(keyword.as_slice()).unwrap();
            let message = if word == "elsif" {
                "Use `else` instead of redundant `elsif` with boolean literal branches.".to_owned()
            } else {
                format!("Remove redundant `{word}` with boolean literal branches.")
            };
            (keyword.start_offset(), keyword.end_offset(), message)
        } else {
            (
                predicate_end,
                node.location().end_offset(),
                "Remove redundant ternary operator with boolean literal branches.".into(),
            )
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: None,
        });
    }
}
fn boolean_branch(statements: Option<StatementsNode<'_>>) -> Option<bool> {
    let statements = statements?;
    if statements.body().len() != 1 {
        return None;
    }
    let node = statements.body().iter().next()?;
    match node {
        Node::TrueNode { .. } => Some(true),
        Node::FalseNode { .. } => Some(false),
        _ => None,
    }
}
fn returns_boolean(node: Node<'_>) -> bool {
    // Iterative, bounded by the condition subtree. AND intentionally checks
    // only its RHS, following the pinned heuristic (not a correction proof).
    let mut pending = vec![node];
    while let Some(node) = pending.pop() {
        #[cfg(test)]
        PREDICATE_VISITS.with(|n| n.set(n.get() + 1));
        if let Some(p) = node.as_parentheses_node() {
            let Some(first) = p
                .body()
                .and_then(|b| b.as_statements_node())
                .and_then(|b| b.body().iter().next())
            else {
                return false;
            };
            pending.push(first);
        } else if let Some(n) = node.as_or_node() {
            pending.extend([n.left(), n.right()]);
        } else if let Some(n) = node.as_and_node() {
            pending.push(n.right());
        } else if let Some(call) = node.as_call_node() {
            if call.block().is_some_and(|b| b.as_block_node().is_some())
                || call
                    .call_operator_loc()
                    .is_some_and(|l| l.as_slice() == b"&.")
            {
                return false;
            }
            let name = call.name().as_slice();
            if matches!(name, b"infinite?" | b"nonzero?") {
                return false;
            }
            let double_negative = name == b"!"
                && call.arguments().is_none()
                && call.block().is_none()
                && call
                    .receiver()
                    .and_then(|r| r.as_call_node())
                    .is_some_and(|c| {
                        c.name().as_slice() == b"!"
                            && c.arguments().is_none()
                            && c.block().is_none()
                            && !c.call_operator_loc().is_some_and(|l| l.as_slice() == b"&.")
                    });
            if !(name.ends_with(b"?")
                || matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<")
                || double_negative)
            {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}
#[cfg(test)]
thread_local! { static PREDICATE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/if_with_boolean_literal_branches_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn found(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        crate::pipeline::lint_str_with(&[&IfWithBooleanLiteralBranches], source)
    }
    #[test]
    fn predicate_blocks_safe_navigation_and_allowed_methods_are_not_boolean_proofs() {
        for condition in [
            "foo? { effect }",
            "foo? do; effect; end",
            "obj&.foo?",
            "num.infinite?",
            "num.nonzero?",
            "obj.<=> other",
            "!value",
            "begin; foo?; end",
            "(value; foo?)",
        ] {
            let source = format!("if {condition}\n true\nelse\n false\nend\n");
            assert!(found(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn double_negation_requires_argumentless_calls() {
        for condition in [
            "obj.!(1).!",
            "obj.!.!(1)",
            "obj.!(&block).!",
            "obj.!.!(&block)",
            "obj.!(&nil).!",
            "obj.!.!(&nil)",
        ] {
            let source = format!("if {condition}; true; else; false; end");
            assert!(found(&source).is_empty(), "{source}");
        }
        for condition in ["!!obj", "obj.!.!", "obj.!().!()"] {
            let source = format!("if {condition}; true; else; false; end");
            let offenses = found(&source);
            assert_eq!(offenses.len(), 1, "{source}");
            assert_eq!((offenses[0].line, offenses[0].col), (1, 1));
            assert_eq!(
                offenses[0].message,
                "Remove redundant `if` with boolean literal branches."
            );
        }
    }
    #[test]
    fn branches_must_be_opposite_single_literals_without_side_effects() {
        for source in [
            "if a?; true; else; true; end",
            "if a?; false; else; false; end",
            "true if a?",
            "if a?; effect; true; else; false; end",
            "if a?; true; else; effect; false; end",
            "if a?; return true; else; return false; end",
            "if a?; (true); else; false; end",
            "if a?; begin; true; ensure; effect; end; else; false; end",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        let source = "if obj.predicate? # header\n true # branch\nelse\n false\nend\n";
        let found = found(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(
            found[0].message,
            "Remove redundant `if` with boolean literal branches."
        );
    }
    #[test]
    fn boolean_heuristic_follows_parenthesis_first_child_and_asymmetric_logical_operands() {
        for (condition, expected) in [
            ("foo && bar?", true),
            ("foo? && bar", false),
            ("foo || bar?", false),
            ("foo? || bar?", true),
            ("(foo?; unknown)", true),
            ("(unknown; foo?)", false),
            ("foo?(&block)", true),
            ("obj.===(other)", true),
            ("obj&.==(other)", false),
            ("!!obj&.foo?", true),
            ("foo.infinite?(other)", false),
            ("CONST.ready?", true),
        ] {
            let source = format!("{condition} ? true : false\n");
            assert_eq!(found(&source).len(), usize::from(expected), "{source}");
        }
    }
    #[test]
    fn single_elsif_is_reported_but_multiple_elsif_and_explicit_nested_else_differ() {
        let source = "if a\n true\nelsif b?\n true\nelse\n false\nend\n";
        let offenses = found(source);
        assert_eq!(offenses.len(), 1);
        assert_eq!((offenses[0].line, offenses[0].col), (3, 1));
        assert_eq!(
            offenses[0].message,
            "Use `else` instead of redundant `elsif` with boolean literal branches."
        );
        assert!(found("if a; true; elsif b?; true; elsif c?; true; else; false; end").is_empty());
        assert_eq!(
            found("if a; true; else; if c?; true; else; false; end; end").len(),
            1
        );
    }
    #[test]
    fn elsif_semantic_parent_includes_single_statement_branches_but_not_multi_statements() {
        for source in [
            "if a; work; elsif b; if c?; true; else; false; end; end",
            "if a; work; elsif b; work; else; if c?; true; else; false; end; end",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        assert_eq!(
            found("if a; work; elsif b; side_effect; if c?; true; else; false; end; end").len(),
            1
        );
    }
    #[test]
    fn predicate_work_is_bounded_over_wide_siblings() {
        let source = "if foo? || bar && baz?\n true\nelse\n false\nend\n".repeat(3000);
        PREDICATE_VISITS.with(|n| n.set(0));
        assert_eq!(found(&source).len(), 3000);
        PREDICATE_VISITS.with(|n| assert_eq!(n.get(), 12_000));
    }
    #[test]
    fn all_pinned_diagnostics_and_unsafe_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_with_boolean_literal_branches_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 93);
        assert_eq!(
            cases.iter().filter(|c| c["correction"].is_string()).count(),
            68
        );
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == IfWithBooleanLiteralBranches.name())
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "spec {}",
                c["spec_line"]
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "spec {}",
                c["spec_line"]
            );
            assert!(
                crate::cops::testing::correction_offenses(&IfWithBooleanLiteralBranches, source)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
    }
}
