//! `Style/ReturnNilInPredicateMethodDefinition` — predicate nil returns.
//! Upstream's nil-to-false correction is unsafe: detection only.
use super::return_nil::nil_argument;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

#[cfg(test)]
thread_local! { static STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub struct ReturnNilInPredicateMethodDefinition;
impl Cop for ReturnNilInPredicateMethodDefinition {
    fn name(&self) -> &'static str {
        "Style/ReturnNilInPredicateMethodDefinition"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let explicit = node.as_return_node().is_some_and(|ret| {
            (ret.arguments().is_none() || nil_argument(ret.arguments()))
                && cx.ancestors().any(|n| {
                    #[cfg(test)]
                    STEPS.with(|steps| steps.set(steps.get() + 1));
                    n.as_def_node().is_some_and(|def| {
                        def.name().as_slice().ends_with(b"?")
                            && def.body().is_some_and(|body| {
                                let loc = node.location();
                                let inside_body = body.location().start_offset()
                                    <= loc.start_offset()
                                    && body.location().end_offset() >= loc.end_offset();
                                // Upstream visits body descendants, not the body root.
                                inside_body && !same(&body_root(body), node)
                            })
                    })
                })
        });
        let implicit = node.as_nil_node().is_some() && implicit_tail(node, cx);
        if !explicit && !implicit {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: "Return `false` instead of `nil` in predicate methods.".into(),
            correction: None,
        });
    }
}

fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
        && std::mem::discriminant(a) == std::mem::discriminant(b)
}

fn body_root(node: Node<'_>) -> Node<'_> {
    node.as_statements_node()
        .filter(|n| n.body().len() == 1)
        .and_then(|n| n.body().first())
        .unwrap_or(node)
}

/// Only literal nil at method/if/unless/ternary tails is inferred upstream.
/// A missing branch, assignment, case, block or rescue/ensure result is not.
/// Climb the existing bounded context rather than rescanning each method body.
fn implicit_tail(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    let mut begins = 0;
    for parent in cx.ancestors() {
        #[cfg(test)]
        STEPS.with(|steps| steps.set(steps.get() + 1));
        let continues = if let Some(n) = parent.as_statements_node() {
            begins += usize::from(n.body().len() > 1);
            n.body().last().is_some_and(|n| same(&n, child))
        } else if let Some(n) = parent.as_parentheses_node() {
            // Parentheses and their multi-statement body are ONE RuboCop begin.
            begins += usize::from(
                !child
                    .as_statements_node()
                    .is_some_and(|n| n.body().len() > 1),
            );
            n.body().is_some_and(|n| same(&n, child))
        } else if let Some(n) = parent.as_if_node() {
            begins = 0;
            n.statements().is_some_and(|n| same(&n.as_node(), child))
                || n.subsequent().is_some_and(|n| same(&n, child))
        } else if let Some(n) = parent.as_unless_node() {
            begins = 0;
            n.statements().is_some_and(|n| same(&n.as_node(), child))
                || n.else_clause().is_some_and(|n| same(&n.as_node(), child))
        } else if let Some(n) = parent.as_else_node() {
            n.statements().is_some_and(|n| same(&n.as_node(), child))
        } else if let Some(n) = parent.as_def_node() {
            return n.name().as_slice().ends_with(b"?")
                && n.body().is_some_and(|n| same(&n, child));
        } else {
            false
        };
        // Upstream looks through at most one begin at each tail, not arbitrary
        // nested groups; this is a matcher, not general return-value analysis.
        if !continues || begins > 1 {
            return false;
        }
        child = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/return_nil_in_predicate_method_definition_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<crate::offense::Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == ReturnNilInPredicateMethodDefinition.name())
            .collect()
    }
    #[test]
    fn all_pinned_examples_and_unsafe_corrections_are_detection_only() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/return_nil_in_predicate_method_definition_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 32);
        assert_eq!(
            cases.iter().filter(|c| c["correction"].is_string()).count(),
            17
        );
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found = offenses(source);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{source}");
            for o in &found {
                assert_eq!(
                    o.message,
                    "Return `false` instead of `nil` in predicate methods."
                );
            }
            let with_fixes = correction_offenses(&ReturnNilInPredicateMethodDefinition, source);
            assert_eq!(with_fixes.len(), found.len());
            assert!(with_fixes.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn conditional_tails_include_unless_elsif_ternary_and_grouping() {
        for (source, line, col) in [
            ("def foo?\n  unless x\n    nil\n  end\nend\n", 3, 5),
            (
                "def self.foo?\n  if x\n    true\n  elsif y\n    nil\n  end\nend\n",
                5,
                5,
            ),
            ("def foo? = x ? nil : true\n", 1, 16),
            ("def foo?\n  (work; nil)\nend\n", 2, 10),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
        }
    }
    #[test]
    fn grouped_begin_nodes_are_not_arbitrarily_flattened() {
        for body in [
            "work; (nil)",
            "x ? ((nil)) : true",
            "(work; (nil))",
            "work; return (nil)",
        ] {
            assert!(
                offenses(&format!("def foo?\n{body}\nend\n")).is_empty(),
                "{body}"
            );
        }
        let found = offenses("def foo?\n  (return nil)\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 4));
    }
    #[test]
    fn explicit_descendants_and_rescue_ensure_scope_match_upstream() {
        // The pin deliberately searches descendants, including nested methods
        // and lambdas, but not a ReturnNode that is the method body's root.
        for body in [
            "work; return",
            "work; return nil",
            "proc { return nil }",
            "lambda { return nil }",
            "-> { return nil }",
            "def inner; return nil; end",
            "begin; return nil; rescue StandardError; nil; ensure; nil; end",
            "return nil\nrescue\nfalse",
            "true\nensure\nreturn nil",
        ] {
            let source = format!("def foo?\n{body}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(
                correction_offenses(&ReturnNilInPredicateMethodDefinition, &source)
                    .iter()
                    .all(|o| o.correction.is_none())
            );
        }
        for body in [
            "return",
            "return nil",
            "nil\nrescue\nfalse",
            "true\nrescue\nnil",
            "true\nensure\nnil",
            "proc { nil }",
            "def inner; nil; end",
        ] {
            assert!(
                offenses(&format!("def foo?\n{body}\nend\n")).is_empty(),
                "{body}"
            );
        }
    }
    #[test]
    fn wide_nil_and_return_sources_have_bounded_context_work() {
        use crate::pipeline::lint_str_with;
        let source = format!(
            "def foo?\n{}nil\nend\n",
            "nil\nreturn nil if x\n".repeat(4000)
        );
        STEPS.with(|s| s.set(0));
        assert_eq!(
            lint_str_with(&[&ReturnNilInPredicateMethodDefinition], &source).len(),
            4001
        );
        STEPS.with(|s| assert!(s.get() <= 4000 * 10 + 3, "{}", s.get()));
    }
    #[test]
    fn non_tail_nil_and_unproven_nil_values_are_not_inferred() {
        for body in [
            "nil; true",
            "x = nil",
            "consume(nil)",
            "x&.consume(nil)",
            "x && nil",
            "case x; when 1; nil; end",
            "begin; nil; end",
            "-> { nil }",
            "",
            "return nil",
        ] {
            assert!(
                offenses(&format!("def foo?\n{body}\nend\n")).is_empty(),
                "{body}"
            );
        }
    }
}
