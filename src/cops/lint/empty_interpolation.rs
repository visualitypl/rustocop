//! `Lint/EmptyInterpolation` — flags interpolations that add nothing:
//! `#{}`, `#{ }`, `#{nil}`, `#{''}`, `#{""}`.
//!
//! Matches RuboCop's semantics: an interpolation whose statements are all
//! `nil` or empty string literals is reported, in every string-like literal
//! (strings, symbols, backticks, regexps, heredocs), unless its nearest
//! enclosing array is a percent literal (`%W[#{''} a]`, `%I[...]`), where an
//! empty element can be intended.
//!
//! The cop walks the tree itself from the program node: whether an array is
//! a percent literal is ancestry the shared walker does not provide.

use ruby_prism::{ArrayNode, EmbeddedStatementsNode, Node, Visit};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/EmptyInterpolation` cop.
pub struct EmptyInterpolation;

impl Cop for EmptyInterpolation {
    fn name(&self) -> &'static str {
        "Lint/EmptyInterpolation"
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
        let mut walker = Walker {
            cop: self,
            file,
            out,
            arrays: Vec::new(),
        };
        walker.visit_program_node(&program);
    }
}

/// Walks the whole tree, tracking enclosing arrays.
struct Walker<'a> {
    cop: &'a EmptyInterpolation,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    /// For each enclosing array, innermost last: whether it is a percent
    /// literal.
    arrays: Vec<bool>,
}

impl<'pr> Visit<'pr> for Walker<'_> {
    fn visit_array_node(&mut self, node: &ArrayNode<'pr>) {
        let percent = node
            .opening_loc()
            .is_some_and(|opening| opening.as_slice().starts_with(b"%"));
        self.arrays.push(percent);
        ruby_prism::visit_array_node(self, node);
        self.arrays.pop();
    }

    fn visit_embedded_statements_node(&mut self, node: &EmbeddedStatementsNode<'pr>) {
        let in_percent_array = self.arrays.last().copied().unwrap_or(false);
        if !in_percent_array && is_empty_interpolation(node) {
            let (line, col) = self.file.line_col(node.location().start_offset());
            self.out.push(Offense {
                correction: None,
                cop: self.cop.name(),
                severity: self.cop.severity(),
                message: "Empty interpolation detected.".to_string(),
                line,
                end_line: self.file.last_line_of(node.location()),
                col,
            });
        }
        ruby_prism::visit_embedded_statements_node(self, node);
    }
}

/// Whether every statement of the interpolation is `nil` or `''`/`""`.
fn is_empty_interpolation(node: &EmbeddedStatementsNode<'_>) -> bool {
    node.statements().is_none_or(|statements| {
        statements.body().iter().all(|statement| {
            statement.as_nil_node().is_some() || is_empty_quoted_string(&statement)
        })
    })
}

/// An empty `''` or `""` literal: the parser gem's empty `str`. Other empty
/// spellings (`%()`, `%q()`, an empty heredoc) are a `dstr` there, which
/// RuboCop does not treat as empty.
fn is_empty_quoted_string(node: &Node<'_>) -> bool {
    node.as_string_node().is_some_and(|string| {
        string.unescaped().is_empty()
            && string
                .opening_loc()
                .is_some_and(|opening| matches!(opening.as_slice(), b"'" | b"\""))
    })
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/empty_interpolation_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyInterpolation")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_empty_interpolation_with_message() {
        let found = offenses("\"this is the #{}\"\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 14));
        assert_eq!(found[0].message, "Empty interpolation detected.");
    }

    #[test]
    fn flags_blank_nil_and_empty_string_interpolations() {
        for source in [
            "\"this is the #{ }\"\n",
            "\"this is the #{''}\"\n",
            "\"this is the #{\"\"}\"\n",
            "\"this is the #{nil}\"\n",
        ] {
            assert_eq!(positions(source), vec![(1, 14)], "should flag {source:?}");
        }
    }

    #[test]
    fn flags_interpolations_in_string_like_contexts() {
        assert_eq!(
            positions("/regexp #{}/\n`backticks #{}`\n:\"symbol #{}\"\n"),
            vec![(1, 9), (2, 12), (3, 10)]
        );
    }

    #[test]
    fn flags_empty_string_interpolation_inside_a_bracket_array() {
        assert_eq!(positions("[\"#{''}\", one, two]\n"), vec![(1, 3)]);
    }

    #[test]
    fn flags_more_empty_shapes() {
        assert_eq!(positions("\"#{nil; ''}\"\n"), vec![(1, 2)]);
        assert_eq!(positions("x = <<~TEXT\n  a #{}\nTEXT\n"), vec![(2, 5)]);
        assert_eq!(positions("\"a\" \"#{}\"\n"), vec![(1, 6)]);
        assert_eq!(positions("/#{}/ =~ x\n"), vec![(1, 2)]);
        // The nearest array decides: a bracket array inside `%W` is checked.
        assert_eq!(positions("%W[#{[\"#{''}\"]}]\n"), vec![(1, 8)]);
    }

    #[test]
    fn accepts_non_empty_interpolation() {
        for source in [
            "\"this is #{top} silly\"\n",
            "\"this is the #{1}\"\n",
            "\"this is the #{true}\"\n",
            "\"#{'a'}\"\n",
            "\"#{\"\" \"\"}\"\n",
            "\"#{:\"\"}\"\n",
            "\"#@foo\"\n",
            // The parser gem reads these empty strings as `dstr`, which
            // RuboCop keeps.
            "\"#{%()}\"\n",
            "\"#{%q()}\"\n",
            "\"#{%Q()}\"\n",
            "\"#{<<~X}\"\nX\n",
            "\"#{%w[]}\"\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_empty_string_interpolation_inside_percent_literal_arrays() {
        for source in [
            "%W[#{''} one two]\n",
            "%I[#{''} one two]\n",
            "%W[a#{} b]\n",
            "%W[#{\"#{''}\"}]\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
