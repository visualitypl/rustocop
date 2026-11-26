//! `Lint/EmptyWhen` — flags `when` branches without a body.
//!
//! Matches RuboCop's semantics with its default `AllowComments: true` (fixed): a `when` with no statements is accepted when a comment
//! starts on one of its lines, which run from the `when` keyword up to (not
//! including) the line where the next branch body starts: the next `when`,
//! the first `else` statement, or the `end` (RuboCop's
//! `CommentsHelp#find_end_line`). `case`/`in` is not checked. Disabling
//! directives suppress findings, but are not explanatory branch comments.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/EmptyWhen` cop.
pub struct EmptyWhen;

impl Cop for EmptyWhen {
    fn name(&self) -> &'static str {
        "Lint/EmptyWhen"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
        file.directives_with_disabled(result, &[]);
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_case_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(case) = node.as_case_node() else {
            return;
        };
        let branches: Vec<Node<'pr>> = case.conditions().iter().collect();
        for (index, branch) in branches.iter().enumerate() {
            let Some(when) = branch.as_when_node() else {
                continue;
            };
            if when.statements().is_some() {
                continue;
            }
            let start = when.location().start_offset();
            // Where the next branch body starts: RuboCop's right sibling (the
            // next `when` or the `else` body), else the `end`.
            let next_start = match branches.get(index + 1) {
                Some(next) => next.location().start_offset(),
                None => case
                    .else_clause()
                    .and_then(|clause| clause.statements())
                    .map_or(case.end_keyword_loc().start_offset(), |body| {
                        body.location().start_offset()
                    }),
            };
            let (line, col) = file.reference_line_col(start);
            let comment_lines = line..file.line_of(next_start);
            if file.has_comment_in_lines(comment_lines.clone())
                && !file.comments_contain_disables(self.name(), comment_lines)
            {
                continue;
            }
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: "Avoid `when` branches without a body.".to_string(),
                line,
                // The parser when range ends with its final condition, not
                // a `then` token that may occupy a later line.
                end_line: when
                    .conditions()
                    .iter()
                    .last()
                    .map_or(line, |condition| file.last_line_of(condition.location())),
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/empty_when_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The spec runs
    //! most offense examples with `AllowComments: false`; rustocop fixes the
    //! default `true`, so examples whose empty branch holds a comment are
    //! ported as no-offense cases.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyWhen")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn range_ends_at_condition_not_later_then_keyword() {
        let found = offenses("case x\nwhen foo(\n1\n)\nthen\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 4));
        assert_eq!(found[0].message, "Avoid `when` branches without a body.");
    }

    #[test]
    fn flags_missing_when_body_with_message() {
        let found = offenses("case foo\nwhen :bar then 1\nwhen :baz\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 1));
        assert_eq!(found[0].message, "Avoid `when` branches without a body.");
    }

    #[test]
    fn flags_missing_when_body_followed_by_else() {
        assert_eq!(
            positions("case foo\nwhen :bar then 1\nwhen :baz\nelse 3\nend\n"),
            vec![(3, 1)]
        );
        assert_eq!(
            positions("case foo\nwhen :bar then 1\nwhen :baz then\nelse 3\nend\n"),
            vec![(3, 1)]
        );
    }

    #[test]
    fn flags_missing_when_body_without_case_expression() {
        assert_eq!(
            positions("case\nwhen :bar\n  1\nwhen :baz\nelse\n  3\nend\n"),
            vec![(4, 1)]
        );
    }

    #[test]
    fn flags_branch_without_comment_next_to_one_with_comment() {
        let source = "\
case condition
when foo
  42 # magic number
when bar
when baz # more comments mixed
  21 # another magic number
end
";
        assert_eq!(positions(source), vec![(4, 1)]);
    }

    #[test]
    fn flags_every_empty_branch() {
        assert_eq!(
            positions("case x\nwhen 1; when 2; end\n"),
            vec![(2, 1), (2, 9)]
        );
        assert_eq!(positions("case\nwhen a\nend\n"), vec![(2, 1)]);
        assert_eq!(
            positions("def m\n  case x\n  when 1\n    a\n  when 2\n  end\nend\n"),
            vec![(5, 3)]
        );
    }

    #[test]
    fn hash_in_a_condition_is_not_a_comment() {
        assert_eq!(positions("case x\nwhen \"#\"\nend\n"), vec![(2, 1)]);
    }

    #[test]
    fn comment_after_the_next_branch_starts_does_not_count() {
        // The comment sits after the `else` body starts.
        assert_eq!(
            positions("case x\nwhen 1\nelse\n  3 # c\nend\n"),
            vec![(2, 1)]
        );
        assert_eq!(
            positions("case x\nwhen 1\n  a\n  # about 2\nwhen 2\nend\n"),
            vec![(5, 1)]
        );
    }

    #[test]
    fn accepts_empty_branches_with_comments() {
        for source in [
            "case foo\nwhen :bar then 1\nwhen :baz # nothing\nend\n",
            "case foo\nwhen :bar then 1\nwhen :baz # nothing\nelse 3\nend\n",
            "case foo\nwhen :bar then 1\nwhen :baz then # nothing\nend\n",
            "case foo\nwhen :bar then 1\nwhen :baz then # nothing\nelse 3\nend\n",
            "case foo\nwhen :bar\n  1\nwhen :baz\n  # nothing\nend\n",
            "case foo\nwhen :bar\n  1\nwhen :baz\n  # nothing\nelse\n  3\nend\n",
            "case\nwhen :bar\n  1\nwhen :baz\n  # nothing\nelse\n  3\nend\n",
            "case condition\nwhen foo\n  do_something\nwhen bar\n  # do nothing\nend\n",
            "case condition\nwhen foo\n  do_something\nwhen bar then # do nothing\nend\n",
            // RuboCop counts comments up to the first `else` statement.
            "case x\nwhen 1\nelse\n  # c\n  3\nend\n",
            "case x\nwhen 1\n# c\nelse\nend\n",
            "case x\nwhen 1, # c\n  2\nwhen 3\n  a\nend\n",
            "case x\nwhen 1\n=begin\nc\n=end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_when_bodies() {
        for source in [
            "case foo\nwhen :bar then 1\nwhen :baz then 2\nend\n",
            "case foo\nwhen :bar then 1\nwhen :baz then 2\nelse 3\nend\n",
            "case foo\nwhen :bar\n  1\nwhen :baz\n  2\nend\n",
            "case foo\nwhen :bar\n  1\nwhen :baz\n  2\nelse\n  3\nend\n",
            "case\nwhen :bar\n  1\nwhen :baz\n  2\nelse\n  3\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "case foo\nwhen :bar\n  nil\nend\n",
            "case foo\nin :bar\nend\n",
            "case foo\nwhen :bar then ()\nend\n",
            "case foo\nwhen 1\n  a\nelse\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
