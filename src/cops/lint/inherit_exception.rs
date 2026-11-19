//! `Lint/InheritException` — flags classes that inherit from `Exception`
//! (`class C < Exception`, `Class.new(Exception)`): a bare `rescue` only
//! handles `StandardError` and its subclasses.
//!
//! Matches RuboCop's semantics with its default `EnforcedStyle:
//! standard_error`, fixed in rustocop, and without its optional project
//! index (indirect inheritance is not detected). A bare `Exception`
//! superclass is accepted when an earlier statement of the same body defines
//! a class or module named `Exception` (the superclass then resolves to that
//! one); `::Exception` never is. `Class.new` takes the constant as its only
//! argument. The offense is reported at the constant.

use ruby_prism::{ClassNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast::const_name},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/InheritException` cop.
pub struct InheritException;

impl Cop for InheritException {
    fn name(&self) -> &'static str {
        "Lint/InheritException"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let found = match node {
            Node::ClassNode { .. } => superclass_exception(&node.as_class_node().unwrap(), cx),
            Node::CallNode { .. } => class_new_exception(&node.as_call_node().unwrap()),
            _ => None,
        };
        let Some(location) = found else {
            return;
        };
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(location.start_offset(), file);
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: location.start_offset()..location.end_offset(),
                    replacement: "StandardError".into(),
                }),
            cop: self.name(),
            severity: self.severity(),
            message: "Inherit from `StandardError` instead of `Exception`.".to_string(),
            line,
            end_line: file.last_line_of(location),
            col,
        });
    }
}

/// For `Class.new(Exception)` or `Class.new(::Exception)` (with `Class` or
/// `::Class`), the location of the `Exception` constant.
fn class_new_exception<'pr>(call: &ruby_prism::CallNode<'pr>) -> Option<ruby_prism::Location<'pr>> {
    if call.name().as_slice() != b"new" || call.is_safe_navigation() {
        return None;
    }
    let receiver = call.receiver()?;
    if top_level_const_name(&receiver) != Some(b"Class") {
        return None;
    }
    let arguments = call.arguments()?.arguments();
    let mut arguments = arguments.iter();
    let argument = arguments.next()?;
    if arguments.next().is_some()
        || call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some())
    {
        return None;
    }
    (top_level_const_name(&argument) == Some(b"Exception")).then(|| argument.location())
}

/// The name of `Name` or `::Name`: a constant with no namespace, or the
/// top-level one.
fn top_level_const_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    match node {
        Node::ConstantReadNode { .. } => {
            Some(node.as_constant_read_node().unwrap().name().as_slice())
        }
        Node::ConstantPathNode { .. } => {
            let path = node.as_constant_path_node().unwrap();
            if path.parent().is_some() {
                return None;
            }
            path.name().map(|name| name.as_slice())
        }
        _ => None,
    }
}

/// For `class C < Exception` or `class C < ::Exception`, the location of the
/// superclass; `None` when an earlier statement of the same body defines a
/// local `Exception` that a bare `Exception` resolves to.
fn superclass_exception<'pr>(
    class: &ClassNode<'pr>,
    cx: &NodeContext<'_, '_>,
) -> Option<ruby_prism::Location<'pr>> {
    let superclass = class.superclass()?;
    if const_name(&superclass).as_deref() != Some("Exception") {
        return None;
    }
    let written_top_level = superclass.as_constant_path_node().is_some();
    if !written_top_level && exception_defined_before(class, cx) {
        return None;
    }
    Some(superclass.location())
}

/// Whether a statement before `class` in its statement list defines a local
/// `Exception`. Classes outside a statement list (e.g. a call argument) have
/// no sibling definitions.
fn exception_defined_before(class: &ClassNode<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let Some(statements) = cx.parent().and_then(Node::as_statements_node) else {
        return false;
    };
    let start = class.location().start_offset();
    statements
        .body()
        .iter()
        .take_while(|statement| statement.location().start_offset() != start)
        .any(|statement| defines_exception(&statement))
}

/// Whether `node` is a class, module or singleton class whose name (or
/// subject) is the constant `Exception` (RuboCop's `identifier`).
fn defines_exception(node: &Node<'_>) -> bool {
    let identifier = match node {
        Node::ClassNode { .. } => node.as_class_node().unwrap().constant_path(),
        Node::ModuleNode { .. } => node.as_module_node().unwrap().constant_path(),
        Node::SingletonClassNode { .. } => node.as_singleton_class_node().unwrap().expression(),
        _ => return false,
    };
    const_name(&identifier).as_deref() == Some("Exception")
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/inherit_exception_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the
    //! `standard_error` style and without a project index.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/InheritException")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_class_inheriting_exception_with_message() {
        let found = offenses("class C < Exception; end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 11));
        assert_eq!(
            found[0].message,
            "Inherit from `StandardError` instead of `Exception`."
        );
    }

    #[test]
    fn flags_class_new_with_exception() {
        assert_eq!(positions("Class.new(Exception)\n"), vec![(1, 11)]);
    }

    #[test]
    fn flags_exception_defined_only_after_the_class() {
        let source = "\
module Foo
  class C < Exception; end # This `Exception` is the same as `::Exception`.
  class Exception < RuntimeError; end
end
";
        assert_eq!(positions(source), vec![(2, 13)]);
        let source = "\
module Foo
  include Bar

  class C < Exception; end # This `Exception` is the same as `::Exception`.
  class Exception < RuntimeError; end
end
";
        assert_eq!(positions(source), vec![(4, 13)]);
    }

    #[test]
    fn flags_top_level_exception_despite_local_definition() {
        let source = "\
module Foo
  class Exception < RuntimeError; end
  class C < ::Exception; end
end
";
        assert_eq!(positions(source), vec![(3, 13)]);
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "class C < Interrupt; end\n",
            "module Foo\n  class Exception < StandardError; end # Foo::Exception\n  class C < Exception; end\nend\n",
            "module Foo\n  class Exception < RuntimeError; end # Foo::Exception\n  class C < Exception; end\nend\n",
            // Indirect inheritance needs RuboCop's project index.
            "class MyError < BaseError\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn local_definitions_count_from_any_earlier_sibling() {
        // Modules and singleton classes named `Exception` count too, as do
        // earlier siblings that are not adjacent.
        assert!(offenses("module Exception; end\nfoo\nclass C < Exception; end\n").is_empty());
        assert!(offenses("class << Exception; end\nclass C < Exception; end\n").is_empty());
        assert!(offenses("class ::Exception; end\nclass C < Exception; end\n").is_empty());
        // Siblings only: not a definition in another body, nor a namespaced one.
        assert_eq!(
            positions("module A\n  class Exception; end\nend\nclass C < Exception; end\n"),
            vec![(4, 11)]
        );
        assert_eq!(
            positions("class Foo::Exception; end\nclass C < Exception; end\n"),
            vec![(2, 11)]
        );
        // The only statement of a body has no earlier sibling.
        assert_eq!(
            positions("module Foo\n  class C < Exception; end\nend\n"),
            vec![(2, 13)]
        );
    }

    #[test]
    fn flags_other_spellings() {
        assert_eq!(positions("class C < ::Exception\nend\n"), vec![(1, 11)]);
        assert_eq!(positions("::Class.new(::Exception)\n"), vec![(1, 13)]);
        assert_eq!(
            positions("Error = Class.new(Exception) do\n  def x; end\nend\n"),
            vec![(1, 19)]
        );
        assert_eq!(positions("foo(class C < Exception; end)\n"), vec![(1, 15)]);
        assert_eq!(
            positions("module M\n  class A\n    class B < Exception; end\n  end\nend\n"),
            vec![(3, 15)]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "class C < StandardError; end\n",
            "class C < Foo::Exception; end\n",
            "class C < MyException; end\n",
            "class Exception; end\n",
            "class C; end\n",
            "Class.new(StandardError)\n",
            "Class.new(Exception, &blk)\n",
            "Class.new(Foo::Exception)\n",
            "Class&.new(Exception)\n",
            "Struct.new(Exception)\n",
            "Foo::Class.new(Exception)\n",
            "Class.new(Exception, Bar)\n",
            "raise Exception\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
