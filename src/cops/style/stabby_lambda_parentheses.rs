//! `Style/StabbyLambdaParentheses` — fixed `require_parentheses`.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct StabbyLambdaParentheses;
impl Cop for StabbyLambdaParentheses {
    fn name(&self) -> &'static str {
        "Style/StabbyLambdaParentheses"
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
        let Some(parameters) = lambda
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
        else {
            return;
        };
        if parameters.opening_loc().is_some() {
            return;
        }
        let loc = parameters.location();
        let (line, col) = file.line_col(loc.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().next_back()?;
                if file.class_method_bindings(root).observes_sources {
                    return None;
                }
                let edit = Correction {
                    range: loc.start_offset()..loc.end_offset(),
                    replacement: format!("({})", std::str::from_utf8(loc.as_slice()).ok()?),
                };
                super::correction::preserves_data_positions(file, cx, &edit).then_some(edit)
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            message: "Wrap stabby lambda arguments with parentheses.".into(),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/stabby_lambda_parentheses_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_diagnostic_and_correction() {
        let found: Vec<_> = lint_str("->a,b,c { a + b + c }\n")
            .into_iter()
            .filter(|o| o.cop == StabbyLambdaParentheses.name())
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 1));
        assert_eq!(
            found[0].message,
            "Wrap stabby lambda arguments with parentheses."
        );
        assert!(found[0].correction.is_none());
        assert_correction(
            &StabbyLambdaParentheses,
            "->a,b,c { a + b + c }\n",
            "->(a,b,c) { a + b + c }\n",
        );
    }
    #[test]
    fn pinned_negatives_and_implicit_parameters() {
        for source in [
            "lambda(&:nil?)",
            "-> { true }",
            "o.lambda",
            "->(a,b,c) { a + b + c }",
            "->() { 1 }",
            "-> { _1 }",
            "-> { it }",
            "proc { |x| x }",
            "'->a {}' # ->b {}",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == StabbyLambdaParentheses.name()),
                "{source}"
            );
        }
    }
    #[test]
    fn wraps_only_explicit_parameter_span_without_moving_tokens() {
        for (before, after) in [
            (
                "-> a, b=1, *xs, c, key:, flag: 2, **kw, &blk { [a,b,xs,c,key,flag,kw,blk] }",
                "-> (a, b=1, *xs, c, key:, flag: 2, **kw, &blk) { [a,b,xs,c,key,flag,kw,blk] }",
            ),
            ("->x, (a,b) { [x,a,b] }", "->(x, (a,b)) { [x,a,b] }"),
            (
                "->a, # keep\n b do\n [a,b,__LINE__]\nend",
                "->(a, # keep\n b) do\n [a,b,__LINE__]\nend",
            ),
            (
                "->a = 'a,b', key: /[,)]/ { [a,key] }",
                "->(a = 'a,b', key: /[,)]/) { [a,key] }",
            ),
            (
                "->a = <<~TEXT do\n literal ) { #\nTEXT\n a\nend",
                "->(a = <<~TEXT) do\n literal ) { #\nTEXT\n a\nend",
            ),
        ] {
            assert_correction(&StabbyLambdaParentheses, before, after);
        }
    }
}
