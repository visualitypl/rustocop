//! `Lint/EmptyConditionalBody` — flags `if`, `elsif` and `unless` branches
//! without a body.
//!
//! Matches RuboCop's semantics with its default `AllowComments: true` (fixed):
//!
//! - A branch with no statements is reported at its keyword, unless a
//!   comment starts on one of its lines: from the keyword up to (not
//!   including) the line of the next `elsif`/`else`, or the `end`
//!   (RuboCop's `CommentsHelp#find_end_line`).
//! - An `if`/`unless` whose `then` (or `;`) and `end` share a line is left
//!   alone (`if x; end`, `if x; else y end`): RuboCop defers those to
//!   `Style/IfWithSemicolon`. An `elsif` has no `end` of its own, so this
//!   never applies to it.
//! - Empty `else` branches are `Style/EmptyElse`'s business.
//!
//! Direct parent parentheses extend an `if`'s comment window. Disabling
//! directives suppress findings, but are not explanatory branch comments.

use ruby_prism::{Location, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/EmptyConditionalBody` cop.
pub struct EmptyConditionalBody;

impl Cop for EmptyConditionalBody {
    fn name(&self) -> &'static str {
        "Lint/EmptyConditionalBody"
    }

    fn check_parse(
        &self,
        result: &ruby_prism::ParseResult<'_>,
        file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
        file.directives_with_disabled(result, &[]);
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(branch) = Branch::of(node) else {
            return;
        };
        if branch.has_body {
            return;
        }
        // `if x; end`, `if x then end`: `then`/`;` and `end` on one line.
        if let (Some(begin), Some(end)) = (branch.begin_offset(file), branch.end)
            && file.line_col(begin).0 == file.line_col(end).0
        {
            return;
        }
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(branch.keyword.start_offset(), file);
        let comment_end = if branch.next_branch.is_none() && branch.keyword.as_slice() == b"if" {
            // StatementsNode is transparent in the parser AST. Only the direct
            // parentheses parent extends CommentsHelp's window, not any ancestor.
            cx.ancestors()
                .find(|p| p.as_statements_node().is_none() && p.as_arguments_node().is_none())
                .and_then(|parent| {
                    parent
                        .as_parentheses_node()
                        .map(|p| p.closing_loc())
                        .or_else(|| {
                            parent
                                .as_call_node()
                                .and_then(|c| c.closing_loc())
                                .filter(|l| l.as_slice() == b")")
                        })
                        .or_else(|| parent.as_super_node().and_then(|n| n.rparen_loc()))
                        .or_else(|| parent.as_yield_node().and_then(|n| n.rparen_loc()))
                        .or_else(|| parent.as_defined_node().and_then(|n| n.rparen_loc()))
                })
                .map_or(branch.chain_end, |closing| closing.start_offset())
        } else {
            branch.comment_range_end()
        };
        let comment_lines = line..file.line_of(comment_end);
        if file.has_comment_in_lines(comment_lines.clone())
            && !file.comments_contain_disables(self.name(), comment_lines)
        {
            return;
        }
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Avoid `{}` branches without a body.",
                String::from_utf8_lossy(branch.keyword.as_slice())
            ),
            line,
            // offense_range joins the empty branch with the *beginning* of
            // the next keyword. Its endpoint still lies on that keyword's line.
            end_line: file.line_of(branch.next_branch.unwrap_or_else(|| {
                if branch.end.is_none() {
                    branch
                        .then_keyword
                        .as_ref()
                        .map_or(branch.predicate_end, |l| l.end_offset())
                        .saturating_sub(1)
                } else {
                    node.location().end_offset().saturating_sub(1)
                }
            })),
            col,
        });
    }
}

/// An `if`, `elsif` or `unless` branch, in the terms RuboCop's checks use.
struct Branch<'pr> {
    /// The `if`/`elsif`/`unless` keyword.
    keyword: Location<'pr>,
    /// Where the condition ends.
    predicate_end: usize,
    /// The `then` keyword.
    then_keyword: Option<Location<'pr>>,
    /// Whether the branch has statements.
    has_body: bool,
    /// Where the next `elsif` or `else` keyword starts.
    next_branch: Option<usize>,
    /// Where the branch's own `end` starts; `None` for `elsif`, whose `end`
    /// belongs to the head `if`.
    end: Option<usize>,
    /// Where the `end` closing the whole chain starts.
    chain_end: usize,
}

impl<'pr> Branch<'pr> {
    fn of(node: &Node<'pr>) -> Option<Self> {
        if let Some(branch) = node.as_if_node() {
            // Ternaries have no keyword.
            let keyword = branch.if_keyword_loc()?;
            let chain_end = branch.end_keyword_loc()?.start_offset();
            let is_elsif = keyword.as_slice() == b"elsif";
            let next_branch = branch.subsequent().map(|next| match next.as_else_node() {
                Some(clause) => clause.else_keyword_loc().start_offset(),
                None => next.location().start_offset(),
            });
            return Some(Branch {
                keyword,
                predicate_end: branch.predicate().location().end_offset(),
                then_keyword: branch.then_keyword_loc(),
                has_body: branch.statements().is_some(),
                next_branch,
                end: (!is_elsif).then_some(chain_end),
                chain_end,
            });
        }
        let branch = node.as_unless_node()?;
        let chain_end = branch.end_keyword_loc()?.start_offset();
        Some(Branch {
            keyword: branch.keyword_loc(),
            predicate_end: branch.predicate().location().end_offset(),
            then_keyword: branch.then_keyword_loc(),
            has_body: branch.statements().is_some(),
            next_branch: branch
                .else_clause()
                .map(|clause| clause.else_keyword_loc().start_offset()),
            end: Some(chain_end),
            chain_end,
        })
    }

    /// Where the parser gem's `begin` location starts: the `then` keyword,
    /// or a `;` right after the condition.
    fn begin_offset(&self, file: &SourceFile) -> Option<usize> {
        if let Some(then_keyword) = &self.then_keyword {
            return Some(then_keyword.start_offset());
        }
        let rest = file.bytes.get(self.predicate_end..)?;
        let gap = rest
            .iter()
            .take_while(|&&b| b == b' ' || b == b'\t')
            .count();
        (rest.get(gap) == Some(&b';')).then_some(self.predicate_end + gap)
    }

    /// Where the lines searched for comments stop: the next `elsif`/`else`,
    /// else the `end`.
    fn comment_range_end(&self) -> usize {
        self.next_branch.unwrap_or(self.chain_end)
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/empty_conditional_body_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). The
    //! `AllowComments: false` examples do not apply (rustocop fixes the
    //! default, `true`); under it they are no-offense cases, covered below.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyConditionalBody")
            .collect()
    }

    /// (line, col, message) triples.
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn at(line: u32, col: u32, keyword: &str) -> (u32, u32, String) {
        (
            line,
            col,
            format!("Avoid `{keyword}` branches without a body."),
        )
    }

    #[test]
    fn reference_ranges_and_parenthesized_comment_window() {
        let result = offenses("if foo(\n  x\n)\nelse\n  y\nend\n");
        assert_eq!(result.len(), 1);
        assert_eq!(
            (result[0].line, result[0].col, result[0].end_line),
            (1, 1, 4)
        );
        assert!(offenses("(if x\nend\n# noop\n)\n").is_empty());
        // CommentsHelp extends only `if`, not `unless`, to its direct parens.
        assert_eq!(
            found("(unless x\nend\n# noop\n)\n"),
            vec![at(1, 2, "unless")]
        );
    }

    #[test]
    fn flags_missing_if_body() {
        assert_eq!(found("if condition\nend\n"), vec![at(1, 1, "if")]);
    }

    #[test]
    fn flags_missing_if_and_else_body() {
        assert_eq!(found("if condition\nelse\nend\n"), vec![at(1, 1, "if")]);
        assert_eq!(
            found("def foo\n  if condition\n  else\n  end\nend\n"),
            vec![at(2, 3, "if")]
        );
    }

    #[test]
    fn flags_missing_if_body_with_else_body() {
        assert_eq!(
            found("class Foo\n  if condition\n  else\n    do_something\n  end\nend\n"),
            vec![at(2, 3, "if")]
        );
        assert_eq!(
            found("if condition; else\n  do_something\nend\n"),
            vec![at(1, 1, "if")]
        );
        assert_eq!(
            found("if condition\nelse\n  do_something\nend\n"),
            vec![at(1, 1, "if")]
        );
        assert_eq!(
            found("if condition\nelse\n  do_something if x\nend\n"),
            vec![at(1, 1, "if")]
        );
    }

    #[test]
    fn flags_missing_unless_body() {
        assert_eq!(found("unless condition\nend\n"), vec![at(1, 1, "unless")]);
        assert_eq!(
            found("unless condition\nelse\n  do_something\nend\n"),
            vec![at(1, 1, "unless")]
        );
        assert_eq!(
            found("unless condition\nelse\n  do_something if x\nend\n"),
            vec![at(1, 1, "unless")]
        );
        assert_eq!(
            found("unless condition\nelse\nend\n"),
            vec![at(1, 1, "unless")]
        );
    }

    #[test]
    fn accepts_single_line_if_with_semicolon() {
        // Left to `Style/IfWithSemicolon`.
        for source in [
            "if condition; else do_something end\n",
            "if condition; end\n",
            "if condition then end\n",
            "unless condition; end\n",
            "if a ;   end\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_missing_elsif_body() {
        assert_eq!(
            found(
                "if condition\n  do_something1\nelsif other_condition1\nelsif other_condition2\n  do_something2\nend\n"
            ),
            vec![at(3, 1, "elsif")]
        );
        assert_eq!(
            found(
                "if condition\n  do_something\nelsif other_condition\nelse\n  do_something_else\nend\n"
            ),
            vec![at(3, 1, "elsif")]
        );
        assert_eq!(
            found("if foo\n  do_foo\nelsif bar\n  do_bar\nelsif baz\nend\n"),
            vec![at(5, 1, "elsif")]
        );
    }

    #[test]
    fn flags_missing_if_and_elsif_bodies() {
        assert_eq!(
            found(
                "if condition\nelsif other_condition1\nelsif other_condition2\n  do_something\nend\n"
            ),
            vec![at(1, 1, "if"), at(2, 1, "elsif")]
        );
        assert_eq!(
            found("if condition\nelsif other_condition\nend\n"),
            vec![at(1, 1, "if"), at(2, 1, "elsif")]
        );
        assert_eq!(
            found("if condition\nelsif other_condition1\nelsif other_condition2\nend\n"),
            vec![at(1, 1, "if"), at(2, 1, "elsif"), at(3, 1, "elsif")]
        );
        assert_eq!(
            found(
                "if condition\nelsif other_condition\n  do_something\nelsif another_condition\n  do_something_else\nend\n"
            ),
            vec![at(1, 1, "if")]
        );
    }

    #[test]
    fn flags_then_and_end_on_different_lines() {
        assert_eq!(found("if a then\nend\n"), vec![at(1, 1, "if")]);
        assert_eq!(found("(if a\n end)\n"), vec![at(1, 2, "if")]);
        assert_eq!(
            found("if a\nelsif b then end\n"),
            vec![at(1, 1, "if"), at(2, 1, "elsif")]
        );
    }

    #[test]
    fn flags_missing_elsif_body_with_end_on_the_same_line() {
        assert_eq!(
            found("if cond_a\n  do_a\nelsif cond_b;end\n"),
            vec![at(3, 1, "elsif")]
        );
        assert_eq!(
            found("if cond_a\n  do_a\nelsif cond_b;else;end\n"),
            vec![at(3, 1, "elsif")]
        );
    }

    #[test]
    fn flags_elsif_without_body_when_only_the_else_has_a_comment() {
        assert_eq!(
            found("if condition\n  do_something\nelsif other_condition\nelse\n  # noop\nend\n"),
            vec![at(3, 1, "elsif")]
        );
    }

    #[test]
    fn flags_if_used_as_a_value() {
        assert_eq!(
            found("if condition\nend.do_something\n"),
            vec![at(1, 1, "if")]
        );
        assert_eq!(
            found("if condition\nend&.do_something\n"),
            vec![at(1, 1, "if")]
        );
        assert_eq!(
            found("x = if foo\nelsif bar\n  5\nend\n"),
            vec![at(1, 5, "if")]
        );
        assert_eq!(found("x = if foo\nend\n"), vec![at(1, 5, "if")]);
        assert_eq!(found("x || if foo\nend\n"), vec![at(1, 6, "if")]);
    }

    #[test]
    fn flags_multi_line_value_omission_in_unless() {
        let source = "\
var =
  # This is the value of `other:`, like so: `other: condition || other_condition`
  unless object.action value:, other:
    condition || other_condition
  end
";
        assert_eq!(found(source), vec![at(3, 3, "unless")]);
    }

    #[test]
    fn accepts_empty_branches_with_comments() {
        for source in [
            "if condition\n  # noop\nend\n",
            "if condition1\n  do_something1\nend\nif condition2\n  # noop\nend\n",
            "if condition1\n  do_something1\nelsif condition2\n  do_something2\nelsif condition3\n  # noop\nend\n",
            "if condition1\n  do_something1\nelsif condition2\n  do_something2\nelsif condition3\n  do_something3\nelsif condition4\n  # noop\nend\n",
            "if condition\n  do_something\nelsif other_condition\n  # noop\nend\n",
            "if condition\n  do_something\nelsif other_condition # no op, but avoid going into the else\nelse\n  do_other_things\nend\n",
            "unless condition\n  # noop\nend\n",
            "if condition # noop\nend\n",
            "if condition\n=begin\nnoop\n=end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn comments_outside_the_branch_lines_do_not_count() {
        assert_eq!(found("if condition\nend # noop\n"), vec![at(1, 1, "if")]);
        assert_eq!(found("# noop\nif condition\nend\n"), vec![at(2, 1, "if")]);
        assert_eq!(
            found("if condition\nelse # noop\n  a\nend\n"),
            vec![at(1, 1, "if")]
        );
        assert_eq!(
            found("if condition\n  \"# not a comment\" if a\nelsif b\nend\n"),
            vec![at(3, 1, "elsif")]
        );
    }

    #[test]
    fn accepts_nil_bodies() {
        for source in [
            "if condition\n  nil\nend\n",
            "unless condition\n  nil\nend\n",
            "if condition\n  do_something\nelsif other_condition\n  nil\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "foo if bar\n",
            "foo unless bar\n",
            "a ? b : c\n",
            "if a\n  ()\nend\n",
            "if a\n  b\nelse\nend\n",
            "while a\nend\n",
            "case a\nwhen b\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
