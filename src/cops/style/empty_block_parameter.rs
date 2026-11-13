//! `Style/EmptyBlockParameter` — omit explicit empty block pipes.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EmptyBlockParameter;
impl Cop for EmptyBlockParameter {
    fn name(&self) -> &'static str {
        "Style/EmptyBlockParameter"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_block_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(block) = node.as_block_node() else {
            return;
        };
        let Some(parameters) = super::empty_parameters::empty(block.parameters()) else {
            return;
        };
        super::empty_parameters::report(
            self,
            &parameters,
            block.opening_loc().end_offset(),
            cx,
            file,
            out,
            "Omit pipes for the empty block parameters.",
        );
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/empty_block_parameter_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_offenses() {
        for (source, line, col) in [
            ("a do ||\nend\n", 1, 6),
            ("a { || do_something }\n", 1, 5),
            ("def foo\n  super { || do_something }\nend\n", 2, 11),
            ("lambda { || do_something }\n", 1, 10),
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == EmptyBlockParameter.name())
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(
                found[0].message,
                "Omit pipes for the empty block parameters."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_no_offenses_and_parameter_lookalikes() {
        for source in [
            "a do\nend\n",
            "a { }",
            "a do |x|\nend",
            "a { |x| }",
            "-> () { do_something }",
            "a { |;x| x }",
            "a { |*| }",
            "a { |**| }",
            "a { |&| }",
            "a { |x = 1| x }",
            "a { |(x,y)| x }",
            "a { |x:| x }",
            "a { _1 }",
            "a { it }",
            "# a { || }\ntext = 'a { || }'",
            "-> { 1 }",
            "->(x) { x }",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == EmptyBlockParameter.name()),
                "{source}"
            );
        }
    }
    #[test]
    fn pinned_corrections() {
        for (before, after) in [
            ("a do ||\nend\n", "a do\nend\n"),
            ("a { || do_something }\n", "a { do_something }\n"),
            (
                "def foo\n  super { || do_something }\nend\n",
                "def foo\n  super { do_something }\nend\n",
            ),
            ("lambda { || do_something }\n", "lambda { do_something }\n"),
        ] {
            assert_correction(&EmptyBlockParameter, before, after);
        }
    }
}
