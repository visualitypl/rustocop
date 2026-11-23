//! `Style/EmptyClassDefinition` — fixed `class_keyword`, no allowed parents.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct EmptyClassDefinition;
impl Cop for EmptyClassDefinition {
    fn name(&self) -> &'static str {
        "Style/EmptyClassDefinition"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(
            node,
            Node::ConstantWriteNode { .. } | Node::ConstantPathWriteNode { .. }
        )
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(_) = candidate(node) else { return };
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use the `class` keyword instead of `Class.new` to define an empty class."
                .into(),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, file))
                .flatten(),
        });
    }
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<(String, CallNode<'pr>, Node<'pr>)> {
    let (name, value) = if let Some(w) = node.as_constant_write_node() {
        (
            String::from_utf8_lossy(w.name().as_slice()).into_owned(),
            w.value(),
        )
    } else {
        let w = node.as_constant_path_write_node()?;
        (
            String::from_utf8_lossy(w.target().location().as_slice()).into_owned(),
            w.value(),
        )
    };
    let call = value.as_call_node()?;
    let args = ast::call_arguments(&call);
    if call.is_safe_navigation()
        || call.block().is_some()
        || call.name().as_slice() != b"new"
        || ast::short_const_name(&call.receiver()?).as_deref() != Some("Class")
        || !matches!(args.as_slice(), [parent] if ast::is_const(parent))
    {
        return None;
    }
    Some((name, call, args.into_iter().next()?))
}
/// RuboCop's `autocorrect_class_new`.
fn correction(node: &Node<'_>, file: &SourceFile) -> Option<Correction> {
    let loc = node.location();
    let (name, _, parent) = candidate(node)?;
    let line_start = file.line_start(file.line_of(loc.start_offset()));
    let indent = " ".repeat(
        std::str::from_utf8(&file.bytes[line_start..loc.start_offset()])
            .ok()?
            .chars()
            .count(),
    );
    let parent = std::str::from_utf8(parent.location().as_slice()).ok()?;
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!("class {name} < {parent}\n{indent}end"),
    })
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop empty_class_definition_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed class_keyword.
    use super::*;
    use crate::cops::testing::assert_correction;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/EmptyClassDefinition")
            .collect()
    }
    #[test]
    fn pinned_positions_and_canonical_fresh_error_definitions() {
        for (s, line, col, after) in [
            (
                "FooError = Class.new(StandardError)\n",
                1,
                1,
                "class FooError < StandardError\nend\n",
            ),
            (
                "module Foo\n  BarError = Class.new(StandardError)\nend\n",
                2,
                3,
                "module Foo\n  class BarError < StandardError\n  end\nend\n",
            ),
            (
                "module Foo; end\nFoo::BarError = Class.new(::StandardError)\n",
                2,
                1,
                "module Foo; end\nclass Foo::BarError < ::StandardError\nend\n",
            ),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (line, col, line));
            assert_eq!(
                o[0].message,
                "Use the `class` keyword instead of `Class.new` to define an empty class."
            );
            assert_correction(&EmptyClassDefinition, s, after);
        }
    }
    #[test]
    fn exact_assignment_receiver_parent_and_block_shapes() {
        for s in [
            "ÉError = Class.new(StandardError)",
            "Foo::BarError = Class.new(StandardError)",
            "MyClass = Class.new(Alchemy::Admin::PreviewUrl)",
            "MyClass = Class.new(::Safemode::Jail)",
            "Foo = ::Class.new(StandardError)",
            "Foo = Other::Class.new(StandardError)",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "MyClass = Class.new",
            "MyClass = Class.new()",
            "class FooError < StandardError; end",
            "class MyClass; end",
            "class FooError < StandardError; def initialize; end; end",
            "MyClass = Class.new(StandardError) { }",
            "MyClass = Class.new(StandardError) do end",
            "MyClass = Class.new(StandardError) { _1 }",
            "MyClass = Class.new(StandardError) { it }",
            "local_var = Class.new(Base)",
            "@instance_var = Class.new(Base)",
            "@@class_var = Class.new(Base)",
            "$global_var = Class.new(Base)",
            "local_var ||= Class.new(Base)",
            "CONST ||= Class.new(Base)",
            "raise Class.new(StandardError)",
            "foo(Class.new(Base))",
            "return Class.new(Error)",
            "MyClass = Class.new(self)",
            "MyClass = Class.new(nil)",
            "MyClass = Class.new(false)",
            "MyClass = Class.new(local_var)",
            "MyClass = Class.new(StandardError).any_method",
            "CONST = condition",
            "BOOLEAN = ActiveRecord::Type::Boolean.new",
            "Foo = Class&.new(StandardError)",
            "Foo = Class.new(StandardError, Object)",
            "Foo = Class.new(*parents)",
            "Foo = Class.new(StandardError, &block)",
            "let(:foo) { Class.new(StandardError) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
