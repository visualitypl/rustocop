//! `Style/For` — prefer `each` over `for` (fixed policy, detection only).
//!
//! Every `for` is reported, including single-line and empty loops. Changing
//! one to `each` is unsafe: loop variables and body locals have different scopes.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct For;

impl Cop for For {
    fn name(&self) -> &'static str {
        "Style/For"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_for_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(loop_node) = node.as_for_node() else {
            return;
        };
        let (line, col) = file.line_col(loop_node.for_keyword_loc().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Prefer `each` over `for`.".into(),
            line,
            col,
            end_line: file.last_line_of(loop_node.location()),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/for_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/For")
            .collect()
    }

    #[test]
    fn every_applicable_pinned_call_has_exact_diagnostics_and_no_corrections() {
        #[derive(serde::Deserialize)]
        struct Case {
            spec_line: u32,
            source: String,
            positions: Vec<(u32, u32)>,
            messages: Vec<String>,
        }
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../../tests/fixtures/for_pinned.json")).unwrap();
        assert_eq!(cases.len(), 29);
        for case in cases {
            let found = offenses(&case.source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                case.positions,
                "spec {}: {}",
                case.spec_line,
                case.source
            );
            assert_eq!(
                found.iter().map(|o| o.message.as_str()).collect::<Vec<_>>(),
                case.messages,
                "spec {}",
                case.spec_line
            );
            let requested = crate::cops::testing::correction_offenses(&super::For, &case.source);
            assert_eq!(requested.len(), found.len());
            assert!(requested.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn flags_for_at_its_start_with_exact_message_and_whole_loop_span() {
        let found = offenses("def func\n  for n in [1, 2, 3] do\n    puts n\n  end\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
        assert_eq!(found[0].message, "Prefer `each` over `for`.");
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn single_line_empty_and_nonlocal_targets_are_still_offenses() {
        for target in ["x", "@x", "@@x", "$x", "obj.x", "obj[0]", "a, (b, *c)"] {
            for suffix in ["; end", " do work end", "\nend", " do\n work\nend"] {
                let source = format!("  for {target} in items{suffix}\n");
                assert_eq!(
                    crate::parser::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 3));
                assert_eq!(found[0].message, "Prefer `each` over `for`.");
            }
        }
    }

    #[test]
    fn visits_nested_loops_in_collection_body_and_typed_children_once() {
        for (source, positions) in [
            (
                "for x in (for y in items; work; end); for z in items; work; end; end",
                vec![(1, 1), (1, 11), (1, 39)],
            ),
            ("consume(for x in items; work; end)", vec![(1, 9)]),
            (
                "begin\n work\nrescue\n for x in items\n end\nensure\n for y in items\n end\nend",
                vec![(4, 2), (7, 2)],
            ),
            ("items.each { for x in _1; p x; end }", vec![(1, 14)]),
            ("items.each { for x in it; p x; end }", vec![(1, 14)]),
            (
                "while ready\n for x in items\n end\nend\nbegin\n for y in items\n end\nend until done",
                vec![(2, 2), (6, 2)],
            ),
        ] {
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert_eq!(
                offenses(source)
                    .iter()
                    .map(|o| (o.line, o.col))
                    .collect::<Vec<_>>(),
                positions,
                "{source}"
            );
        }
    }

    #[test]
    fn ignores_each_variants_other_loops_and_keyword_lookalikes() {
        for source in [
            "items.each do |n|\n puts n\nend",
            "each { |n| work(n) }; items&.each { work(_1) }",
            "items.each { work(it) }; items.each { |*args| work(args) }",
            "while ready; work; end; begin; work; end until done",
            "loop { work }; work until ready; work while ready",
            "[:for, :ala, :bala]",
            "def for; end; obj.for; self.for(1)",
            "# for x in items; end\n\"for x in items; end\"",
            "pattern = /for x in items/; x = :for",
            "__END__\nfor x in items\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn many_siblings_and_nested_loops_need_no_subtree_or_source_scan() {
        // The matcher only reads the current ForNode's two locations. Main
        // traversal owns recursion; adding siblings/body size cannot rescan it.
        let source = "for x in items; work; end\n".repeat(6_000);
        let found = crate::cops::testing::correction_offenses(&super::For, &source);
        assert_eq!(found.len(), 6_000);
        assert_eq!((found[5_999].line, found[5_999].col), (6_000, 1));
        assert!(found.iter().all(|o| o.correction.is_none()));
        let source = format!(
            "{}work\n{}",
            "for x in items\n".repeat(20),
            "end\n".repeat(20)
        );
        let found = crate::cops::testing::correction_offenses(&super::For, &source);
        assert_eq!(found.len(), 20);
        assert_eq!(found[0].end_line, 41);
        assert_eq!(found[19].line, 20);
    }
}
