//! `Lint/EmptyExpression` — empty parenthesized expressions and interpolations.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct EmptyExpression;

impl Cop for EmptyExpression {
    fn name(&self) -> &'static str {
        "Lint/EmptyExpression"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Both represent RuboCop's `begin` nodes. Explicit begin/end is a
        // `kwbegin`, not an expression targeted by this cop.
        let empty = if let Some(parentheses) = node.as_parentheses_node() {
            parentheses.body().is_none()
        } else if let Some(embedded) = node.as_embedded_statements_node() {
            embedded.statements().is_none()
        } else {
            false
        };
        if !empty {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: "Avoid empty expressions.".into(),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/empty_expression_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyExpression")
            .collect()
    }

    #[test]
    fn flags_all_pinned_expression_contexts() {
        for (source, line, col) in [
            ("()", 1, 1),
            ("(())", 1, 2),
            ("if (); end", 1, 4),
            ("if foo\n  1\nelsif ()\n  2\nend", 3, 7),
            ("case ()\nwhen :foo then 1\nend", 1, 6),
            ("case foo\nwhen () then 1\nend", 2, 6),
            ("() ? true : false", 1, 1),
            ("foo ? () : bar", 1, 7),
            ("def foo\n  ()\nend", 2, 3),
            ("if foo\n  ()\nend", 2, 3),
            ("case foo\nwhen :bar then ()\nend", 2, 16),
            ("foo = ()", 1, 7),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line),
                "{source}"
            );
            assert_eq!(found[0].message, "Avoid empty expressions.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn flags_only_innermost_empty_groups_and_empty_interpolation_statements() {
        for (source, line, col, end) in [
            ("((()))", 1, 3, 1),
            ("  (\n# comment\n)", 1, 3, 3),
            ("\"#{}\"", 1, 2, 1),
            ("%W[#{;} a]", 1, 4, 1),
            ("def f(x = ()); end", 1, 11, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end),
                "{source}"
            );
        }
    }

    #[test]
    fn ignores_nonempty_groups_explicit_begin_and_syntactic_lookalikes() {
        for source in [
            "(nil)",
            "(false)",
            "(some_expression)",
            "if (some_expression); bar; end",
            "begin; end",
            "foo()",
            "def foo(); end",
            "-> {}",
            "[]",
            "{}",
            "\"()\"",
            "# ()",
            "\"#{nil}\"",
            "__END__\n()\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
