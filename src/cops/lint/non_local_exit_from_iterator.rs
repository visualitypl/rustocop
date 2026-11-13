//! `Lint/NonLocalExitFromIterator` — flags a bare `return` inside an
//! iterator block, which leaves the enclosing method rather than the block.
//!
//! Matches RuboCop's semantics. A `return` without a value is reported when,
//! walking out from it, the first block that is not transparent is an
//! iterator: a block with parameters (explicit, numbered or `it`) whose
//! method has a receiver (`items.each do |item|`). On the way out:
//!
//! - a method definition or a lambda (`-> {}`, `lambda {}`) ends the search:
//!   `return` leaves that scope;
//! - so does a `define_method(name)`/`define_singleton_method(name)` block,
//!   where `return` is fine;
//! - blocks without parameters, or whose method has no receiver
//!   (`transaction do`, `find_each do |item|`), are walked through.
//!
//! As in RuboCop's AST, a block encloses its method call's receiver and
//! arguments, not just its body.

use ruby_prism::{BlockNode, CallNode, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/NonLocalExitFromIterator` cop.
pub struct NonLocalExitFromIterator;

const MESSAGE: &str = "Non-local exit from iterator, without return value. \
                       `next`, `break`, `Array#find`, `Array#any?`, etc. is preferred.";

impl Cop for NonLocalExitFromIterator {
    fn name(&self) -> &'static str {
        "Lint/NonLocalExitFromIterator"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_return_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(ret) = node.as_return_node() else {
            return;
        };
        if ret.arguments().is_some() {
            return;
        }
        // Use the pipeline's complete ancestor stack. A Prism call with a
        // literal block encloses its receiver/arguments as well as its body,
        // just as the synthetic block parent does in the reference AST.
        for ancestor in cx.ancestors() {
            if matches!(ancestor, Node::DefNode { .. } | Node::LambdaNode { .. }) {
                return;
            }
            let Some(call) = ancestor.as_call_node() else {
                continue;
            };
            match classify(&call) {
                BlockKind::Scope => return,
                BlockKind::Iterator => {
                    let (line, col) = super::array_literal_in_regexp::reference_line_col(
                        ret.keyword_loc().start_offset(),
                        file,
                    );
                    out.push(Offense {
                        correction: None,
                        cop: self.name(),
                        severity: self.severity(),
                        message: MESSAGE.to_string(),
                        line,
                        end_line: line,
                        col,
                    });
                    return;
                }
                BlockKind::NoBlock | BlockKind::Transparent => {}
            }
        }
    }
}

/// How a call with a block affects a `return` inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    /// No literal block: not a scope.
    NoBlock,
    /// A lambda or `define_method` block: `return` stays inside.
    Scope,
    /// An iterator: a block with parameters on a call with a receiver.
    Iterator,
    /// Any other block: walked through.
    Transparent,
}

fn classify(call: &CallNode<'_>) -> BlockKind {
    let Some(block) = call.block().and_then(|block| block.as_block_node()) else {
        return BlockKind::NoBlock;
    };
    let name = call.name().as_slice();
    // RuboCop's `BlockNode#lambda?` ignores the receiver.
    if name == b"lambda" || is_define_method(call) {
        BlockKind::Scope
    } else if call.receiver().is_some() && has_parameters(&block) {
        BlockKind::Iterator
    } else {
        BlockKind::Transparent
    }
}

/// `define_method(name)` or `define_singleton_method(name)`: a plain call
/// (not `&.`) with exactly one argument, as RuboCop's pattern.
fn is_define_method(call: &CallNode<'_>) -> bool {
    matches!(
        call.name().as_slice(),
        b"define_method" | b"define_singleton_method"
    ) && !call.is_safe_navigation()
        && call
            .arguments()
            .is_some_and(|arguments| arguments.arguments().iter().count() == 1)
}

/// Whether the block declares parameters: explicit ones (block-locals
/// included), numbered ones or `it`.
fn has_parameters(block: &BlockNode<'_>) -> bool {
    let Some(parameters) = block.parameters() else {
        return false;
    };
    match parameters.as_block_parameters_node() {
        Some(explicit) => {
            explicit.parameters().is_some() || explicit.locals().iter().next().is_some()
        }
        // `NumberedParametersNode` or `ItParametersNode`.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/non_local_exit_from_iterator_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NonLocalExitFromIterator")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_return_in_chained_block_with_argument() {
        let found = offenses(
            "items.each do |item|\n  return if item.stock == 0\n  item.update!(foobar: true)\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 3));
        assert_eq!(
            found[0].message,
            "Non-local exit from iterator, without return value. `next`, `break`, \
             `Array#find`, `Array#any?`, etc. is preferred."
        );
    }

    #[test]
    fn flags_safe_navigation_numbered_and_it_blocks() {
        assert_eq!(
            positions("items&.each do |item|\n  return if item.stock == 0\nend\n"),
            vec![(2, 3)]
        );
        assert_eq!(
            positions("items.each do\n  return if baz?(_1)\n  _1.update!(foobar: true)\nend\n"),
            vec![(2, 3)]
        );
        assert_eq!(
            positions("items.each do\n  return if baz?(it)\n  it.update!(foobar: true)\nend\n"),
            vec![(2, 3)]
        );
    }

    #[test]
    fn flags_return_in_block_with_multiple_arguments() {
        assert_eq!(
            positions(
                "items.each_with_index do |item, i|\n  return if item.stock == 0\n  item.update!(foobar: true)\nend\n"
            ),
            vec![(2, 3)]
        );
    }

    #[test]
    fn accepts_block_without_arguments() {
        assert!(
            offenses(
                "item.with_lock do\n  return if item.stock == 0\n  item.update!(foobar: true)\nend\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn accepts_block_without_method_chain() {
        let source = "\
transaction do
  return unless update_necessary?
  find_each do |item|
    return if item.stock == 0 # false-negative...
    item.update!(foobar: true)
  end
end
";
        assert!(offenses(source).is_empty());
    }

    #[test]
    fn accepts_lambdas() {
        let source = "\
items.each(lambda do |item|
  return if item.stock == 0
  item.update!(foobar: true)
end)
items.each -> (item) {
  return if item.stock == 0
  item.update!(foobar: true)
}
";
        assert!(offenses(source).is_empty());
    }

    #[test]
    fn accepts_lambda_inside_a_chained_block() {
        let source = r#"
RSpec.configure do |config|
  # some configuration

  if Gem.loaded_specs["paper_trail"].version < Gem::Version.new("4.0.0")
    current_behavior = ActiveSupport::Deprecation.behavior
    ActiveSupport::Deprecation.behavior = lambda do |message, callstack|
      return if message =~ /foobar/
      Array.wrap(current_behavior).each do |behavior|
        behavior.call(message, callstack)
      end
    end

    # more configuration
  end
end
"#;
        assert!(offenses(source).is_empty());
    }

    #[test]
    fn flags_each_return_against_its_nearest_iterator() {
        let source = "\
transaction do
  return unless update_necessary?
  items.each do |item|
    return if item.nil?
    item.with_lock do
      return if item.stock == 0
      item.very_complicated_update_operation!
    end
  end
end
";
        assert_eq!(positions(source), vec![(4, 5), (6, 7)]);
    }

    #[test]
    fn accepts_return_with_value() {
        let source = "\
def find_first_sold_out_item(items)
  items.each do |item|
    return item if item.stock == 0
    item.foobar!
  end
end
";
        assert!(offenses(source).is_empty());
    }

    #[test]
    fn accepts_return_in_define_method_and_define_singleton_method() {
        let source = "\
[:method_one, :method_two].each do |method_name|
  define_method(method_name) do
    return if predicate?
  end
end
";
        assert!(offenses(source).is_empty());
        let source = "\
str = 'foo'
str.define_singleton_method :bar do |baz|
  return unless baz
  replace baz
end
";
        assert!(offenses(source).is_empty());
    }

    #[test]
    fn accepts_return_in_nested_method_definitions() {
        assert!(
            offenses("Foo.configure do |c|\n  def bar\n    return if baz?\n  end\nend\n")
                .is_empty()
        );
        assert!(
            offenses("Foo.configure do |c|\n  def self.bar\n    return if baz?\n  end\nend\n")
                .is_empty()
        );
    }

    #[test]
    fn walks_through_transparent_blocks_to_the_iterator() {
        assert_eq!(
            positions("items.each do |item|\n  transaction do\n    return\n  end\nend\n"),
            vec![(3, 5)]
        );
        assert_eq!(
            positions("items.each do |item|\n  super do |x|\n    return\n  end\nend\n"),
            vec![(3, 5)]
        );
        // Block-local variables count as parameters.
        assert_eq!(
            positions("items.each do |;x|\n  return\nend\n"),
            vec![(2, 3)]
        );
        // A safe-navigation `define_method` is not RuboCop's `define_method`.
        assert_eq!(
            positions("obj&.define_method(:x) do |a|\n  return\nend\n"),
            vec![(2, 3)]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "items.each do |item|\n  next if item\nend\n",
            "items.each do |item|\n  break\nend\n",
            "items.each { || return }\n",
            "each do |item|\n  return\nend\n",
            "def m\n  return\nend\n",
            "items.each do |item|\n  -> { return }\nend\n",
            "items.each(&:foo)\nreturn\n",
            "foo.lambda do |x|\n  return\nend\n",
            "self.define_method(:x) do |a|\n  return\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
