//! `Lint/EmptyEnsure` — flags `ensure` clauses without a body.
//!
//! Matches RuboCop's semantics: an `ensure` keyword followed by no
//! statements (comments do not count) is reported at the keyword, in
//! `begin`/`end`, method and block bodies alike.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/EmptyEnsure` cop.
pub struct EmptyEnsure;

impl Cop for EmptyEnsure {
    fn name(&self) -> &'static str {
        "Lint/EmptyEnsure"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(ensure) = node.as_ensure_node() else {
            return;
        };
        if ensure.statements().is_some() {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            ensure.ensure_keyword_loc().start_offset(),
            file,
        );
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Empty `ensure` block detected.".to_string(),
            line,
            end_line: line,
            col,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/empty_ensure_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyEnsure")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_empty_ensure_with_message() {
        let found = offenses("begin\n  something\nensure # hello\n  # world\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 1));
        assert_eq!(found[0].message, "Empty `ensure` block detected.");
    }

    #[test]
    fn flags_empty_ensure_in_method_and_block_bodies() {
        assert_eq!(
            positions("def some_method\n  do_something\nensure\nend\n"),
            vec![(3, 1)]
        );
        assert_eq!(positions("foo do\n  bar\n  ensure\nend\n"), vec![(3, 3)]);
        assert_eq!(
            positions("begin\n  a\nrescue\n  b\nensure\nend\n"),
            vec![(5, 1)]
        );
        assert_eq!(positions("begin; a; ensure; end\n"), vec![(1, 11)]);
    }

    #[test]
    fn accepts_non_empty_ensure() {
        assert!(offenses("begin\n  something\n  return\nensure\n  file.close\nend\n").is_empty());
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "begin\n  a\nensure\n  nil\nend\n",
            "def m\n  a\nensure\n  ()\nend\n",
            "begin\n  a\nrescue\nend\n",
            "ensure_cleanup\n",
            "def ensure; end\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
