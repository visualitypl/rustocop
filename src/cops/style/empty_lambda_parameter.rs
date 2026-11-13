//! `Style/EmptyLambdaParameter` — omit explicit empty arrow-lambda parentheses.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EmptyLambdaParameter;
impl Cop for EmptyLambdaParameter {
    fn name(&self) -> &'static str {
        "Style/EmptyLambdaParameter"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_lambda_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(lambda) = node.as_lambda_node() else {
            return;
        };
        let Some(parameters) = super::empty_parameters::empty(lambda.parameters()) else {
            return;
        };
        super::empty_parameters::report(
            self,
            &parameters,
            lambda.operator_loc().end_offset(),
            cx,
            file,
            out,
            "Omit parentheses for the empty lambda parameters.",
        );
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/empty_lambda_parameter_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_offense() {
        let found: Vec<_> = lint_str("-> () { do_something }\n")
            .into_iter()
            .filter(|o| o.cop == EmptyLambdaParameter.name())
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 4, 1));
        assert_eq!(
            found[0].message,
            "Omit parentheses for the empty lambda parameters."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn pinned_no_offenses_and_parameter_lookalikes() {
        for source in [
            "lambda { || do_something }",
            "def foo\n  super { || do_something }\nend\n",
            "-> { 1 }",
            "->(x) { x }",
            "->(;x) { x }",
            "->(*) { }",
            "->(**) { }",
            "->(&) { }",
            "->(x = 1) { x }",
            "->(x:) { x }",
            "-> { _1 }",
            "-> { it }",
            "proc { || 1 }",
            "# ->() {}\ntext = '->() {}'",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == EmptyLambdaParameter.name()),
                "{source}"
            );
        }
    }
    #[test]
    fn pinned_correction() {
        assert_correction(
            &EmptyLambdaParameter,
            "-> () { do_something }\n",
            "-> { do_something }\n",
        );
    }
}
