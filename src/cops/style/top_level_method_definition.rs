//! `Style/TopLevelMethodDefinition` — root definitions only, not eval guesses.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct TopLevelMethodDefinition;
impl Cop for TopLevelMethodDefinition {
    fn name(&self) -> &'static str {
        "Style/TopLevelMethodDefinition"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let definition = node.as_def_node().is_some()
            || node.as_call_node().is_some_and(|call| {
                call.name().as_slice() == b"define_method"
                    && call
                        .call_operator_loc()
                        .is_none_or(|l| l.as_slice() != b"&.")
                    && (call.block().is_none_or(|b| b.as_block_node().is_none())
                        || call.arguments().is_some_and(|a| a.arguments().len() == 1))
            });
        if !definition {
            return;
        }
        // StatementsNode is transparent except for a multi-statement begin.
        // A single root begin may hold the definition; deeper wrappers, classes,
        // blocks, visibility sends and conditionals are deliberately not roots.
        let mut begins = 0;
        let mut ancestors = cx.ancestors().peekable();
        while let Some(ancestor) = ancestors.next() {
            if let Some(s) = ancestor.as_statements_node() {
                begins += usize::from(
                    s.body().len() > 1
                        && !ancestors
                            .peek()
                            .is_some_and(|n| n.as_parentheses_node().is_some()),
                );
            } else if ancestor.as_parentheses_node().is_some() {
                begins += 1;
            } else if ancestor.as_program_node().is_none() {
                return;
            }
            if begins > 1 {
                return;
            }
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not define methods at the top-level.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/top_level_method_definition_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{offense::Offense, pipeline::lint_str_with};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&TopLevelMethodDefinition], source)
    }
    #[test]
    fn pinned_root_definitions_and_dynamic_calls() {
        for source in [
            "def foo; end",
            "def self.foo; end",
            "def f = 1",
            "define_method(:foo) { puts 1 }",
            "define_method(:foo) { puts _1 }",
            "define_method(:foo) { puts it }",
            "define_method(:foo) do |x|\n puts 1\nend",
            "define_method(:foo, instance_method(:bar))",
            "obj.define_method(:f) {}",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].message, "Do not define methods at the top-level.");
            assert!(found[0].correction.is_none());
        }
        let found = offenses("class Foo\nend\n\ndef foo; end\n");
        assert_eq!((found[0].line, found[0].col), (4, 1));
    }
    #[test]
    fn root_parenthesized_statement_lists_are_one_begin_not_two() {
        for (source, col) in [
            ("(x=1;def f;end)", 6),
            ("(def f;end; x=1)", 2),
            ("(x=1;define_method(:f) {})", 6),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
        }
        for source in ["x=1; (def f;end)", "((def f;end))", "(x=1;(def f;end))"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_enclosing_scopes_and_near_misses() {
        for source in [
            "module Foo\n def foo; end\nend",
            "class Foo\n def self.foo; end\nend",
            "Foo = Struct.new do\n def foo; end\nend",
            "Foo = types.each do |type|\n def foo(type)\n puts type\n end\nend",
            "class Foo\n define_method(:a) { puts 1 }\n define_method(:b) do |x|\n puts x\n end\n define_method(:c, instance_method(:d))\nend",
            "require_relative 'foo'",
            "private def foo; end",
            "if ready\n def foo; end\nend",
            "begin\n def foo; end\nend",
            "class << self\n def foo; end\nend",
            "-> { def foo; end }",
            "x = def foo; end",
            "obj&.define_method(:f) {}",
            "define_method(:f, proc {}) {}",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
