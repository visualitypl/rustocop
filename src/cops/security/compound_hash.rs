//! `Security/CompoundHash` — flags `hash` implementations that combine
//! values by hand (`@a.hash ^ @b.hash`) instead of delegating to
//! `[@a, @b].hash`, and two misuses of `Array#hash`.
//!
//! Matches RuboCop's semantics. Three offenses:
//!
//! - **Combinator**: a binary `^`, `+`, `*` or `|` (a plain call with one
//!   argument, or an operator assignment such as `h ^= x`) inside a hash
//!   method, unless another combinator encloses it. A hash method is
//!   `def hash`/`def recv.hash` without parameters, or a
//!   `define_method(:hash)`/`define_singleton_method(:hash)` call with a
//!   parameterless `{ }`/`do` block (not `_1` or `it`). Any enclosing hash
//!   method counts, through blocks, lambdas and nested `def`s, as does any
//!   enclosing combinator, even one outside the hash method. Reported at
//!   the combinator.
//! - **Monuple**: `[x].hash`, anywhere. Reported at the call.
//! - **Redundant**: `[a.hash, b].hash`, anywhere: a direct element of the
//!   array that is a `hash` call (`&.` too) without arguments or a block.
//!   Reported at the element.
//!
//! As in RuboCop's AST, a block argument (`&blk`) counts as an argument, and
//! the block of a call encloses the call's receiver and arguments, not the
//! other way round: in `a.+(b) { x ^ y }` the block body is not inside the
//! `+` combinator.

use ruby_prism::{CallNode, DefNode, Node, Visit};

use crate::{
    cops::{Cop, NodeContext, ast::call_arguments},
    offense::Offense,
    source::SourceFile,
};

/// The `Security/CompoundHash` cop.
pub struct CompoundHash;

const COMBINATOR_MESSAGE: &str = "Use `[...].hash` instead of combining hash values manually.";
const MONUPLE_MESSAGE: &str =
    "Delegate hash directly without wrapping in an array when only using a single value.";
const REDUNDANT_MESSAGE: &str = "Calling .hash on elements of a hashed array is redundant.";

impl Cop for CompoundHash {
    fn name(&self) -> &'static str {
        "Security/CompoundHash"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Combinators offend depending on all of their ancestors, so one walk
        // from the root tracks them.
        let Some(program) = node.as_program_node() else {
            return;
        };
        // Every offense needs a `hash` method name or `:hash` symbol
        // spelled out (an escaped `:"h\x61sh"` is not recognized).
        if !file.contains(b"hash") {
            return;
        }
        let mut finder = Finder {
            hash_methods: 0,
            combinators: 0,
            offenses: Vec::new(),
        };
        finder.visit_program_node(&program);
        for (offset, end, message) in finder.offenses {
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: message.to_string(),
                line,
                end_line: file.line_of(end.saturating_sub(1).max(offset)),
                col,
            });
        }
    }
}

/// Finds the offenses, tracking enclosing hash methods and combinators.
struct Finder {
    /// How many hash method definitions enclose the current node.
    hash_methods: usize,
    /// How many combinators enclose the current node.
    combinators: usize,
    /// Source spans and messages of the offenses.
    offenses: Vec<(usize, usize, &'static str)>,
}

impl Finder {
    /// Visit a combinator's own children: report it if it is the outermost
    /// one inside a hash method, and count it as enclosing them.
    fn combinator(&mut self, start: usize, end: usize, visit_children: impl FnOnce(&mut Self)) {
        if self.combinators == 0 && self.hash_methods > 0 {
            self.offenses.push((start, end, COMBINATOR_MESSAGE));
        }
        self.combinators += 1;
        visit_children(self);
        self.combinators -= 1;
    }

    /// The `[x].hash` and `[a.hash, b].hash` checks.
    fn check_array_hash(&mut self, call: &CallNode<'_>) {
        if call.name().as_slice() != b"hash"
            || call.is_safe_navigation()
            || !call_arguments(call).is_empty()
        {
            return;
        }
        let Some(array) = call
            .receiver()
            .and_then(|receiver| receiver.as_array_node())
        else {
            return;
        };
        let elements = array.elements();
        if elements.iter().count() == 1 {
            self.offenses.push((
                call.location().start_offset(),
                crate::cops::ast::source_end(&call.as_node()),
                MONUPLE_MESSAGE,
            ));
        }
        for element in elements.iter() {
            let redundant = element.as_call_node().is_some_and(|inner| {
                inner.name().as_slice() == b"hash"
                    && inner.arguments().is_none()
                    && inner.block().is_none()
            });
            if redundant {
                self.offenses.push((
                    element.location().start_offset(),
                    crate::cops::ast::source_end(&element),
                    REDUNDANT_MESSAGE,
                ));
            }
        }
    }
}

/// The operators that combine hash values.
fn is_combinator_operator(name: &[u8]) -> bool {
    matches!(name, b"^" | b"+" | b"*" | b"|")
}

/// `recv ^ arg`: a plain call (not `&.`) to a combinator operator with
/// exactly one argument.
fn is_combinator_call(call: &CallNode<'_>) -> bool {
    is_combinator_operator(call.name().as_slice())
        && !call.is_safe_navigation()
        && call_arguments(call).len() == 1
}

/// `def hash` or `def recv.hash`, without parameters.
fn is_static_hash_method(def: &DefNode<'_>) -> bool {
    def.name().as_slice() == b"hash" && def.parameters().is_none()
}

/// `define_method(:hash) { ... }` or `define_singleton_method(:hash) do ...
/// end`: a plain call with the single argument `:hash` and a literal block
/// without parameters (RuboCop's `block` node with empty `args`, so not a
/// `_1` or `it` block).
fn is_dynamic_hash_method(call: &CallNode<'_>) -> bool {
    if !matches!(
        call.name().as_slice(),
        b"define_method" | b"define_singleton_method"
    ) || call.is_safe_navigation()
    {
        return false;
    }
    let names_hash = match call_arguments(call).as_slice() {
        [name] => name
            .as_symbol_node()
            .is_some_and(|symbol| symbol.unescaped() == b"hash"),
        _ => false,
    };
    let Some(block) = call.block().and_then(|block| block.as_block_node()) else {
        return false;
    };
    let parameterless = match block.parameters() {
        None => true,
        Some(parameters) => parameters
            .as_block_parameters_node()
            .is_some_and(|parameters| {
                parameters.parameters().is_none() && parameters.locals().iter().next().is_none()
            }),
    };
    names_hash && parameterless
}

/// Overrides for the operator-assignment nodes (RuboCop's `op_asgn`), which
/// are combinators when their operator is one.
macro_rules! operator_writes {
    ($($method:ident: $kind:ident,)*) => {
        $(
            fn $method(&mut self, node: &ruby_prism::$kind<'pr>) {
                if is_combinator_operator(node.binary_operator().as_slice()) {
                    self.combinator(node.location().start_offset(), node.location().end_offset(), |finder| {
                        ruby_prism::$method(finder, node)
                    });
                } else {
                    ruby_prism::$method(self, node);
                }
            }
        )*
    };
}

impl<'pr> Visit<'pr> for Finder {
    fn visit_def_node(&mut self, node: &DefNode<'pr>) {
        let hash_method = usize::from(is_static_hash_method(node));
        self.hash_methods += hash_method;
        ruby_prism::visit_def_node(self, node);
        self.hash_methods -= hash_method;
    }

    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.check_array_hash(node);
        // The block node encloses the whole call in RuboCop's AST.
        let hash_method = usize::from(is_dynamic_hash_method(node));
        self.hash_methods += hash_method;
        if is_combinator_call(node) {
            // Only the receiver and arguments are inside the combinator; a
            // literal block wraps it.
            self.combinator(
                node.location().start_offset(),
                crate::cops::ast::source_end(&node.as_node()),
                |finder| {
                    if let Some(receiver) = node.receiver() {
                        finder.visit(&receiver);
                    }
                    if let Some(arguments) = node.arguments() {
                        finder.visit_arguments_node(&arguments);
                    }
                    if let Some(block) = node
                        .block()
                        .filter(|block| block.as_block_argument_node().is_some())
                    {
                        finder.visit(&block);
                    }
                },
            );
            if let Some(block) = node.block().filter(|block| block.as_block_node().is_some()) {
                self.visit(&block);
            }
        } else {
            ruby_prism::visit_call_node(self, node);
        }
        self.hash_methods -= hash_method;
    }

    operator_writes! {
        visit_call_operator_write_node: CallOperatorWriteNode,
        visit_class_variable_operator_write_node: ClassVariableOperatorWriteNode,
        visit_constant_operator_write_node: ConstantOperatorWriteNode,
        visit_constant_path_operator_write_node: ConstantPathOperatorWriteNode,
        visit_global_variable_operator_write_node: GlobalVariableOperatorWriteNode,
        visit_index_operator_write_node: IndexOperatorWriteNode,
        visit_instance_variable_operator_write_node: InstanceVariableOperatorWriteNode,
        visit_local_variable_operator_write_node: LocalVariableOperatorWriteNode,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/security/compound_hash_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::pipeline::lint_str;

    const COMBINATOR: &str = "Use `[...].hash` instead of combining hash values manually.";
    const MONUPLE: &str =
        "Delegate hash directly without wrapping in an array when only using a single value.";
    const REDUNDANT: &str = "Calling .hash on elements of a hashed array is redundant.";

    fn found(source: &str) -> Vec<(u32, u32, &'static str)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Security/CompoundHash")
            .map(|o| {
                let message = [COMBINATOR, MONUPLE, REDUNDANT]
                    .into_iter()
                    .find(|known| *known == o.message)
                    .unwrap_or_else(|| panic!("unexpected message {:?}", o.message));
                (o.line, o.col, message)
            })
            .collect()
    }

    /// Positions of the combinator offenses in `source`.
    fn combinators(source: &str) -> Vec<(u32, u32)> {
        found(source)
            .into_iter()
            .filter(|(_, _, message)| *message == COMBINATOR)
            .map(|(line, col, _)| (line, col))
            .collect()
    }

    #[test]
    fn flags_xor_in_hash_methods() {
        for source in [
            "def hash\n  1.hash ^ 2.hash ^ 3.hash\nend\n",
            "def hash\n  1 ^ 2 ^ 3\nend\n",
            "def object.hash\n  1.hash ^ 2.hash ^ 3.hash\nend\n",
            "define_method(:hash) do\n  1.hash ^ 2.hash ^ 3.hash\nend\n",
            "define_singleton_method(:hash) do\n  1.hash ^ 2.hash ^ 3.hash\nend\n",
        ] {
            assert_eq!(found(source), [(2, 3, COMBINATOR)], "{source:?}");
        }
    }

    #[test]
    fn flags_a_single_element_array_hash() {
        assert_eq!(found("def hash\n  [1].hash\nend\n"), [(2, 3, MONUPLE)]);
    }

    #[test]
    fn flags_hash_calls_on_elements_of_a_hashed_array() {
        assert_eq!(found("[1, 2.hash, 3].hash\n"), [(1, 5, REDUNDANT)]);
        assert_eq!(
            found("def hash\n  [foo&.hash, bar&.hash].hash\nend\n"),
            [(2, 4, REDUNDANT), (2, 15, REDUNDANT)]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "def hash\n  [1, 2, 3].hash\nend\n",
            "def hash\n  1.hash\nend\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_combinators_in_assignments_and_operator_assignments() {
        assert_eq!(
            found("def hash\n  value = 1.hash ^ 2.hash ^ 3.hash\n  value\nend\n"),
            [(2, 11, COMBINATOR)]
        );
        for operator in ["^=", "+=", "*="] {
            let source = format!(
                "def hash\n  h = 0\n\n  things.each do |thing|\n    h {operator} thing.hash\n  end\n\n  h\nend\n"
            );
            assert_eq!(found(&source), [(5, 5, COMBINATOR)], "{source:?}");
        }
    }

    #[test]
    fn flags_addition_multiplication_and_mixed_operators() {
        for (source, col) in [
            ("def hash\n  foo.hash + bar.hash\nend\n", 3),
            ("def hash\n  to_s.hash * -1\nend\n", 3),
            (
                "def hash\n  [red, blue, green, alpha].hash ^ self.class.hash\nend\n",
                3,
            ),
            ("def hash\n  foo.hash ^ super ^ bar.hash\nend\n", 3),
            (
                "def hash\n  foo.hash ^ bar.hash << 1 ^ biz.hash << 2 ^ bar.hash << 3\nend\n",
                3,
            ),
            (
                "def hash\n  ([@addr, @mask_addr, @zone_id].hash << 1) | (ipv4? ? 0 : 1)\nend\n",
                3,
            ),
        ] {
            assert_eq!(found(source), [(2, col, COMBINATOR)], "{source:?}");
        }
    }

    #[test]
    fn flags_complex_usage() {
        let source = "\
def hash
  @hash ||= begin
    hash_ = [@factory, geometry_type].hash
    (0...num_geometries).inject(hash_) do |h_, i_|
      (1664525 * h_ + geometry_n(i_).hash).hash
    end
  end
end
";
        assert_eq!(found(source), [(5, 8, COMBINATOR)]);
    }

    #[test]
    fn flags_every_operator_assignment_target() {
        let source = "\
def hash
  @h += a
  self.h *= 2
  x[0] |= 1
  A ^= 1
  A::B += 1
  $g += 1
  @@c += 1
  h ||= a ^ b
end
";
        assert_eq!(
            combinators(source),
            [
                (2, 3),
                (3, 3),
                (4, 3),
                (5, 3),
                (6, 3),
                (7, 3),
                (8, 3),
                (9, 9)
            ]
        );
    }

    #[test]
    fn mixed_file_matches_rubocop() {
        // Expected positions checked against RuboCop 1.92.
        let source = "\
def hash
  a&.b += 1
  ::A += 1
  foo ^ super { }
  [x].hash.hash
  a.+ b do x * y end
  (a ^ b).hash
  [a ^ b].hash
end
class Foo
  class << self
    def hash
      x ^ y
    end
  end
end
[(a.hash), b.hash].hash
[a.hash, *b].hash
x = [a.hash,
     b.hash].hash
";
        assert_eq!(
            found(source),
            [
                (2, 3, COMBINATOR),
                (3, 3, COMBINATOR),
                (4, 3, COMBINATOR),
                (5, 3, MONUPLE),
                (6, 3, COMBINATOR),
                (6, 12, COMBINATOR),
                (7, 4, COMBINATOR),
                (8, 3, MONUPLE),
                (8, 4, COMBINATOR),
                (13, 7, COMBINATOR),
                (17, 12, REDUNDANT),
                (18, 2, REDUNDANT),
                (19, 6, REDUNDANT),
                (20, 6, REDUNDANT),
            ]
        );
    }

    #[test]
    fn reports_only_the_outermost_combinator() {
        assert_eq!(combinators("def hash\n  a + b + c\nend\n"), [(2, 3)]);
        assert_eq!(combinators("def hash\n  h += a + b\nend\n"), [(2, 3)]);
        // Any enclosing combinator counts, even outside the hash method.
        assert!(combinators("x + (def hash; a ^ b; end)\n").is_empty());
    }

    #[test]
    fn a_block_on_a_combinator_call_is_not_inside_it() {
        // In RuboCop's AST the block node wraps the `+` call, so the block
        // body is not inside the combinator.
        assert_eq!(
            combinators("def hash\n  a.+(b) { x ^ y }\nend\n"),
            [(2, 3), (2, 12)]
        );
        assert_eq!(
            combinators("def hash\n  x + foo { a ^ b }\nend\n"),
            [(2, 3)]
        );
    }

    #[test]
    fn counts_combinator_arguments_as_rubocop_does() {
        assert_eq!(combinators("def hash\n  a.+(*b)\nend\n"), [(2, 3)]);
        assert_eq!(combinators("def hash\n  a.+(&b)\nend\n"), [(2, 3)]);
        for source in [
            "def hash\n  a.+(b, c)\nend\n",
            "def hash\n  a&.+(b)\nend\n",
            "def hash\n  a - b\nend\n",
            "def hash\n  a & b\nend\n",
            "def hash\n  a << b\nend\n",
            "def hash\n  +a\nend\n",
        ] {
            assert!(combinators(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn hash_method_definitions() {
        for (source, position) in [
            ("def self.hash\n  a ^ b\nend\n", (2, 3)),
            ("def hash = a ^ b\n", (1, 12)),
            ("define_method(:hash) { || a ^ b }\n", (1, 27)),
            ("define_method(:hash) do a ^ b end\n", (1, 25)),
            ("define_method(:\"hash\") { a ^ b }\n", (1, 26)),
            ("define_method :hash do\n  a ^ b\nend\n", (2, 3)),
            (
                "x.define_singleton_method(:hash) do\n  a ^ b\nend\n",
                (2, 3),
            ),
            ("z.foo(define_method(:hash) { a ^ b })\n", (1, 30)),
            ("def (a ^ b).hash; end\n", (1, 6)),
            (
                "def hash\n  @foo.hash ^ @bar.hash\nrescue\n  0\nend\n",
                (2, 3),
            ),
        ] {
            assert_eq!(combinators(source), [position], "{source:?}");
        }
    }

    #[test]
    fn anything_inside_a_hash_method_counts() {
        // RuboCop checks every ancestor: blocks, lambdas, nested methods and
        // interpolation inside a hash method all count.
        for (source, position) in [
            ("def hash\n  [1, 2].map { |v| v * 2 }\nend\n", (2, 20)),
            ("def hash\n  -> { a | b }\nend\n", (2, 8)),
            ("def hash\n  def other\n    a ^ b\n  end\nend\n", (3, 5)),
            ("def hash\n  \"#{a ^ b}\"\nend\n", (2, 6)),
        ] {
            assert_eq!(combinators(source), [position], "{source:?}");
        }
    }

    #[test]
    fn ignores_combinators_outside_hash_methods() {
        for source in [
            "a ^ b\n",
            "def other\n  a ^ b\nend\n",
            "def hash(x)\n  a ^ b\nend\n",
            "def hash(...)\n  a ^ b\nend\n",
            "define_method(:hash) { |x| a ^ b }\n",
            "define_method(:hash) { |;x| a ^ b }\n",
            "define_method(:hash) { _1 ^ b }\n",
            "define_method(:hash) { it ^ b }\n",
            "obj&.define_method(:hash) { a ^ b }\n",
            "define_method(\"hash\") { a ^ b }\n",
            "define_method(:hash, 1) { a ^ b }\n",
            "define_method(:hash, &(a ^ b))\n",
            "define_method(:to_s) { a ^ b }\n",
            "def hash?\n  a ^ b\nend\n",
        ] {
            assert!(combinators(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn single_element_arrays() {
        assert_eq!(found("[*x].hash\n"), [(1, 1, MONUPLE)]);
        assert_eq!(found("key = %w[a].hash\n"), [(1, 7, MONUPLE)]);
        assert_eq!(found("[x].hash { }\n"), [(1, 1, MONUPLE)]);
        assert_eq!(
            found("[a.hash].hash\n"),
            [(1, 1, MONUPLE), (1, 2, REDUNDANT)]
        );
        assert_eq!(found("[[a.hash]].hash\n"), [(1, 1, MONUPLE)]);
        for source in [
            "[x].hash(1)\n",
            "[x]&.hash\n",
            "[x].hash(&blk)\n",
            "([x]).hash\n",
            "[].hash\n",
            "[x].to_s\n",
            "Array[x].hash\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn redundant_element_hashes() {
        // Only direct elements that are argument- and block-less `hash`
        // calls, including a bare `hash`.
        assert_eq!(
            found("[x.hash(1), y.hash { }, (z.hash), hash, *w.hash].hash\n"),
            [(1, 35, REDUNDANT)]
        );
        assert_eq!(
            found("[a.hash, b].hash { }\n"),
            [(1, 2, REDUNDANT)],
            "a block on the outer call does not matter"
        );
        for source in [
            "[a.hash, b].hash(&blk)\n",
            "[a.hash, b]&.hash\n",
            "[a.hash, b].to_s\n",
            "[a.hash, b]\n",
            "hash = 1\n[hash, b].hash\n",
            "{ a: b.hash }.hash\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
