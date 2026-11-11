//! `Lint/RaiseException` — flags `raise Exception` / `fail Exception` (and
//! `raise Exception.new(...)`): raise `StandardError` or a specific class
//! instead, so a bare `rescue` can handle it.
//!
//! Matches RuboCop's semantics with its default
//! `AllowedImplicitNamespaces: ['Gem']`, fixed in rustocop: a bare
//! `Exception` (no leading `::`) inside a `module Gem` body, at any depth,
//! refers to `Gem::Exception` and is accepted. The receiverless `raise` or
//! `fail` takes `Exception`/`::Exception` as its first argument, or as the
//! receiver of `.new` when that is its only argument. The offense is
//! reported at the constant.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/RaiseException` cop.
pub struct RaiseException;

/// Modules whose own `Exception` a bare `Exception` may refer to.
const ALLOWED_IMPLICIT_NAMESPACES: [&[u8]; 1] = [b"Gem"];

impl Cop for RaiseException {
    fn name(&self) -> &'static str {
        "Lint/RaiseException"
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
        let Some((location, top_level)) = raised_exception(&call) else {
            return;
        };
        if !top_level && cx.ancestors().any(is_allowed_namespace) {
            return;
        }
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(location.start_offset(), file);
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Use `StandardError` over `Exception`.".to_string(),
            line,
            end_line: file.last_line_of(location),
            col,
        });
    }
}

/// Whether `node` is a `module` whose own `Exception` a bare `Exception`
/// inside it may refer to.
fn is_allowed_namespace(node: &Node<'_>) -> bool {
    node.as_module_node().is_some_and(|module| {
        ALLOWED_IMPLICIT_NAMESPACES.contains(&module.constant_path().location().as_slice())
    })
}

/// For `raise Exception, ...` or `raise Exception.new(...)`, the location of
/// the `Exception` constant and whether it is written `::Exception`.
fn raised_exception<'pr>(call: &CallNode<'pr>) -> Option<(ruby_prism::Location<'pr>, bool)> {
    let name = call.name().as_slice();
    if call.receiver().is_some() || !(name == b"raise" || name == b"fail") {
        return None;
    }
    let arguments: Vec<Node<'_>> = call.arguments()?.arguments().iter().collect();
    let first = arguments.first()?;
    if let Some(top_level) = exception_const(first) {
        return Some((first.location(), top_level));
    }
    // `raise Exception.new(...)`, as the only argument.
    let new = first.as_call_node()?;
    let is_plain_new = arguments.len() == 1
        && !call
            .block()
            .is_some_and(|block| block.as_block_argument_node().is_some())
        && new.name().as_slice() == b"new"
        && !new.is_safe_navigation()
        && !new
            .block()
            .is_some_and(|block| block.as_block_node().is_some());
    if !is_plain_new {
        return None;
    }
    let receiver = new.receiver()?;
    let top_level = exception_const(&receiver)?;
    Some((receiver.location(), top_level))
}

/// Whether `node` is `Exception` (`Some(false)`) or `::Exception`
/// (`Some(true)`).
fn exception_const(node: &Node<'_>) -> Option<bool> {
    match node {
        Node::ConstantReadNode { .. } => (node.as_constant_read_node().unwrap().name().as_slice()
            == b"Exception")
            .then_some(false),
        Node::ConstantPathNode { .. } => {
            let path = node.as_constant_path_node().unwrap();
            (path.parent().is_none()
                && path
                    .name()
                    .is_some_and(|name| name.as_slice() == b"Exception"))
            .then_some(true)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/raise_exception_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RaiseException")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn constructor_match_excludes_outer_block_pass_but_direct_constant_allows_it() {
        assert!(offenses("raise Exception.new, &block").is_empty());
        assert!(offenses("fail ::Exception.new, &block").is_empty());
        assert_eq!(positions("raise Exception, &block"), [(1, 7)]);
        assert_eq!(positions("raise Exception.new(&block)"), [(1, 7)]);
    }

    #[test]
    fn flags_raise_exception_with_message() {
        let found = offenses("raise Exception\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 7));
        assert_eq!(found[0].message, "Use `StandardError` over `Exception`.");
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, col) in [
            ("raise ::Exception\n", 7),
            ("raise ::Exception.new 'Error with exception'\n", 7),
            ("raise ::Exception, 'Error with exception'\n", 7),
            ("raise Exception, 'Error with exception'\n", 7),
            ("raise Exception.new 'Error with exception'\n", 7),
            ("raise Exception.new('arg1', 'arg2')\n", 7),
            ("fail Exception\n", 6),
            ("fail Exception, 'Error with exception'\n", 6),
            ("fail Exception.new 'Error with exception'\n", 6),
        ] {
            assert_eq!(positions(source), vec![(1, col)], "should flag {source:?}");
        }
    }

    #[test]
    fn flags_inside_namespaces_that_are_not_allowed() {
        let source = "\
module Foo
  module Bar
    def self.foo
      raise Exception
    end
  end
end
";
        assert_eq!(positions(source), vec![(4, 13)]);
    }

    #[test]
    fn flags_top_level_exception_inside_allowed_namespace() {
        let source = "module Gem\n  def self.foo\n    raise ::Exception\n  end\nend\n";
        assert_eq!(positions(source), vec![(3, 11)]);
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "raise\n",
            "fail\n",
            "raise Foo::Exception\n",
            "module Gem\n  def self.foo\n    raise Exception\n  end\nend\n",
            "module Gem\n  module Inner\n    def self.foo\n      raise Exception\n    end\n  end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_parenthesized_and_nested_raises() {
        assert_eq!(positions("raise(Exception, 'x')\n"), vec![(1, 7)]);
        assert_eq!(positions("foo { fail(Exception.new) }\n"), vec![(1, 12)]);
        // Leaving the allowed namespace re-enables the check.
        assert_eq!(
            positions("module Gem\nend\nraise Exception\n"),
            vec![(3, 7)]
        );
        // Only a `module` named exactly `Gem` is an allowed namespace.
        assert_eq!(
            positions("class Gem\n  raise Exception\nend\n"),
            vec![(2, 9)]
        );
        assert_eq!(
            positions("module Foo::Gem\n  raise Exception\nend\n"),
            vec![(2, 9)]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "raise StandardError\n",
            "raise RuntimeError, 'Exception'\n",
            "raise MyException\n",
            "Kernel.raise Exception\n",
            "foo.fail Exception\n",
            "raise 'msg', Exception\n",
            "raise Exception.new('a'), 'b'\n",
            "raise Exception&.new\n",
            "raise Exception.new { }\n",
            "raise Exception.build\n",
            "raise self::Exception\n",
            "rescue_from Exception\n",
            "Exception\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
