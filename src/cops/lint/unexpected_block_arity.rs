//! `Lint/UnexpectedBlockArity` — known two-argument methods with too few block slots.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct UnexpectedBlockArity;

impl Cop for UnexpectedBlockArity {
    fn name(&self) -> &'static str {
        "Lint/UnexpectedBlockArity"
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
        // Deliberately syntactic: even nil/self/constants and safe navigation
        // qualify. No receiver type inference, and no receiverless calls.
        let method = call.name();
        if call.receiver().is_none()
            || !matches!(
                method.as_slice(),
                b"chunk_while"
                    | b"each_with_index"
                    | b"each_with_object"
                    | b"inject"
                    | b"max"
                    | b"min"
                    | b"minmax"
                    | b"reduce"
                    | b"slice_when"
                    | b"sort"
            )
        {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        let actual = positional_count(block.parameters());
        if actual >= 2 {
            return;
        }
        let (line, col) = file.reference_line_col(call.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "`{}` expects at least 2 positional arguments, got {actual}.",
                String::from_utf8_lossy(method.as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(call.location()),
            correction: None,
        });
    }
}

fn positional_count(parameters: Option<Node<'_>>) -> usize {
    let Some(parameters) = parameters else {
        return 0;
    };
    if let Some(numbered) = parameters.as_numbered_parameters_node() {
        // Parser-owned maximum, not references from nested implicit scopes.
        return numbered.maximum().into();
    }
    if parameters.as_it_parameters_node().is_some() {
        return 1;
    }
    let Some(parameters) = parameters
        .as_block_parameters_node()
        .and_then(|p| p.parameters())
    else {
        return 0;
    };
    // Only a top-level *rest is variadic. A trailing comma is ImplicitRest,
    // and a rest inside destructuring still occupies just one mlhs slot.
    if parameters
        .rest()
        .is_some_and(|p| p.as_rest_parameter_node().is_some())
    {
        return usize::MAX;
    }
    parameters.requireds().len() + parameters.optionals().len() + parameters.posts().len()
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/unexpected_block_arity_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Ruby27/34 examples both apply.
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnexpectedBlockArity")
            .collect()
    }

    #[test]
    fn pinned_insufficient_explicit_and_implicit_parameters() {
        for (block, count) in [
            ("{ }", 0),
            ("{ |a| a }", 1),
            ("{ |a:, b:| a + b }", 0),
            ("{ |**kwargs| kwargs }", 0),
            ("{ |(a, b)| a + b }", 1),
            ("{ |a = 1| a }", 1),
            ("{ |a; b| a + b }", 1),
            ("{ _1 }", 1),
            ("{ it }", 1),
        ] {
            let source = format!("# example\n  values.reduce {block}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("`reduce` expects at least 2 positional arguments, got {count}.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn pinned_sufficient_parameters_and_receiverless_calls() {
        for block in [
            "{ |a, b| a + b }",
            "{ |a, b, c| a + b }",
            "{ |*x| x }",
            "{ |(a, b), c| a + b + c }",
            "{ |a = 1, b = 2| a + b }",
            "{ _1 + _2 }",
            "{ _1 + _2 + _3 }",
            "{ _2 }",
        ] {
            assert!(
                offenses(&format!("values.reduce {block}")).is_empty(),
                "{block}"
            );
        }
        for block in ["{ }", "{ _1 }", "{ it }"] {
            assert!(offenses(&format!("reduce {block}")).is_empty());
        }
    }

    #[test]
    fn all_fixed_methods_use_syntactic_receivers_including_safe_navigation() {
        for method in [
            "chunk_while",
            "each_with_index",
            "each_with_object",
            "inject",
            "max",
            "min",
            "minmax",
            "reduce",
            "slice_when",
            "sort",
        ] {
            for receiver in [
                "values.",
                "self.",
                "nil.",
                "false.",
                "Other::Thing.",
                "values&.",
            ] {
                let source = format!("{receiver}{method} {{}}");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    found[0].message,
                    format!("`{method}` expects at least 2 positional arguments, got 0.")
                );
            }
            assert!(offenses(&format!("{method} {{}}")).is_empty());
            assert!(offenses(&format!("values.{method}(&block)")).is_empty());
        }
        for source in [
            "values.map {}",
            "values.reduce! {}",
            "values.reduce",
            "lambda { _1 }",
            "->(x) { x }",
            "def reduce; super { _1 }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn only_top_level_positional_rest_is_unbounded_and_posts_count() {
        for parameters in [
            "*",
            "*args",
            "a,*args",
            "*args,b",
            "a=1,b",
            "a,b=1",
            "(a,*b),c",
            "a,b;local",
        ] {
            let source = format!("values.reduce {{ |{parameters}| nil }}");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for (parameters, count) in [
            ("(a,*b)", 1),
            ("((a,b),*c)", 1),
            ("a,", 1),
            ("a,b:,**kw,&blk", 1),
            ("**", 0),
            ("**nil", 0),
            ("&", 0),
            (";local", 0),
        ] {
            let source = format!("values.reduce {{ |{parameters}| nil }}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                found[0].message,
                format!("`reduce` expects at least 2 positional arguments, got {count}.")
            );
        }
    }

    #[test]
    fn nested_implicit_parameters_do_not_inflate_outer_arity() {
        for inner in ["proc { _2 }", "proc { it }"] {
            let source = format!("values.reduce {{ {inner} }}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                found[0].message,
                "`reduce` expects at least 2 positional arguments, got 0."
            );
        }
        let found = offenses("consume(<<TEXT)\n#{values.reduce { _1 }}\nTEXT\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
    }

    #[test]
    fn entire_block_range_excludes_later_heredoc_body() {
        let found = offenses("  values\n    .reduce do |a|\n    a\n  end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 4));
        let found = offenses("values.reduce(<<TEXT) { |a| a }\nbody\nTEXT\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
    }

    #[test]
    fn pinned_multiple_offenses() {
        let found = offenses("values.reduce { |a| a }\nvalues.inject { }\n");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!((found[1].line, found[1].col), (2, 1));
        assert_eq!(
            found[1].message,
            "`inject` expects at least 2 positional arguments, got 0."
        );
    }
}
