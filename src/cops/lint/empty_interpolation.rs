//! `Lint/EmptyInterpolation` — flags interpolations that add nothing:
//! `#{}`, `#{ }`, `#{nil}`, `#{''}`, `#{""}`.
//!
//! Matches RuboCop's semantics: an interpolation whose statements are all
//! `nil` or empty string literals is reported, in every string-like literal
//! (strings, symbols, backticks, regexps, heredocs), unless its nearest
//! enclosing array is a percent literal (`%W[#{''} a]`, `%I[...]`), where an
//! empty element can be intended.
//!
//! The nearest array is obtained from the shared complete NodeContext stack.

use ruby_prism::{EmbeddedStatementsNode, Node};

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
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(interpolation) = node.as_embedded_statements_node() else {
            return;
        };
        let mut child = node;
        let in_percent_array = cx
            .ancestors()
            .find_map(|ancestor| {
                let result = if let Some(array) = ancestor.as_array_node() {
                    Some(
                        array
                            .opening_loc()
                            .is_some_and(|o| o.as_slice().starts_with(b"%")),
                    )
                } else if ancestor.as_rescue_node().is_some_and(|rescue| {
                    rescue
                        .exceptions()
                        .iter()
                        .any(|exception| crate::cops::ast::same_node(&exception, child))
                }) {
                    // Parser's resbody wraps its exception list in a synthetic,
                    // ordinary array. This nearest array overrides an outer %W/%I.
                    Some(false)
                } else {
                    None
                };
                child = ancestor;
                result
            })
            .unwrap_or(false);
        if in_percent_array || !is_empty_interpolation(&interpolation, file) {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Empty interpolation detected.".to_string(),
            line,
            end_line: file.last_line_of(node.location()),
            col,
        });
    }
}

/// Whether every statement of the interpolation is `nil` or `''`/`""`.
fn is_empty_interpolation(node: &EmbeddedStatementsNode<'_>, file: &SourceFile) -> bool {
    node.statements().is_none_or(|statements| {
        statements.body().iter().all(|statement| {
            statement.as_nil_node().is_some() || (statement.as_string_node().is_some()
                && matches!(super::literal_assignment_in_condition::scope_literal(&statement, file),
                    Some(super::literal_assignment_in_condition::ScopeLiteral::Str(value)) if value.is_empty()))
        })
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
    fn empty_value_without_a_parser_str_child_is_not_removed() {
        assert!(offenses("\"#{\"\\\n\"}\"\n").is_empty());
        assert_eq!(positions("\"#{\"\\u{}\"}\"\n"), vec![(1, 2)]);
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
    fn rescue_exception_list_is_a_nearer_ordinary_array() {
        assert_eq!(
            positions("%W[#{begin; x; rescue Foo(\"#{nil}\"); end}]\n"),
            vec![(1, 28)]
        );
        assert!(offenses("%W[#{begin; x; rescue Foo; \"#{nil}\"; end}]\n").is_empty());
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
