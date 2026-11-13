//! `Style/TrailingCommaInBlockArgs` — opt-in, strictly detection-only.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct TrailingCommaInBlockArgs;
impl Cop for TrailingCommaInBlockArgs {
    fn name(&self) -> &'static str {
        "Style/TrailingCommaInBlockArgs"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_block_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Arrow lambdas are LambdaNodes, never these BlockNodes. Ordinary
        // `lambda { |...| }` remains covered, irrespective of method lookup.
        let Some(block) = node.as_block_node() else {
            return;
        };
        let Some(pipes) = block
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
        else {
            return;
        };
        if !pipes.locals().is_empty() {
            return;
        }
        let Some(parameters) = pipes.parameters() else {
            return;
        };
        let Some(comma) = parameters
            .rest()
            .filter(|r| r.as_implicit_rest_node().is_some())
        else {
            return;
        };
        // Match upstream's leaf arg/optarg/kwoptarg count (not rest/kwrest,
        // required keywords or block locals). Nested destructuring is counted,
        // not flattened or rewritten. This small parameter-only walk occurs
        // only for comma candidates, bounded by the pipeline AST depth budget.
        let mut count = ArgumentCount::default();
        crate::cops::walk::walk(&mut count, &parameters.as_node());
        if count.count < 2 {
            return;
        }
        let (line, col) = file.line_col(comma.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Useless trailing comma present in block arguments.".into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

#[derive(Default)]
struct ArgumentCount {
    count: usize,
    splat_parents: Vec<bool>,
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for ArgumentCount {
    fn enter(&mut self, node: Node<'pr>) {
        if matches!(
            node,
            Node::RequiredParameterNode { .. }
                | Node::OptionalParameterNode { .. }
                | Node::OptionalKeywordParameterNode { .. }
        ) && self.splat_parents.last() != Some(&true)
        {
            self.count += 1;
        }
        self.splat_parents.push(node.as_splat_node().is_some());
    }
    fn leave(&mut self) {
        self.splat_parents.pop();
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/trailing_comma_in_block_args_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    fn offenses(source: &str) -> Vec<Offense> {
        correction_offenses(&TrailingCommaInBlockArgs, source)
    }

    #[test]
    fn pinned_diagnostics_never_offer_unsafe_corrections() {
        for (source, col) in [
            ("test { |a, b,| a + b }", 13),
            ("test do |a, b,|\n  a + b\nend", 14),
            ("lambda { |foo, bar,| do_something(foo, bar) }", 19),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(
                found[0].message,
                "Useless trailing comma present in block arguments."
            );
            assert!(found[0].correction.is_none());
        }
        assert!(!TrailingCommaInBlockArgs.safe_fixes());
        assert!(!TrailingCommaInBlockArgs.default_enabled());
        assert!(
            crate::pipeline::lint_str("test { |a, b,| a + b }")
                .iter()
                .all(|o| o.cop != TrailingCommaInBlockArgs.name())
        );
    }

    #[test]
    fn pinned_no_offenses_for_required_commas_defaults_keywords_and_locals() {
        for (open, close) in [("{", "}"), ("do", "end")] {
            for body in [
                "|a,| a",
                "a",
                "||",
                "|a,b| a+b",
                "|a,b,c=nil| a+b+c",
                "|a,b,c:1| a+b+c",
                "|foo,bar=','| foo+bar",
                "|foo,bar,;baz| foo+bar",
            ] {
                let source = format!("test {open} {body}\n{close}");
                assert_eq!(
                    ruby_prism::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in [
            "-> (foo, bar) { do_something(foo, bar) }",
            "lambda { |foo,bar| do_something(foo,bar) }",
            "test { [1,] }",
            "test { |a,*rest| a }",
            "test { |a:,b:| a }",
            "test { |(a,*rest),| a }",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn destructuring_counts_leaf_arguments_but_is_never_rewritten() {
        for source in [
            "test { |(a,b),| a }",
            "test { |((a,b),c),| a }",
            "test { |a,b, # comma\n| a }",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }
}
