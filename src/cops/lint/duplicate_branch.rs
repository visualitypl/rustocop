//! `Lint/DuplicateBranch` — repeated structural bodies in branching statements.

use std::collections::HashSet;

use ruby_prism::{Location, Node, StatementsNode};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// Fixed defaults: literal, constant and else branches are all considered.
/// Upstream provides no autocorrection; duplicate effects cannot be merged safely.
pub struct DuplicateBranch;

impl Cop for DuplicateBranch {
    fn name(&self) -> &'static str {
        "Lint/DuplicateBranch"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut branches = Vec::new();
        if let Some(n) = node.as_if_node() {
            if n.if_keyword_loc().is_some_and(|k| k.as_slice() == b"elsif") {
                return;
            }
            if_branches(node, &mut branches);
        } else if let Some(n) = node.as_unless_node() {
            // RuboCop normalizes unless by truth value, including its reporting
            // quirk: the source-body duplicate is highlighted at `else`.
            if let Some(e) = n.else_clause() {
                push(&mut branches, e.statements(), n.location());
                push(&mut branches, n.statements(), e.else_keyword_loc());
            }
        } else if let Some(n) = node.as_case_node() {
            for clause in n.conditions().iter() {
                let clause = clause.as_when_node().unwrap();
                push(&mut branches, clause.statements(), clause.location());
            }
            if let Some(e) = n.else_clause() {
                push(&mut branches, e.statements(), e.else_keyword_loc());
            }
        } else if let Some(n) = node.as_case_match_node() {
            for clause in n.conditions().iter() {
                let clause = clause.as_in_node().unwrap();
                push(&mut branches, clause.statements(), clause.location());
            }
            if let Some(e) = n.else_clause() {
                push(&mut branches, e.statements(), e.else_keyword_loc());
            }
        } else if let Some(n) = node.as_begin_node() {
            // Prism owns the rescue chain and its else under Begin, including
            // implicit method/block rescue bodies. Visit it once, not once per
            // linked RescueNode (which would rescan suffixes quadratically).
            let mut clause = n.rescue_clause();
            if clause.is_none() {
                return;
            }
            while let Some(rescue) = clause {
                let range = rescue.location();
                let end = ast::source_end(&rescue.as_node());
                push_range(
                    &mut branches,
                    rescue.statements(),
                    range.start_offset()..end,
                );
                clause = rescue.subsequent();
            }
            if let Some(e) = n.else_clause() {
                push(&mut branches, e.statements(), e.else_keyword_loc());
            }
        } else {
            return;
        }
        let mut previous = HashSet::new();
        for (body, range) in branches {
            let key = ast::fingerprint_for_branch(&body, file);
            if previous.insert(key) {
                continue;
            }
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Duplicate branch body detected.".into(),
                line,
                col,
                end_line: file.line_of(range.end.saturating_sub(1).max(range.start)),
                correction: None,
            });
        }
    }
}

type Branch<'pr> = (Node<'pr>, std::ops::Range<usize>);

fn push<'pr>(
    branches: &mut Vec<Branch<'pr>>,
    body: Option<StatementsNode<'pr>>,
    range: Location<'pr>,
) {
    push_range(branches, body, range.start_offset()..range.end_offset());
}

fn push_range<'pr>(
    branches: &mut Vec<Branch<'pr>>,
    body: Option<StatementsNode<'pr>>,
    range: std::ops::Range<usize>,
) {
    if let Some(body) = body {
        // Statements is a Prism-only wrapper around single expressions. Empty
        // branches are absent, not explicit Ruby nil nodes (`branches.compact`).
        let nodes = body.body();
        if nodes.len() == 1 {
            branches.push((nodes.first().unwrap(), range));
        } else if !nodes.is_empty() {
            branches.push((body.as_node(), range));
        }
    }
}

fn if_branches<'pr>(node: &Node<'pr>, branches: &mut Vec<Branch<'pr>>) {
    let mut n = node.as_if_node().unwrap();
    let mut elsif_indices = Vec::new();
    let end = loop {
        let index = branches.len();
        push(branches, n.statements(), n.location());
        if index < branches.len() && n.if_keyword_loc().is_some_and(|k| k.as_slice() == b"elsif") {
            elsif_indices.push(index);
        }
        let Some(next) = n.subsequent() else {
            break n
                .statements()
                .map_or(n.predicate().location().end_offset(), |s| {
                    s.location().end_offset()
                });
        };
        if let Some(e) = next.as_else_node() {
            let range = if n.if_keyword_loc().is_none() {
                // Ternary: report the expression rather than the colon.
                e.statements().unwrap().location()
            } else {
                e.else_keyword_loc()
            };
            push(branches, e.statements(), range);
            break e
                .statements()
                .map_or(e.else_keyword_loc().end_offset(), |s| {
                    s.location().end_offset()
                });
        }
        n = next.as_if_node().unwrap();
    };
    // Parser's elsif source ranges include subsequent clauses but not the
    // outer conditional's closing `end`. Compute that shared tail once.
    for index in elsif_indices {
        branches[index].1.end = end;
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/lint/duplicate_branch_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateBranch")
            .collect()
    }

    #[test]
    fn flags_repeated_elsif_and_else_bodies_once_at_their_owner() {
        let found =
            offenses("if foo\n  work\nelsif bar\n  other\nelsif baz\n  work\nelse\n  other\nend\n");
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.end_line))
                .collect::<Vec<_>>(),
            [(5, 1, 8), (7, 1, 7)]
        );
        for o in found {
            assert_eq!(o.message, "Duplicate branch body detected.");
            assert!(o.correction.is_none());
        }
    }

    #[test]
    fn flags_case_when_in_rescue_and_else_at_owning_clause() {
        for (source, expected) in [
            (
                "case x\nwhen foo\n  work\nwhen bar\n  work\nelse\n  work\nend",
                vec![(4, 1, 5), (6, 1, 6)],
            ),
            (
                "case x\nin [a]\n  work\nin [b]\n  work\nelse\n  work\nend",
                vec![(4, 1, 5), (6, 1, 6)],
            ),
            (
                "begin\n  work\nrescue Foo\n  handle\nrescue Bar\n  handle\nelse\n  handle\nend",
                vec![(5, 1, 6), (7, 1, 7)],
            ),
            (
                "def f\n  work\nrescue Foo\n  handle\nrescue Bar\n  handle\nend",
                vec![(5, 1, 6)],
            ),
        ] {
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line))
                    .collect::<Vec<_>>(),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn every_pinned_spec_source_and_control_has_exact_fixed_policy_findings() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_branch.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 546);
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let found = offenses(source);
            let actual: Vec<_> = found
                .iter()
                .map(|o| serde_json::json!([o.line, o.col, o.end_line, o.message]))
                .collect();
            assert_eq!(serde_json::json!(actual), row["findings"], "{row}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn all_lead_probes_and_pinned_structural_controls_have_exact_findings() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_branch_structure.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 97);
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{row}");
            let found = offenses(source);
            let actual: Vec<_> = found
                .iter()
                .map(|o| serde_json::json!([o.line, o.col, o.end_line, o.message]))
                .collect();
            assert_eq!(serde_json::json!(actual), row["findings"], "{row}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn parser_equivalent_conditional_lambda_and_setter_forms() {
        for (a, b) in [
            ("if a; b; else; c; end", "a ? b : c"),
            ("if a then b else c end", "if a; b; else; c; end"),
            ("->(a) { a }", "lambda { |a| a }"),
            ("->(a = 1; b) { a; b }", "lambda { |a = 1; b| a; b }"),
            ("-> {}", "lambda do end"),
            ("obj[1] = 2", "obj.[]=(1, 2)"),
            ("obj[1, 2] = 3", "obj.[]=(1, 2, 3)"),
        ] {
            let source = format!("if x\n  {a}\nelse\n  {b}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1), "{source}");
            assert_eq!(found[0].message, "Duplicate branch body detected.");
            assert!(found[0].correction.is_none());
        }
        for (a, b) in [
            ("if a; b; else; c; end", "a ? c : b"),
            ("if a; if b; c; end; end", "if a; elsif b; c; end"),
            ("if a; b; else; c; end", "unless a; b; else; c; end"),
            ("->(a) { a }", "lambda { |b| b }"),
            ("->(a) { a }", "obj.lambda { |a| a }"),
            ("->(a) { a }", "lambda(1) { |a| a }"),
            ("->(a) { a }", "proc { |a| a }"),
            ("->(a) { a }", "lambda { |a; b| a }"),
            ("obj[1] = 2", "obj.[]=(2, 1)"),
            ("obj[1] = 2", "obj&.[]=(1, 2)"),
        ] {
            let source = format!("if x\n  {a}\nelse\n  {b}\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn normalizes_sequences_recursively_but_keeps_single_grouping_and_kwbegin() {
        for (a, b) in [
            ("foo { a; b }", "foo { (a; b) }"),
            ("foo { bar { a; b } }", "foo { bar { (a; b) } }"),
            ("[-> { a; b }]", "[-> { (a; b) }]"),
            (
                "foo { if a; b; c; else; d; end }",
                "foo { if a; (b; c); else; d; end }",
            ),
        ] {
            let source = format!("if x\n  {a}\nelse\n  {b}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1), "{source}");
            assert_eq!(found[0].message, "Duplicate branch body detected.");
            assert!(found[0].correction.is_none());
        }
        for (a, b) in [
            ("foo { a }", "foo { (a) }"),
            ("foo { a; b }", "foo { ((a; b)) }"),
            ("foo { a; b }", "foo { begin; a; b; end }"),
            ("foo { a; b }", "foo { b; a }"),
            ("foo { a && (b && c) }", "foo { a && b && c }"),
        ] {
            let source = format!("if x\n  {a}\nelse\n  {b}\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    fn assert_pairs(equivalent: &[(&str, &str)], distinct: &[(&str, &str)]) {
        for (pairs, duplicate) in [(equivalent, true), (distinct, false)] {
            for (a, b) in pairs {
                let source = format!("if x\n  {a}\nelse\n  {b}\nend\n");
                let parsed = crate::parser::parse(source.as_bytes());
                assert_eq!(parsed.errors().count(), 0, "{source}");
                let found = offenses(&source);
                assert_eq!(found.len(), usize::from(duplicate), "{source}");
                if duplicate {
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
                    assert_eq!(found[0].message, "Duplicate branch body detected.");
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }

    #[test]
    fn canonical_conditionals_preserve_truth_value_slots_and_absent_vs_nil() {
        assert_pairs(
            &[
                ("unless a; b; else; c; end", "a ? c : b"),
                ("unless a; b; end", "if a; else; b; end"),
                ("if a; b; else; end", "if a; b; end"),
                ("if a; elsif b; c; end", "if a; else; if b; c; end; end"),
                (
                    "if a; b; elsif c; d; end",
                    "if a; b; else; if c; d; end; end",
                ),
                ("unless a; b; end", "b unless a"),
                ("unless a; b; else; end", "if a; else; b; end"),
                ("unless a; else; b; end", "if a; b; end"),
                ("unless a; end", "if a; else; end"),
                (
                    "unless a; b; c; else; d; e; end",
                    "if a; d; e; else; b; c; end",
                ),
                ("[unless a; b; else; c; end]", "[a ? c : b]"),
                (
                    "if a; b; elsif c; d; elsif e; f; end",
                    "if a; b; else; if c; d; else; if e; f; end; end; end",
                ),
            ],
            &[
                ("unless a; b; else; c; end", "if a; b; else; c; end"),
                ("if a; b; else; end", "if a; b; else; nil; end"),
                ("unless a; b; end", "if a; nil; else; b; end"),
                ("unless a; else; b; end", "if a; b; else; nil; end"),
                ("if a; b; end", "if a; else; b; end"),
                ("if a; if b; c; end; end", "if a; elsif b; c; end"),
                (
                    "if a; elsif b; c; end",
                    "if a; else; begin; if b; c; end; end; end",
                ),
                (
                    "unless a; b; c; else; d; e; end",
                    "if a; d; e; else; c; b; end",
                ),
                ("unless a; b; end", "b unless (a)"),
                ("if a; b; end", "if a; (b); end"),
            ],
        );
    }

    #[test]
    fn canonical_loops_keep_kind_test_position_and_child_structure() {
        assert_pairs(
            &[
                ("while a; b; end", "b while a"),
                ("until a; b; end", "b until a"),
                ("while a; b; end", "while a do b end"),
                ("until a; b; end", "until a do b end"),
                ("for a in xs; f(a); end", "for a in xs do f(a) end"),
                ("while a; end", "while a do end"),
                ("until a; end", "until a do end"),
                (
                    "for a, b in xs; f(a, b); end",
                    "for a, b in xs do f(a, b) end",
                ),
                ("foo { while a; b; end }", "foo { b while a }"),
                ("begin; b; end while a", "begin b end while a"),
            ],
            &[
                ("while a; b; end", "until a; b; end"),
                ("while a; begin; b; end; end", "begin; b; end while a"),
                ("until a; begin; b; end; end", "begin; b; end until a"),
                ("while a; end", "while a; nil; end"),
                ("until a; end", "until a; nil; end"),
                ("while a; b; end", "while b; a; end"),
                ("while a; b; end", "while (a); b; end"),
                ("until a; b; c; end", "until a; c; b; end"),
                (
                    "for a, b in xs; f(a, b); end",
                    "for b, a in xs; f(a, b); end",
                ),
                ("for a in xs; f(a); end", "for a in ys; f(a); end"),
                ("for a in xs; f(a); end", "for a in xs; f; end"),
                ("for a in xs; end", "for a in xs; nil; end"),
            ],
        );
    }

    #[test]
    fn canonical_method_definitions_keep_receiver_name_parameters_and_body() {
        assert_pairs(
            &[
                ("def f(a); a; end", "def f(a) = a"),
                ("def self.f(a); a; end", "def self.f(a) = a"),
                ("def f(a); a; end", "def f a; a; end"),
                ("def f; a; end", "def f(); a; end"),
                ("def f; nil; end", "def f = nil"),
                ("def obj.f; a; end", "def (obj).f = a"),
                (
                    "def f(a = 1, *b, c:, **d, &e); [a, b, c, d, e]; end",
                    "def f(a = 1, *b, c:, **d, &e) = [a, b, c, d, e]",
                ),
                ("def obj.f(...); g(...); end", "def obj.f(...) = g(...)"),
                ("foo { def self.f(a); a; end }", "foo { def self.f(a) = a }"),
            ],
            &[
                ("def f(a); a; end", "def g(a) = a"),
                ("def f(a); a; end", "def self.f(a) = a"),
                ("def self.f(a); a; end", "def obj.f(a) = a"),
                ("def f(a); a; end", "def f(b) = b"),
                ("def f(a); a; end", "def f(a = 1) = a"),
                ("def f(a); a; end", "def f(*a) = a"),
                ("def f(a); a; end", "def f(a:) = a"),
                ("def f(a, b); a; end", "def f(b, a) = a"),
                ("def f(a); a; end", "def f(a, &block) = a"),
                ("def f; end", "def f = nil"),
                ("def self.f; end", "def self.f = nil"),
                ("def f(a); a; end", "def f(a) = (a)"),
                ("def f; a; b; end", "def f = (b; a)"),
                ("def obj.f; a; end", "def ((obj)).f = a"),
            ],
        );
    }

    fn assert_review2_group(group: &str) {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/duplicate_branch_review2.json"
        ))
        .unwrap();
        let rows: Vec<_> = rows.iter().filter(|row| row["origin"] == group).collect();
        assert!(!rows.is_empty());
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{row}");
            let found = offenses(source);
            let actual: Vec<_> = found
                .iter()
                .map(|o| serde_json::json!([o.line, o.col, o.end_line, o.message]))
                .collect();
            assert_eq!(serde_json::json!(actual), row["findings"], "{row}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn review2_explicit_begin_preserves_sequence_grouping() {
        assert_review2_group("grouping");
    }

    #[test]
    fn review2_case_clause_layout_and_empty_else_follow_parser_slots() {
        assert_review2_group("case");
    }

    #[test]
    fn review2_implicit_and_modifier_rescue_preserve_clause_structure() {
        assert_review2_group("rescue");
    }

    #[test]
    fn large_case_and_independent_rescue_scopes_keep_each_duplicate_once() {
        let mut source = "case x\n".to_owned();
        for i in 0..2000 {
            source.push_str(&format!("when {i}; work\n"));
        }
        source.push_str("end\n");
        let found = offenses(&source);
        assert_eq!(found.len(), 1999);
        assert_eq!((found[0].line, found[0].col), (3, 1));
        assert_eq!((found[1998].line, found[1998].col), (2001, 1));
        let source = "begin; work; rescue A; handle; rescue B; handle; end\n".repeat(300);
        assert_eq!(offenses(&source).len(), 300);
    }

    #[test]
    fn ignores_distinct_absent_and_empty_branches() {
        for source in [
            "if foo; work; elsif bar; other; end",
            "unless foo; work; else; other; end",
            "if foo; work; end",
            "unless foo; work; end",
            "if foo; # comment\nend",
            "unless foo; end",
            "work if foo",
            "work unless foo",
            "if foo; else; end",
            "foo ? work : other",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
