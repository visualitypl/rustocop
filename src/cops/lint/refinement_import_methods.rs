//! `Lint/RefinementImportMethods` — removed refinement mixin calls.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct RefinementImportMethods;

impl Cop for RefinementImportMethods {
    fn name(&self) -> &'static str {
        "Lint/RefinementImportMethods"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.receiver().is_some()
            || !matches!(call.name().as_slice(), b"include" | b"prepend")
            // A literal block makes this send's parent its own block in RuboCop.
            || call.block().is_some_and(|b| b.as_block_node().is_some())
        {
            return;
        }
        let mut ancestors = cx.ancestors();
        // Prism always supplies Statements; RuboCop only adds begin for a
        // sequence. Parentheses, explicit begin, inner calls and parameter
        // defaults are barriers, not transparent enclosing-refine searches.
        let Some(statements) = ancestors.next().and_then(Node::as_statements_node) else {
            return;
        };
        if statements.body().len() != 1 {
            return;
        }
        let Some(block) = ancestors.next().and_then(Node::as_block_node) else {
            return;
        };
        if block.parameters().is_some_and(|p| {
            p.as_numbered_parameters_node().is_some() || p.as_it_parameters_node().is_some()
        }) {
            return;
        }
        if !ancestors
            .next()
            .and_then(Node::as_call_node)
            .is_some_and(|c| c.name().as_slice() == b"refine")
        {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(selector.start_offset(), file);
        let method = if call.name().as_slice() == b"include" {
            "include"
        } else {
            "prepend"
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(selector),
            message: format!(
                "Use `import_methods` instead of `{method}` because it was removed in Ruby 3.2."
            ),
            // Snapshot imports are not equivalent to live include/prepend.
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/refinement_import_methods_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RefinementImportMethods")
            .collect()
    }

    #[test]
    fn pinned_removed_message_and_selector_only_location() {
        for method in ["include", "prepend"] {
            let source = format!("refine Foo do\n  {method}(\n    Bar\n  )\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!(
                    "Use `import_methods` instead of `{method}` because it was removed in Ruby 3.2."
                )
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn requires_immediate_ordinary_refine_parent_not_merely_an_ancestor() {
        for body in [
            "include M; nil",
            "nil; prepend M",
            "(include M)",
            "begin; include M; end",
            "other { include M }",
            "-> { prepend M }",
            "lambda { include M }",
            "proc { include M }",
            "include M if flag",
            "consume(include M)",
            "include(_1)",
            "prepend(it)",
            "include(M) {}",
            "self.include M",
            "self&.prepend M",
        ] {
            let source = format!("refine(Foo) {{ {body} }}\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        // Only the inner refine has a sole direct body; the outer sequence
        // must not suppress this genuine nested finding.
        let source = "refine(Foo) do\n nil\n other.refine(Bar) do\n  include M\n end\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (4, 3));
    }

    #[test]
    fn accepts_arbitrary_arguments_refine_receivers_and_explicit_parameters() {
        for refine in ["refine", "self.refine", "other&.refine"] {
            for params in ["", "||", "|v|", "|v = include(M)|", "|; v|"] {
                for call in [
                    "include",
                    "include()",
                    "include(A, B)",
                    "prepend(*mods)",
                    "prepend(M, &block)",
                ] {
                    let source = format!("{refine}(Foo) do {params}\n  {call}\nend\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "refine Foo do\n  import_methods Bar\nend\n",
            "refine Foo do\n  Bar.include Baz\nend\n",
            "include Foo\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
