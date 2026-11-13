//! `Style/CombinableLoops` — consecutive nonempty loops over the same data.
//!
//! Detection only: fusion can change evaluation order, control flow and values.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct CombinableLoops;

impl Cop for CombinableLoops {
    fn name(&self) -> &'static str {
        "Style/CombinableLoops"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_statements_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(statements) = node.as_statements_node() else {
            return;
        };
        // A plain explicit begin/end (also in post-test loops) is kwbegin,
        // not RuboCop's implicit begin sequence. Rescue/ensure wrap a sequence.
        if cx
            .parent()
            .and_then(|n| n.as_begin_node())
            .is_some_and(|b| {
                b.begin_keyword_loc().is_some()
                    && b.rescue_clause().is_none()
                    && b.ensure_clause().is_none()
            })
        {
            return;
        }
        // Scan each statement list once, not the siblings anew for every loop.
        // Other multi-statement Prism lists correspond to a begin parent.
        let mut previous = None;
        for current in statements.body().iter() {
            if previous
                .as_ref()
                .is_some_and(|left| same_loops(left, &current, file))
            {
                let loc = current.location();
                let (line, col) = file.line_col(loc.start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: "Combine this loop with the previous loop.".into(),
                    line,
                    col,
                    end_line: file.line_of(loc.end_offset().saturating_sub(1)),
                    correction: None,
                });
            }
            previous = Some(current);
        }
    }
}

#[cfg(test)]
thread_local! {
    static PAIR_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn same_loops(left: &Node<'_>, right: &Node<'_>, file: &SourceFile) -> bool {
    #[cfg(test)]
    PAIR_CHECKS.with(|count| count.set(count.get() + 1));
    if let (Some(left), Some(right)) = (left.as_for_node(), right.as_for_node()) {
        return left.statements().is_some()
            && right.statements().is_some()
            && same_node(&left.collection(), &right.collection(), file);
    }
    let (Some(left), Some(right)) = (left.as_call_node(), right.as_call_node()) else {
        return false;
    };
    let name = right.name().as_slice();
    (name.starts_with(b"each") || name.ends_with(b"_each"))
        && name == left.name().as_slice()
        && left
            .block()
            .and_then(|b| b.as_block_node())
            .is_some_and(|b| b.body().is_some())
        && right
            .block()
            .and_then(|b| b.as_block_node())
            .is_some_and(|b| b.body().is_some())
        && same_optional(left.receiver(), right.receiver(), file)
        && same_optional(
            left.arguments().map(|n| n.as_node()),
            right.arguments().map(|n| n.as_node()),
            file,
        )
}

fn same_optional(left: Option<Node<'_>>, right: Option<Node<'_>>, file: &SourceFile) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_node(&left, &right, file),
        _ => false,
    }
}

fn same_node(left: &Node<'_>, right: &Node<'_>, file: &SourceFile) -> bool {
    // Detection-only parser equality, including selector spellings, regexp
    // option sets, numeric values and cached __LINE__ values.
    ast::fingerprint_for_detection(left, file) == ast::fingerprint_for_detection(right, file)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/combinable_loops_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/CombinableLoops")
            .collect()
    }

    #[test]
    fn recognizes_method_prefix_suffix_and_structural_arguments() {
        for method in [
            "each",
            "each_with_index",
            "each_slice",
            "each_custom",
            "reverse_each",
            "custom_each",
        ] {
            let source =
                format!("items.{method}(0x2) {{ foo(_1) }}\nitems.{method}(2) do bar(it) end\n");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
        for source in [
            "each_slice(2) { foo }; each_slice 2 do bar end",
            "items&.each { foo }; items.each { bar }",
            "[1, 'a'].each { foo }; [0x1, \"a\"].each { bar }",
            "items.each { foo }; # comment\nitems.each { bar }; items.each { baz }",
        ] {
            let expected = if source.contains("baz") { 2 } else { 1 };
            assert_eq!(offenses(source).len(), expected, "{source}");
        }
    }

    #[test]
    fn equivalent_selector_spellings_and_regexp_option_sets() {
        for source in [
            "items[0].each { foo }\nitems.[](0).each { bar }",
            "items.each_slice(/a/im) { foo }\nitems.each_slice(/a/mi) { bar }",
            "items.each_slice(/#{pattern}/im) { foo }\nitems.each_slice(/#{pattern}/mi) { bar }",
            "for x in items[0]; foo; end\nfor y in items.[](0); bar; end",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
            assert_eq!(
                found[0].message,
                "Combine this loop with the previous loop."
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "items[0].each { foo }\nothers.[](0).each { bar }",
            "items[0].each { foo }\nitems.[](1).each { bar }",
            "items[0].each { foo }\nitems&.[](0).each { bar }",
            "items.each_slice(/a/im) { foo }\nitems.each_slice(/a/i) { bar }",
            "items.each_slice(/a/im) { foo }\nitems.each_slice(/b/mi) { bar }",
            "items.each_slice(/#{pattern}/im) { foo }\nitems.each_slice(/#{pattern}/i) { bar }",
            "items.each_slice(/a/n) { foo }\nitems.each_slice(/a/u) { bar }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn excludes_nonadjacent_empty_mixed_and_value_context_lookalikes() {
        for source in [
            "items.each { foo }; other; items.each { bar }",
            "items.each { foo }; others.each { bar }",
            "items.each(1) { foo }; items.each(2) { bar }",
            "items.each { foo }; items.reverse_each { bar }",
            "items.map { foo }; items.map { bar }",
            "items.teach { foo }; items.teach { bar }",
            "items.each {}; items.each { foo }",
            "items.each { foo }; items.each { # comment\n}",
            "for x in items; end; for y in items; foo; end",
            "for x in items; foo; end; for y in items; end",
            "for x in items; foo; end; items.each { bar }",
            "items.each { foo }; items.each(&proc {})",
            "x = items.each { foo }; y = items.each { bar }",
            "consume(items.each { foo }, items.each { bar })",
            "if ready; items.each { foo }; else; items.each { bar }; end",
            "items = []; items.each { foo }; self.items.each { bar }",
            "items.each { foo }; items = []; items.each { bar }",
            "while cond; foo; end; while cond; bar; end",
            "begin; foo; end while cond; begin; bar; end while cond",
            "until cond; foo; end; until cond; bar; end",
            "# items.each { foo }; items.each { bar }\n'items.each { foo }; items.each { bar }'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn nested_statement_lists_and_exception_bodies_are_checked() {
        for wrapper in [
            ("def f\n", "\nend"),
            ("if ready\n", "\nend"),
            ("begin\n", "\nrescue\nrecover\nend"),
            ("outer {\n", "\n}"),
            ("while ready\n", "\nend"),
            ("begin\n", "\nensure\ncleanup\nend"),
        ] {
            let source = format!(
                "{}items.each {{ foo }}\nitems.each {{ bar }}{}",
                wrapper.0, wrapper.1
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1));
        }
        for source in [
            "items.each do; foo; rescue; recover; end\nitems.each do; bar; ensure; cleanup; end",
            "items.each { break 1 }; items.each { next 2 }",
            "items.each { redo if ready }; items.each { return 1 }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn explicit_begin_sequences_are_kwbegin_not_implicit_begin_parents() {
        for suffix in ["end", "end while ready", "end until ready"] {
            for loops in [
                "items.each { foo }; items.each { bar }",
                "for x in items; foo; end; for y in items; bar; end",
            ] {
                let source = format!("begin\n{loops}\n{suffix}");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }

    #[test]
    fn sibling_comparisons_are_linear_and_do_not_search_past_barriers() {
        use super::{CombinableLoops, PAIR_CHECKS};
        use crate::cops::testing::correction_offenses;
        for n in [128, 1024] {
            let source = "items.each { foo }; barrier; items.each { bar }\n".repeat(n);
            PAIR_CHECKS.with(|count| count.set(0));
            let found = correction_offenses(&CombinableLoops, &source);
            assert_eq!(found.len(), n - 1);
            // Three top-level statements per repetition; all block bodies have
            // one statement, so they require no pair comparison.
            assert_eq!(PAIR_CHECKS.with(|count| count.get()), 3 * n - 1);
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[derive(serde::Deserialize)]
    struct Case {
        spec_line: u32,
        source: String,
        positions: Vec<(u32, u32)>,
        messages: Vec<String>,
    }

    #[test]
    fn every_pinned_example_has_exact_diagnostics_and_no_edits() {
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/combinable_loops_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 46);
        for c in cases {
            let found = offenses(&c.source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                c.positions,
                "spec line {}: {}",
                c.spec_line,
                c.source
            );
            assert_eq!(
                found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
                c.messages,
                "spec line {}",
                c.spec_line
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn source_line_arguments_compare_by_evaluated_integer_value() {
        for source in [
            "each_slice(__LINE__) { foo }; each_slice(__LINE__) { bar }",
            "each_slice(__LINE__) { foo }\neach_slice(1) { bar }",
            "for a in [__LINE__]; foo; end; for b in [__LINE__]; bar; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("each_slice(__LINE__) { foo }\neach_slice(__LINE__) { bar }").is_empty());
    }

    #[test]
    fn consecutive_for_loops_ignore_iteration_variable_names() {
        let found =
            offenses("for item in items do foo(item) end\nfor other in items do bar(other) end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 1));
        assert_eq!(
            found[0].message,
            "Combine this loop with the previous loop."
        );
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn consecutive_each_blocks_report_the_second_loop() {
        let found = offenses(
            "def f\n  items.each { |item| foo(item) }\n  items.each { |x| bar(x) }\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 3));
        assert_eq!(
            found[0].message,
            "Combine this loop with the previous loop."
        );
        assert!(found[0].correction.is_none());
    }
}
