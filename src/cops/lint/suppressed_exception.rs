//! `Lint/SuppressedException` — flags `rescue` clauses with an empty body,
//! which silently swallow the exception.
//!
//! Matches RuboCop's semantics with its defaults (`AllowComments: true`,
//! `AllowNil: true`), fixed in rustocop:
//!
//! - a clause whose body is just `nil` is accepted, so modifier `rescue`
//!   (which always has a body) is never flagged;
//! - an empty clause is accepted when a whole-line comment (`/^\s*#/`) sits
//!   on a line after the `rescue` keyword, up to and including the `end` line
//!   of the nearest enclosing `begin`, `def` or block (lambdas included).
//!   Like RuboCop, a clause with no such enclosing node (a `class` body at the
//!   top level) is flagged whatever its comments, and the check is textual,
//!   so `#` lines inside a string literal count too.
//!
//! The offense is reported at the `rescue` keyword.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/SuppressedException` cop.
pub struct SuppressedException;

impl Cop for SuppressedException {
    fn name(&self) -> &'static str {
        "Lint/SuppressedException"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_rescue_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(rescue) = node.as_rescue_node() else {
            return;
        };
        // A real body handles the exception, and a lone `nil` is accepted
        // (AllowNil), so only empty clauses can offend.
        if rescue.statements().is_some() {
            return;
        }
        let rescue_line = file.line_col(rescue.keyword_loc().start_offset()).0;
        if let Some(scope_end) = nearest_scope_end(cx)
            && (rescue_line + 1..=file.line_col(scope_end).0)
                .any(|line| is_comment_line(file.line_text(line)))
        {
            return;
        }
        let (line, col) = file.reference_line_col(rescue.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Do not suppress exceptions.".to_string(),
            line,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            col,
        });
    }
}

/// The offset of the `end` of the clause's nearest enclosing scope:
/// RuboCop's nearest `kwbegin`, `any_def` or `any_block` ancestor (a later
/// clause of the same `begin` is no scope); `None` outside any.
fn nearest_scope_end(cx: &NodeContext<'_, '_>) -> Option<usize> {
    cx.ancestors().find_map(|ancestor| match ancestor {
        // Only an explicit `begin` is a scope; the implicit one of a `def`
        // or block body is part of it.
        Node::BeginNode { .. } => {
            let begin = ancestor.as_begin_node().unwrap();
            begin.begin_keyword_loc()?;
            Some(end_offset(begin.end_keyword_loc().as_ref(), ancestor))
        }
        Node::DefNode { .. } => {
            let def = ancestor.as_def_node().unwrap();
            Some(end_offset(def.end_keyword_loc().as_ref(), ancestor))
        }
        Node::BlockNode { .. } => {
            let block = ancestor.as_block_node().unwrap();
            Some(end_offset(Some(&block.closing_loc()), ancestor))
        }
        Node::LambdaNode { .. } => {
            let lambda = ancestor.as_lambda_node().unwrap();
            Some(end_offset(Some(&lambda.closing_loc()), ancestor))
        }
        _ => None,
    })
}

/// Where a scope's closing token starts, or its last byte when it has none
/// (RuboCop's `loc.end&.line || loc.last_line`, once mapped to a line).
fn end_offset(closing: Option<&ruby_prism::Location<'_>>, node: &Node<'_>) -> usize {
    match closing {
        Some(closing) => closing.start_offset(),
        None => node.location().end_offset().saturating_sub(1),
    }
}

/// RuboCop's `comment_line?`: the line matches `/^\s*#/`.
fn is_comment_line(text: &[u8]) -> bool {
    text.iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\x0c' | b'\x0b'))
        .is_some_and(|&byte| byte == b'#')
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/suppressed_exception_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for its default
    //! configuration (`AllowComments: true`, `AllowNil: true`).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/SuppressedException")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_empty_rescue_in_def_with_message() {
        let found = offenses("def foo\n  do_something\nrescue\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 1));
        assert_eq!(found[0].message, "Do not suppress exceptions.");
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for source in [
            "def self.foo\n  do_something\nrescue\nend\n",
            "foo do\n  do_something\nrescue\nend\n",
            "foo do\n  _1\nrescue\nend\n",
            "begin\n  something\nrescue\nend\n",
        ] {
            assert_eq!(positions(source), vec![(3, 1)], "should flag {source:?}");
        }
    }

    #[test]
    fn flags_single_line_rescue_despite_comments_around_it() {
        let source = "\
RSpec.describe Dummy do
  it 'dummy spec' do
    # This rescue is here to ensure the test does not fail because of the `raise`
    expect { begin subject; rescue ActiveRecord::Rollback; end }.not_to(change(Post, :count))
    # Done
  end
end
";
        assert_eq!(positions(source), vec![(4, 29)]);
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "begin\n  something\n  return\nrescue\n  file.close\nend\n",
            "begin\n  something\n  return\nrescue\n  # do nothing\nend\n",
            "def foo\n  do_something\nrescue\n  # do nothing\nend\n",
            "def self.foo\n  do_something\nrescue\n  # do nothing\nend\n",
            "foo do\nrescue\n  # do nothing\nend\n",
            "foo do\n  _1\nrescue\n  # do nothing\nend\n",
            "begin\n  do_something\nrescue\n  nil\nend\n",
            "something rescue nil\n",
            "def some_method = other_method(42) rescue nil\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_every_empty_clause_and_reference_only_clauses() {
        assert_eq!(
            positions("begin\n  a\nrescue A\nrescue B => e\n  handle(e)\nrescue => e\nend\n"),
            vec![(3, 1), (6, 1)]
        );
        assert_eq!(
            positions("begin\n  a\nrescue\nelse\n  b\nensure\n  c\nend\n"),
            vec![(3, 1)]
        );
    }

    #[test]
    fn comments_must_be_whole_lines_after_rescue_within_the_scope() {
        // A trailing comment on the `rescue` line, or one after `end`,
        // does not count; nor does an `=begin` block.
        assert_eq!(
            positions("begin\n  a\nrescue # trailing\nend\n"),
            vec![(3, 1)]
        );
        assert_eq!(positions("begin\n  a\nrescue\nend # after\n"), vec![(3, 1)]);
        assert_eq!(
            positions("begin\n  a\nrescue\n=begin\n=end\nend\n"),
            vec![(3, 1)]
        );
        assert_eq!(
            positions("begin\n  a\nrescue\nend\n# later\n"),
            vec![(3, 1)]
        );
        // A comment anywhere up to the scope's `end`, even after a later
        // clause, counts.
        assert!(offenses("begin\n  a\nrescue\nensure\n  b\n  # note\nend\n").is_empty());
    }

    #[test]
    fn scope_is_the_nearest_begin_def_or_block() {
        // The nearest scope is the inner `begin`; the comment is past its end.
        assert_eq!(
            positions("def m\n  begin\n    a\n  rescue\n  end\n  # outer\nend\n"),
            vec![(4, 3)]
        );
        // Lambdas and `do` blocks are scopes; class bodies are not.
        assert!(offenses("x = -> do\n  a\nrescue\n  # c\nend\n").is_empty());
        assert_eq!(positions("y = lambda do\n  a\nrescue\nend\n"), vec![(3, 1)]);
        assert!(
            offenses("Module.new do\n  class Bar\n    a\n  rescue\n    # c\n  end\nend\n")
                .is_empty()
        );
        // With no enclosing scope at all, comments never excuse it.
        assert_eq!(
            positions("class Foo\n  bar\nrescue\n  # comment\nend\n"),
            vec![(3, 1)]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "def m\n  a\nrescue\n  (nil)\nend\n",
            "def n\n  a\nrescue A\n  nil\nrescue\n  nil; nil\nend\n",
            "begin\n  a\nrescue\n  x = \"\n  # in string\n  \"\nend\n",
            "foo rescue bar\n",
            "begin\n  a\nensure\nend\n",
            "begin\nrescue => e\n  raise\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
