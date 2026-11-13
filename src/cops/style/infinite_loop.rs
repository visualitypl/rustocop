//! `Style/InfiniteLoop` — prefer Kernel#loop for literal infinite loops.
//! Detection only: block scope, dispatch and StopIteration make rewriting unsafe.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::{Assignment, AssignmentKind, ScopeAnalysis},
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::{cmp::Reverse, ops::Range};

pub struct InfiniteLoop;

impl Cop for InfiniteLoop {
    fn name(&self) -> &'static str {
        "Style/InfiniteLoop"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(program) = node.as_program_node() else {
            return;
        };
        // Whole-file variable identity is needed, just as for UselessAssignment.
        // Collect once, not once per loop; do not reparse or rescan source text.
        let mut collector = Loops(Vec::new());
        walk::walk(&mut collector, node);
        let mut loops = collector.0;
        if loops.is_empty() {
            return;
        }
        loops.sort_by_key(|l| (l.range.start, Reverse(l.range.end)));
        let mut stack: Vec<usize> = Vec::new();
        for i in 0..loops.len() {
            while stack.last().is_some_and(|&p| {
                loops[p].range.end < loops[i].range.end
                    || loops[p].range.end <= loops[i].range.start
            }) {
                stack.pop();
            }
            loops[i].parent = stack.last().copied();
            stack.push(i);
        }
        let analysis = ScopeAnalysis::of(&program);
        for scope in analysis.scopes_in_exit_order() {
            for id in analysis.scope(scope).variables() {
                let variable = analysis.variable(id);
                let Some(last_read) = variable
                    .references
                    .iter()
                    .map(|&r| analysis.reference(r).range.start)
                    .max()
                else {
                    continue;
                };
                let first_end = variable
                    .assignments
                    .iter()
                    .map(|&a| assignment_range(analysis.assignment(a)).end)
                    .min()
                    .unwrap_or(usize::MAX);
                for &id in &variable.assignments {
                    let range = assignment_range(analysis.assignment(id));
                    let start = range.start;
                    let end = range.end;
                    let mut cursor = loops
                        .partition_point(|l| l.range.start <= start)
                        .checked_sub(1);
                    // The nearest preceding candidate and its parents include
                    // every containing loop. Depth is bounded by the pipeline's
                    // 64-node AST limit, independently of sibling/variable count.
                    while let Some(i) = cursor {
                        #[cfg(test)]
                        QUERY_EDGES.with(|n| n.set(n.get() + 1));
                        let candidate = &mut loops[i];
                        if end <= candidate.range.end
                            && first_end >= candidate.range.start
                            && last_read > candidate.range.end
                        {
                            candidate.suppressed = true;
                        }
                        cursor = candidate.parent;
                    }
                }
            }
        }
        for candidate in loops.into_iter().filter(|l| !l.suppressed) {
            let (line, col) = file.line_col(candidate.keyword);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Use `Kernel#loop` for infinite loops.".into(),
                line,
                col,
                end_line: line,
                correction: None,
            });
        }
    }
}

// RuboCop represents a compound write's assignment as a synthetic local
// target, excluding its RHS. Other assignments retain their full node range.
fn assignment_range(assignment: &Assignment<'_>) -> Range<usize> {
    if assignment.kind == AssignmentKind::Operator {
        assignment.name_range.clone()
    } else {
        let location = assignment.node.location();
        location.start_offset()..location.end_offset()
    }
}

#[cfg(test)]
thread_local! { static QUERY_EDGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

struct Loop {
    range: Range<usize>,
    keyword: usize,
    parent: Option<usize>,
    suppressed: bool,
}
struct Loops(Vec<Loop>);
impl<'pr> NodeVisitor<'pr> for Loops {
    fn enter(&mut self, node: Node<'pr>) {
        let keyword = if let Some(n) = node.as_while_node() {
            let predicate = n.predicate();
            (ast::is_literal(&predicate)
                && !matches!(predicate, Node::FalseNode { .. } | Node::NilNode { .. }))
            .then(|| n.keyword_loc())
        } else if let Some(n) = node.as_until_node() {
            matches!(n.predicate(), Node::FalseNode { .. } | Node::NilNode { .. })
                .then(|| n.keyword_loc())
        } else {
            None
        };
        if let Some(keyword) = keyword {
            let location = node.location();
            self.0.push(Loop {
                range: location.start_offset()..location.end_offset(),
                keyword: keyword.start_offset(),
                parent: None,
                suppressed: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/infinite_loop_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            crate::parser::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/InfiniteLoop")
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
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/infinite_loop_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 26);
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
            let requested =
                crate::cops::testing::correction_offenses(&super::InfiniteLoop, &case.source);
            assert_eq!(requested.len(), found.len());
            assert!(requested.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn nested_same_start_modifier_and_typed_child_loops_are_checked_once() {
        for (source, positions) in [
            ("work while true until false", vec![(1, 6), (1, 17)]),
            ("consume(while true; break; end)", vec![(1, 9)]),
            (
                "begin; work; rescue; until false; break; end; ensure; while true; break; end; end",
                vec![(1, 22), (1, 55)],
            ),
            (
                "while true; until false; a = 1; break; end; break; end; p a",
                vec![],
            ),
            (
                "while true; until false; a = 1; break; end; p a; break; end",
                vec![(1, 1)],
            ),
            (
                "items.each { while true; work(it); break; end }",
                vec![(1, 14)],
            ),
            (
                "items.each { while true; work(_1); break; end }",
                vec![(1, 14)],
            ),
            (
                "until false; a = 1; break; end; while true; b = 1; break; end; p a",
                vec![(1, 33)],
            ),
        ] {
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
    fn indexed_assignment_queries_are_bounded_by_depth_not_sibling_count() {
        let n = 4_000;
        let source: String = (0..n)
            .map(|i| format!("while true; a{i} = 1; break; end; p a{i}\n"))
            .collect();
        super::QUERY_EDGES.with(|c| c.set(0));
        let found = crate::cops::testing::correction_offenses(&super::InfiniteLoop, &source);
        assert!(found.is_empty());
        super::QUERY_EDGES.with(|c| assert_eq!(c.get(), n));
        let source = format!(
            "{}a = 1; break; {}p a",
            "while true; ".repeat(20),
            "end; ".repeat(20)
        );
        super::QUERY_EDGES.with(|c| c.set(0));
        assert!(
            crate::cops::testing::correction_offenses(&super::InfiniteLoop, &source).is_empty()
        );
        super::QUERY_EDGES.with(|c| assert_eq!(c.get(), 20));
    }

    #[test]
    fn literal_conditions_report_only_the_keyword_with_exact_message() {
        for (keyword, literals) in [
            (
                "while",
                vec![
                    "true",
                    "1",
                    "0",
                    "-1",
                    "2.0",
                    "1r",
                    "2i",
                    "[work]",
                    "{}",
                    "{a: work}",
                    "\"\"",
                    "\"#{work}\"",
                    ":x",
                    ":\"#{work}\"",
                    "`work`",
                    "__FILE__",
                    "__LINE__",
                ],
            ),
            ("until", vec!["false", "nil"]),
        ] {
            for literal in literals {
                let source = format!("def example\n  {keyword} {literal}\n    break\n  end\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (2, 3, 2),
                    "{source}"
                );
                assert_eq!(found[0].message, "Use `Kernel#loop` for infinite loops.");
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn pre_post_modifier_and_multiline_modifier_positions() {
        for (source, position) in [
            ("while true do\nend", (1, 1)),
            ("until false\nend", (1, 1)),
            ("work while true # comment", (1, 6)),
            ("something += 1 until false", (1, 16)),
            ("begin # comment\n work\nend while true", (3, 5)),
            ("begin\n work\n other\nend until nil", (4, 5)),
            ("something 1,\n  2 while true", (2, 5)),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), position, "{source}");
            assert_eq!(found[0].end_line, position.0);
        }
    }

    #[test]
    fn ignores_nonliteral_opposite_and_grouped_conditions_and_lookalikes() {
        for source in [
            "while false; end",
            "while nil; end",
            "until true; end",
            "until 1; end",
            "while ready; end",
            "until done; end",
            "while (true); break; end",
            "while true && ready; end",
            "while !false; end",
            "while x = true; break; end",
            "while /x/; break; end",
            "while /#{work}/; break; end",
            "while 1..2; break; end",
            "while __ENCODING__; break; end",
            "loop { break if something }",
            "Kernel.loop { break }",
            "obj.while(true)",
            "# while true\n\"until false\"",
            "__END__\nwhile true\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn new_locals_referenced_after_loop_suppress_detection() {
        for source in [
            "while true\n a = 42\n break\nend\np a",
            "a = next_value or break while true\np a",
            "begin\n a = work\n break\nend until false\np a",
            "while true\n a, b = 42, 42\n break\nend\nputs a, b",
            "while true; a ||= 1; break; end; p a",
            "while true; for a in []; end; break; end; p a",
            "while true; begin; work; rescue => e; end; break; end; p e",
            "while true; a = 1; break; end; proc { a }",
            // Pinned VariableForce counts assignments, not parameter declarations.
            "def f(a); while true; a = 1; break; end; p a; end",
            "items.each { |a| while true; a = 1; break; end; p a }",
            "def f1; a = nil; p a; end\ndef f2; b = 17; while true; a,b = 42,42; break; end; puts a,b; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn compound_assignment_targets_precede_loops_in_their_rhs() {
        for operator in ["||=", "&&=", "+="] {
            let source = format!("a {operator} while true; a = 1; break; end; p a");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, operator.len() as u32 + 4, 1),
                "{source}"
            );
            assert_eq!(found[0].message, "Use `Kernel#loop` for infinite loops.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn compound_assignment_targets_inside_loops_still_suppress_detection() {
        for operator in ["||=", "&&=", "+="] {
            let source = format!("while true; a {operator} 1; break; end; p a");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn earlier_assignment_unused_after_and_nonlocals_allow_detection() {
        for source in [
            "a = nil\na = next_value or break until false\np a",
            "until false; a = 43; break; end",
            "while true; @a = 42; break; end; puts @a",
            "while true; $a = 42; break; end; puts $a",
            "while true; @@a = 42; break; end; puts @@a",
            "while true; a = 1; p a; break; end",
            "a = 0\nwhile true; a = 42; break; end\np a",
            "while true; items.each { a = 1 }; break; end; a",
            "while true; def f; a = 1; end; break; end; a",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        let source = "while true\n a = 42\n break\nend\nuntil false\n a = 43\n break\nend\np a";
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(5, 1)]
        );
    }
}
