//! `Lint/EachWithObjectArgument` — immutable literal accumulators.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// The detection-only `Lint/EachWithObjectArgument` cop.
pub struct EachWithObjectArgument;

impl Cop for EachWithObjectArgument {
    fn name(&self) -> &'static str {
        "Lint/EachWithObjectArgument"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"each_with_object" {
            return;
        }
        let arguments = ast::call_arguments(&call);
        let [argument] = arguments.as_slice() else {
            return;
        };
        // RuboCop's immutable_literal? is a shallow syntactic predicate,
        // including dynamic symbols, but not grouped expressions, ranges,
        // regexps, frozen strings or values inferred through local bindings.
        if !matches!(
            argument,
            Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::RationalNode { .. }
                | Node::ImaginaryNode { .. }
                | Node::SymbolNode { .. }
                | Node::InterpolatedSymbolNode { .. }
                | Node::TrueNode { .. }
                | Node::FalseNode { .. }
                | Node::NilNode { .. }
                | Node::SourceLineNode { .. }
        ) && ast::negated_numeric_literal(argument).is_none()
        {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            message: "The argument to each_with_object cannot be immutable.".into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/each_with_object_argument_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EachWithObjectArgument")
            .collect()
    }

    #[test]
    fn pinned_numeric_and_safe_navigation_examples() {
        for expression in ["0", "0.1", "100000000000000000000"] {
            for operator in [".", "&."] {
                let source = format!(
                    "\n  collection{operator}each_with_object({expression}) {{ |e, a| a + e }}\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    "The argument to each_with_object cannot be immutable."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn immutable_types_are_shallow_syntax_including_dynamic_symbols_and_line_numbers() {
        for argument in [
            "true",
            "false",
            "nil",
            ":symbol",
            ":\"#{value}\"",
            ":''",
            "__LINE__",
            "-1",
            "--1",
            "1r",
            "-0.1r",
            "1i",
            "-0.1ri",
        ] {
            let source = format!("each_with_object({argument})");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
        for argument in [
            "__ENCODING__",
            "__FILE__",
            "0.freeze",
            "''.freeze",
            "(nil)",
            "begin; nil; end",
        ] {
            assert!(offenses(&format!("each_with_object({argument})")).is_empty());
        }
    }

    #[test]
    fn accepts_mutable_variable_and_wrong_arity_arguments() {
        for argument in [
            "x",
            "1, 2",
            "''",
            "[]",
            "{}",
            "1..2",
            "/x/",
            "(0)",
            "0, &block",
            "*items",
        ] {
            let source = format!("collection.each_with_object({argument}) {{ |e, a| a.add(e) }}");
            // Explicit &block and a literal block cannot coexist.
            let source = if argument.contains('&') {
                format!("collection.each_with_object({argument})")
            } else {
                source
            };
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
}
