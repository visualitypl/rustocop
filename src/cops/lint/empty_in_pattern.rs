//! `Lint/EmptyInPattern` — empty pattern-matching branches.
//!
//! Fixed AllowComments=true. A branch's comment window ends at the next `in`,
//! the first else statement (or empty-else keyword), or the case's `end`.
//! Disabling directives are suppression, not explanatory comments.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct EmptyInPattern;

impl Cop for EmptyInPattern {
    fn name(&self) -> &'static str {
        "Lint/EmptyInPattern"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
        // Pipeline callers have already initialized this from the main parse;
        // direct dispatcher callers use that same parse, never a candidate parse.
        file.directives_with_disabled(result, &[]);
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_case_match_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(case) = node.as_case_match_node() else {
            return;
        };
        let mut branches = case.conditions().iter().peekable();
        while let Some(branch) = branches.next() {
            let Some(branch) = branch.as_in_node() else {
                continue;
            };
            if branch.statements().is_some() {
                continue;
            }
            let (line, col) = file.line_col(branch.in_loc().start_offset());
            // CommentsHelp#find_end_line: the right sibling (including the
            // first else *statement*, not its keyword), or the case end.
            let next_start = branches.peek().map_or_else(
                || {
                    case.else_clause()
                        .map_or(case.end_keyword_loc().start_offset(), |e| {
                            e.statements()
                                .map_or(e.else_keyword_loc().start_offset(), |body| {
                                    body.location().start_offset()
                                })
                        })
                },
                |next| next.location().start_offset(),
            );
            let comment_lines = line..file.line_of(next_start);
            if file.has_comment_in_lines(comment_lines.clone())
                && !file.comments_contain_disables(self.name(), comment_lines)
            {
                continue;
            }
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                // RuboCop's empty in_pattern range stops after the pattern /
                // guard, excluding even a `then` on a later physical line.
                end_line: file.last_line_of(branch.pattern().location()),
                message: "Avoid `in` branches without a body.".into(),
                correction: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/empty_in_pattern_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyInPattern")
            .collect()
    }

    #[test]
    fn allows_comments_only_in_the_branch_comment_window() {
        for source in [
            "case foo\nin [a] then 1\nin [a, b] # nothing\nend\n",
            "case foo\nin [a] then 1\nin [a, b] then # nothing\nelse 3\nend\n",
            "case foo\nin [a, b]\n  # nothing\nend\n",
            "case foo\nin [a, b]\nelse\n  # before else body\n  3\nend\n",
            "case foo\nin [\n  a, # pattern comment\n  b\n]\nend\n",
            "case foo\nin [a]\n=begin\nnoop\n=end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "case foo\nin [a]\nin [a, b]\n  # next branch\nend\n",
            "case foo\nin [a]\nelse\n  3 # after else body\nend\n",
            "case foo\nin [a, b]\nelse\n  # empty else\nend\n",
            "# before\ncase foo\nin [a]\nend # after\n",
            "case foo\nin \"# not a comment\"\nend\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn accepts_real_bodies_and_nonbranch_pattern_lookalikes() {
        for source in [
            "case foo\nin [a] then nil\nin [a, b] then false\nend\n",
            "case foo\nin Integer | String if ready?\n  work\nelse\nend\n",
            "case foo\nin [a] unless ready?\n  ()\nend\n",
            "case foo\nin [a]\n  begin; end\nend\n",
            "foo in [a]\n",
            "foo => [a]\n",
            "case foo\nwhen 1\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn guards_alternatives_and_then_have_exact_multiline_highlights() {
        for (source, line, col, end_line) in [
            ("case x\n  in Integer | String if ready?\nend\n", 2, 3, 2),
            (
                "case x\n  in [a] unless (\n    ready?\n  )\n  then\nend\n",
                2,
                3,
                4,
            ),
            ("case x\n  in [\n    a,\n    b\n  ]\n  then\nend\n", 2, 3, 5),
            ("case x\nin [a]\n  case a\n  in [b]\n  end\nend\n", 4, 3, 4),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line),
                "{source}"
            );
        }
    }

    #[test]
    fn completed_disables_in_comment_tail_are_not_explanations() {
        for prefix in ["rubocop", "rustocop"] {
            for target in ["Lint/EmptyInPattern", "Lint", "all"] {
                for tail in [
                    format!("# {prefix}:disable {target}\n# {prefix}:enable {target}\n"),
                    format!("# {prefix}:disable {target}\n# {prefix}:disable {target}\n"),
                ] {
                    let source = format!("case x\nin [a]\n{tail}end\n# {prefix}:enable {target}\n");
                    assert_eq!(offenses(&source).len(), 1, "{source}");
                }
                // A merely overlapping tail interval is not contained in the
                // comment window; RuboCop still accepts its comment here.
                let source = format!(
                    "case x\nin [a]\n# {prefix}:disable {target}\nend\n# {prefix}:enable {target}\n"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }

    #[test]
    fn lexical_block_directives_are_not_explanatory_comments() {
        for prefix in ["rubocop", "rustocop"] {
            for (open, close) in [
                ("push -Lint/EmptyInPattern", "pop"),
                ("disable Lint/EmptyInPattern", "enable Lint/EmptyInPattern"),
                ("todo Lint/EmptyInPattern", "enable Lint/EmptyInPattern"),
            ] {
                let source = format!(
                    "case x\nin [a]\n=begin\n# {prefix}:{open}\n=end\n# {prefix}:{close}\nend\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_eq!(found[0].message, "Avoid `in` branches without a body.");
                assert!(found[0].correction.is_none());
            }
            // A fourth exploratory case already agreed: its next
            // interval starts at `end`, outside the branch's comment window.
            let source = format!(
                "case x\nin [a]\n=begin\n# {prefix}:next -Lint/EmptyInPattern\n=end\n# {prefix}:pop\nend\n"
            );
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn minimum_ruby_gate_applies_to_defaults_and_forced_selection() {
        use crate::cops::{Registry, policy::RubyVersion, select_active_for_version};
        let registry = Registry::new();
        let config = Default::default();
        for (version, count) in [
            (RubyVersion(2, 6), 0),
            (RubyVersion(2, 7), 1),
            (RubyVersion(3, 4), 1),
        ] {
            for only in [
                vec![],
                vec!["Lint/EmptyInPattern".into()],
                vec!["Lint".into()],
            ] {
                let active = select_active_for_version(&registry, &config, &only, &[], version);
                assert_eq!(
                    active
                        .iter()
                        .filter(|c| c.name() == "Lint/EmptyInPattern")
                        .count(),
                    count
                );
            }
        }
    }

    #[test]
    fn flags_missing_body_with_exact_location_and_message() {
        let found = offenses("case foo\nin [a] then 1\n  in [a, b]\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 3));
        assert_eq!(found[0].message, "Avoid `in` branches without a body.");
        assert!(found[0].correction.is_none());
    }
}
