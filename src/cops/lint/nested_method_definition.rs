//! `Lint/NestedMethodDefinition` — flags a `def` nested inside another
//! `def`: the inner method is defined on the outer method's class (not
//! locally), and redefined on every call.
//!
//! Matches RuboCop's semantics with its defaults (`AllowedMethods: []`,
//! `AllowedPatterns: []`), fixed in rustocop. A nested `def` is accepted when
//! any enclosing node, at any depth, is a `class << ...` body or a call with
//! a literal block that opens a new scope: `instance_eval`, `class_eval`,
//! `module_eval` and their `_exec` forms (any receiver), or `Class.new`,
//! `Module.new`, `Struct.new` or `Data.define` (on `Name` or `::Name`). Safe
//! navigation (`&.`) calls do not open a scope, as in RuboCop. Singleton
//! methods on a variable, constant or method call (`def obj.y`) are
//! accepted; `def self.y` is not. The offense is reported at the `def`.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/NestedMethodDefinition` cop.
pub struct NestedMethodDefinition;

impl Cop for NestedMethodDefinition {
    fn name(&self) -> &'static str {
        "Lint/NestedMethodDefinition"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        if def
            .receiver()
            .is_some_and(|receiver| is_allowed_subject(&receiver))
        {
            return;
        }
        // A scope-opening ancestor at any depth excuses it, even one above
        // the enclosing `def`.
        let mut nested = false;
        for ancestor in cx.ancestors() {
            match ancestor {
                Node::DefNode { .. } => nested = true,
                Node::SingletonClassNode { .. } => return,
                // In RuboCop's AST the block node encloses its call, so the
                // call's receiver and arguments are inside the scope too.
                Node::CallNode { .. } if opens_scope(&ancestor.as_call_node().unwrap()) => return,
                _ => {}
            }
        }
        if !nested {
            return;
        }
        let (line, col) = file.line_col(def.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Method definitions must not be nested. Use `lambda` instead.".to_string(),
            line,
            end_line: file.last_line_of(def.location()),
            col,
        });
    }
}

/// RuboCop's `allowed_subject_type?`: the receiver of `def recv.name` is a
/// variable, a constant or a method call (parentheses are transparent).
fn is_allowed_subject(receiver: &Node<'_>) -> bool {
    match receiver {
        Node::ParenthesesNode { .. } => {
            let Some(body) = receiver.as_parentheses_node().unwrap().body() else {
                return false;
            };
            match body.as_statements_node() {
                Some(statements) => {
                    let body = statements.body();
                    body.iter().count() == 1 && body.iter().all(|inner| is_allowed_subject(&inner))
                }
                None => is_allowed_subject(&body),
            }
        }
        Node::LocalVariableReadNode { .. }
        | Node::InstanceVariableReadNode { .. }
        | Node::ClassVariableReadNode { .. }
        | Node::GlobalVariableReadNode { .. }
        | Node::ConstantReadNode { .. }
        | Node::ConstantPathNode { .. }
        | Node::CallNode { .. } => true,
        _ => false,
    }
}

/// Whether a call with a literal block opens a scope in which `def` defines
/// methods elsewhere: RuboCop's `eval_call?`, `exec_call?` and
/// `class_constructor?`.
fn opens_scope(call: &CallNode<'_>) -> bool {
    if call.is_safe_navigation() || !call.block().is_some_and(|b| b.as_block_node().is_some()) {
        return false;
    }
    const EVAL_METHODS: [&[u8]; 6] = [
        b"instance_eval",
        b"class_eval",
        b"module_eval",
        b"instance_exec",
        b"class_exec",
        b"module_exec",
    ];
    let name = call.name().as_slice();
    if EVAL_METHODS.contains(&name) {
        return true;
    }
    let Some(receiver) = call.receiver() else {
        return false;
    };
    match name {
        b"new" => is_global_const(&receiver, &[b"Class", b"Module", b"Struct"]),
        b"define" => is_global_const(&receiver, &[b"Data"]),
        _ => false,
    }
}

/// `Name` or `::Name` for one of `names`.
fn is_global_const(node: &Node<'_>, names: &[&[u8]]) -> bool {
    match node {
        Node::ConstantReadNode { .. } => {
            names.contains(&node.as_constant_read_node().unwrap().name().as_slice())
        }
        Node::ConstantPathNode { .. } => {
            let path = node.as_constant_path_node().unwrap();
            path.parent().is_none()
                && path
                    .name()
                    .is_some_and(|name| names.contains(&name.as_slice()))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/nested_method_definition_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! (empty) `AllowedMethods` and `AllowedPatterns`.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/NestedMethodDefinition")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn flags_nested_definition_with_message() {
        let found = offenses("def x; def y; end; end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 8));
        assert_eq!(
            found[0].message,
            "Method definitions must not be nested. Use `lambda` instead."
        );
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        for (source, position) in [
            (
                "class Foo\nend\nfoo = Foo.new\ndef foo.bar\n  def baz\n  end\nend\n",
                (5, 3),
            ),
            ("def foo\n  bar = -> { def baz; puts; end }\nend\n", (2, 14)),
            (
                "class Foo\n  def self.x\n    def self.y\n    end\n  end\nend\n",
                (3, 5),
            ),
            // `AllowedMethods` and `AllowedPatterns` are empty.
            (
                "def do_something\n  has_many :articles do\n    def find_or_create_by_name(name)\n    end\n  end\nend\n",
                (3, 5),
            ),
            (
                "def do_something\n  denied_method :articles do\n    def find_or_create_by_name(name)\n    end\n  end\nend\n",
                (3, 5),
            ),
            (
                "def foo(obj)\n  obj.do_baz do\n    def bar\n    end\n  end\nend\n",
                (3, 5),
            ),
            (
                "def foo(obj)\n  obj.do_qux do\n    def bar\n    end\n  end\nend\n",
                (3, 5),
            ),
        ] {
            assert_eq!(positions(source), vec![position], "should flag {source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        let wrap = |body: &str| format!("class Foo\n  def x(obj)\n{body}  end\nend\n");
        let mut sources = vec![
            "def foo\n  bar = -> { puts  }\n  bar.call\nend\n".to_string(),
            "class Foo\n  def x\n    def @obj.y\n    end\n  end\nend\n".to_string(),
            "class Foo\n  def x\n    def @@obj.y\n    end\n  end\nend\n".to_string(),
            "class Foo\n  def x\n    def $obj.y\n    end\n  end\nend\n".to_string(),
            "class Foo\n  def x\n    def Const.y\n    end\n  end\nend\n".to_string(),
            "class Foo\n  def x\n    def do_something.y\n    end\n  end\nend\n".to_string(),
            "class Foo\n  def x\n    def (do_something&.y).z\n    end\n  end\nend\n".to_string(),
            "class Foo\n  def bar\n    class << self\n      def baz\n      end\n    end\n  end\nend\n"
                .to_string(),
            "class Foo\n  def x(obj)\n    def obj.y\n    end\n  end\nend\n".to_string(),
        ];
        for call in [
            "obj.instance_eval do",
            "obj.instance_exec do",
            "obj.class_eval do",
            "obj.class_exec do",
            "obj.module_eval do",
            "obj.module_exec do",
            "Class.new(S) do",
            "Class.new do",
            "::Class.new(S) do",
            "::Class.new do",
            "Module.new do",
            "::Module.new do",
            "Struct.new(:name) do",
            "Struct.new do",
            "::Struct.new(:name) do",
            "::Struct.new do",
            "Data.define(:name) do",
            "Data.define do",
            "::Data.define(:name) do",
            "::Data.define do",
        ] {
            sources.push(wrap(&format!(
                "    {call}\n      def y\n      end\n    end\n"
            )));
        }
        // Block parameters, numbered parameters and `it` are all blocks.
        for body in [
            "    Module.new do |m|\n      def y\n      end\n\n      do_something(m)\n    end\n",
            "    Module.new do\n      def y\n      end\n\n      do_something(_1)\n    end\n",
            "    obj.instance_eval do\n      @bar = _1\n      def y\n      end\n    end\n",
            "    obj.instance_exec(3) do\n      @bar = _1\n      def y\n      end\n    end\n",
            "    Module.new do\n      def y\n      end\n\n      do_something(it)\n    end\n",
            "    obj.instance_eval do\n      @bar = it\n      def y\n      end\n    end\n",
            "    obj.instance_exec(3) do\n      @bar = it\n      def y\n      end\n    end\n",
        ] {
            sources.push(wrap(body));
        }
        for source in &sources {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn scopes_count_at_any_depth() {
        // A scope above the outer `def` covers the whole chain.
        assert!(offenses("Class.new do\n  def a\n    def b; end\n  end\nend\n").is_empty());
        assert!(offenses("class << self\n  def a\n    def b; end\n  end\nend\n").is_empty());
        assert!(
            offenses("def a\n  instance_eval do\n    foo { def b; end }\n  end\nend\n").is_empty()
        );
        // Every level of nesting is reported.
        assert_eq!(
            positions("def a\n  def b\n    def c; end\n  end\nend\n"),
            vec![(2, 3), (3, 5)]
        );
        // A singleton method on a variable still counts as an enclosing def.
        assert_eq!(
            positions("def a(o)\n  def o.b\n    def c; end\n  end\nend\n"),
            vec![(3, 5)]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "def a; end\ndef b; end\n",
            "class Foo\n  def a; end\n  class << self\n    def b; end\n  end\nend\n",
            "def a\n  define_method(:b) { }\nend\n",
            "foo do\n  def a; end\nend\n",
            "-> { def a; end }\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn non_scoping_blocks_and_safe_navigation_do_not_excuse() {
        assert_eq!(
            positions("def a\n  define_method(:b) do\n    def c; end\n  end\nend\n"),
            vec![(3, 5)]
        );
        assert_eq!(
            positions("def a(o)\n  o&.instance_eval do\n    def c; end\n  end\nend\n"),
            vec![(3, 5)]
        );
        assert_eq!(
            positions("def a\n  Foo::Class.new do\n    def c; end\n  end\nend\n"),
            vec![(3, 5)]
        );
        assert_eq!(
            positions("def a\n  instance_eval(&blk)\n  def c; end\nend\n"),
            vec![(3, 3)]
        );
        assert_eq!(positions("def a\n  def self.b; end\nend\n"), vec![(2, 3)]);
    }
}
