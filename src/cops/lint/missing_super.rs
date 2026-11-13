//! `Lint/MissingSuper` — constructors and lifecycle callbacks without `super`.
use std::collections::HashSet;

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MissingSuper;
impl Cop for MissingSuper {
    fn name(&self) -> &'static str {
        "Lint/MissingSuper"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
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
        let constructor = def.receiver().is_none() && def.name().as_slice() == b"initialize";
        let callback = matches!(
            def.name().as_slice(),
            b"inherited"
                | b"method_added"
                | b"method_removed"
                | b"method_undefined"
                | b"singleton_method_added"
                | b"singleton_method_removed"
                | b"singleton_method_undefined"
        ) && cx.ancestors().any(|n| {
            matches!(
                n,
                Node::ClassNode { .. } | Node::SingletonClassNode { .. } | Node::ModuleNode { .. }
            )
        });
        if !(constructor && stateful_parent(cx) || callback) {
            return;
        }
        let root = cx.ancestors().last().unwrap_or(node);
        if file
            .missing_super_facts(root)
            .with_super
            .contains(&key(node))
        {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(node.location().end_offset().saturating_sub(1)),
            message: if constructor {
                "Call `super` to initialize state of the parent class."
            } else {
                "Call `super` to invoke callback defined in the parent class."
            }
            .into(),
            correction: None,
        });
    }
}

fn stateful_parent(cx: &NodeContext<'_, '_>) -> bool {
    // Upstream prioritizes the nearest ANY block, even outside an inner class.
    // Prism nests a block below its call/super; RuboCop also makes it an
    // ancestor of the callee's receiver and arguments. Project that edge too.
    let mut ancestors = cx.ancestors();
    while let Some(node) = ancestors.next() {
        if node.as_lambda_node().is_some() {
            return false;
        }
        if node.as_block_node().is_some() {
            return ancestors.next().is_some_and(|n| stateful_class_new(n));
        }
        if has_literal_block(node) {
            return stateful_class_new(node);
        }
    }
    cx.ancestors()
        .find_map(|n| n.as_class_node())
        .and_then(|c| c.superclass())
        .is_some_and(|n| !allowed_parent(&n))
}
fn has_literal_block(node: &Node<'_>) -> bool {
    let block = match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().block(),
        Node::SuperNode { .. } => node.as_super_node().unwrap().block(),
        Node::ForwardingSuperNode { .. } => {
            return node.as_forwarding_super_node().unwrap().block().is_some();
        }
        _ => None,
    };
    block.is_some_and(|b| b.as_block_node().is_some())
}
fn stateful_class_new(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.is_safe_navigation()
        || call.name().as_slice() != b"new"
        || !call
            .receiver()
            .is_some_and(|n| ast::is_global_const(&n, &["Class"]))
    {
        return false;
    }
    // Only the one-argument matcher matters; do not collect a potentially
    // large argument list for each definition nested under the same block.
    let Some(arguments) = call.arguments() else {
        return false;
    };
    let arguments = arguments.arguments();
    arguments.len() == 1
        && arguments
            .iter()
            .next()
            .is_some_and(|parent| !allowed_parent(&parent))
}
fn allowed_parent(node: &Node<'_>) -> bool {
    // Unlike global_const?, upstream const_name also accepts a plain casgn.
    // Inspect its target, not its value; parentheses/begin and compound writes
    // are distinct reference nodes and must not gain the exemption.
    if let Some(write) = node.as_constant_write_node() {
        return matches!(write.name().as_slice(), b"Object" | b"BasicObject");
    }
    let target = node
        .as_constant_path_write_node()
        .map(|w| w.target().as_node());
    ast::is_global_const(target.as_ref().unwrap_or(node), &["Object", "BasicObject"])
}
fn key(node: &Node<'_>) -> (usize, usize) {
    (node.location().start_offset(), node.location().end_offset())
}

/// One lazy postorder pass over the main parse. A `super` anywhere in the
/// definition (receiver/parameters/nested scopes/dead code/heredoc children)
/// counts. Propagate one bit per edge, not one subtree rescan per definition.
#[derive(Default)]
pub(crate) struct Facts {
    with_super: HashSet<(usize, usize)>,
    #[cfg(test)]
    visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Builder {
            facts: Facts,
            stack: Vec<(Option<(usize, usize)>, bool)>,
        }
        impl<'pr> NodeVisitor<'pr> for Builder {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                self.stack.push((
                    node.as_def_node().map(|_| key(&node)),
                    matches!(
                        node,
                        Node::SuperNode { .. } | Node::ForwardingSuperNode { .. }
                    ),
                ));
            }
            fn leave(&mut self) {
                let (definition, contains) = self.stack.pop().unwrap();
                if contains {
                    if let Some(definition) = definition {
                        self.facts.with_super.insert(definition);
                    }
                    if let Some(parent) = self.stack.last_mut() {
                        parent.1 = true;
                    }
                }
            }
        }
        let mut builder = Builder::default();
        walk::walk(&mut builder, root);
        builder.facts
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/lint/missing_super_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/MissingSuper")
            .collect()
    }
    #[test]
    fn constructors_report_whole_definition_without_correction() {
        for source in [
            "class Child < Parent\n  def initialize\n  end\nend\n",
            "class Child < Parent\n  def initialize\n  end\n  def do_something; end\nend\n",
            "Class.new(Parent) do\n  def initialize\n  end\nend\n",
            "Class.new(Parent) do\n  def initialize\n  end\n  do_something(_1)\nend\n",
            "Class.new(Parent) do\n  def initialize\n  end\n  do_something(it)\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            let o = &found[0];
            assert_eq!((o.line, o.col, o.end_line), (2, 3, 3));
            assert_eq!(
                o.message,
                "Call `super` to initialize state of the parent class."
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn constructor_lookalikes_and_explicit_super() {
        for source in [
            "class Child; def initialize; end; end",
            "class Child < Object; def initialize; end; end",
            "class Child < ::BasicObject; def initialize; end; end",
            "class Child < Parent; Class.new do; def initialize; end; end; end",
            "module M; def initialize; end; end",
            "class Child < Parent; def initialize; super; end; end",
            "class Child < Parent; def initialize; super(); end; end",
            "class Child < Parent; def self.initialize; end; end",
            "def initialize; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for parent in ["", "Object", "::BasicObject"] {
            for tail in ["", "; do_something(_1)", "; do_something(it)"] {
                let source = format!("Class.new({parent}) do; def initialize; end{tail}; end");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn direct_stateless_parent_assignments_use_the_assigned_constant_name() {
        // const_name accepts casgn as well as const.
        // These are syntax-only fixtures: never execute the assignments.
        for source in [
            "Class.new(Object = Parent) { def initialize; end }",
            "class C < Object = Parent; def initialize; end; end",
            "Class.new(::BasicObject = Parent) { def initialize; end }",
        ] {
            assert_eq!(ruby_prism::parse(source.as_bytes()).errors().count(), 0);
            assert!(offenses(source).is_empty(), "{source}");
        }
        for parent in ["Object", "::Object", "BasicObject", "::BasicObject"] {
            for value in ["Parent", "nil", "Other = Parent", "-> {}", "Class.new {}"] {
                for source in [
                    format!("class C < {parent} = {value}; def initialize; end; end"),
                    format!("Class.new({parent} = {value}) {{ def initialize; end }}"),
                    format!("::Class.new({parent} = {value}) {{ def initialize; end; use(_1) }}"),
                    format!("Class.new({parent} = {value}) do; def initialize; end; use(it); end"),
                ] {
                    assert_eq!(
                        ruby_prism::parse(source.as_bytes()).errors().count(),
                        0,
                        "{source}"
                    );
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
        }
    }
    #[test]
    fn stateless_parent_assignment_lookalikes_still_report_exact_findings() {
        for parent in [
            "Other::Object = Parent",
            "self::BasicObject = Parent",
            "(Object = Parent)",
            "begin Object = Parent end",
            "Object ||= Parent",
            "::BasicObject &&= Parent",
            "Object += Parent",
            "Parent = Object",
            "Object::Parent = Object",
        ] {
            for source in [
                format!("Class.new({parent}) {{ def initialize; end }}"),
                format!("class C < {parent}; def initialize; end; end"),
            ] {
                assert_eq!(
                    ruby_prism::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, source.find("def initialize").unwrap() as u32 + 1, 1)
                );
                assert_eq!(
                    found[0].message,
                    "Call `super` to initialize state of the parent class."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn nearest_any_block_outranks_even_an_inner_class() {
        for source in [
            "other { class Child < Parent; def initialize; end; end }",
            "-> { class Child < Parent; def initialize; end; end }",
            "Class.new(Object) { class Child < Parent; def initialize; end; end }",
            "class Child < Parent; other { def initialize; end }; end",
            "Class.new(Parent) { other { def initialize; end } }",
            "Other::Class.new(Parent) { def initialize; end }",
            "Class&.new(Parent) { def initialize; end }",
            "Class.new(Parent, Other) { def initialize; end }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "Class.new(Parent) { class Child < Object; def initialize; end; end }",
            "class Child < factory; def initialize; end; end",
            "class Child < Other::Object; def initialize; end; end",
            "class Child < Array; def initialize; end; end",
            "Class.new(Parent) { def initialize = nil }",
            "class Child < Parent; module M; def initialize; end; end; end",
            "class Child < Parent; class << self; def initialize; end; end; end",
            "Class.new(def initialize; end) { value }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn super_literal_blocks_are_ancestors_of_arguments_not_just_the_body() {
        for source in [
            "class C < Parent; def outer; super(def initialize; end) {}; end; end",
            "class C < Parent; def outer; super((def initialize; end)) { value }; end; end",
            "class C < Parent; def outer; super(k: (def initialize; end)) { value }; end; end",
            "Class.new(Parent) { super(def initialize; end) { value } }",
            "class C < Parent; super(def initialize; end) { value }; end",
            "class C < Parent; super { def initialize; end }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // A forwarded/explicit block argument is not a literal any-block;
        // absent that boundary the nearest class/Class.new still applies.
        for source in [
            "class C < Parent; def outer; super(def initialize; end); end; end",
            "class C < Parent; def outer; super(def initialize; end, &block); end; end",
            "Class.new(Parent) { super(def initialize; end, &block) }",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].line, 1);
            assert_eq!(found[0].end_line, 1);
            assert_eq!(
                found[0].col,
                source.find("def initialize").unwrap() as u32 + 1
            );
            assert_eq!(
                found[0].message,
                "Call `super` to initialize state of the parent class."
            );
            assert!(found[0].correction.is_none());
        }
        // Callback ancestry is independent of literal-block precedence.
        assert_eq!(
            offenses("module M; super(def self.inherited; end) {}; end").len(),
            1
        );
    }
    #[test]
    fn every_super_descendant_counts_without_scope_or_flow_barriers() {
        for body in [
            "super",
            "super()",
            "super(a)",
            "super if false",
            "-> { super }",
            "proc { super }",
            "def unrelated; super; end",
            "class Other; def unrelated; super; end; end",
            "module M; def unrelated; super; end; end",
            "\"#{super}\"",
            "begin; value; rescue; super; ensure; done; end",
            "consume(<<TEXT)\n#{super}\nTEXT\n",
        ] {
            for source in [
                format!("class Child < Parent; def initialize; {body}; end; end"),
                format!("module M; def self.inherited(*); {body}; end; end"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for source in [
            "class Child < Parent; def initialize(x = super()); end; end",
            "class Child < Parent; def initialize; def (super()).f; end; end; end",
            "class Foo; def (super()).inherited; end; end",
            "class Child < Parent; def initialize = super; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for body in [
            "'super'",
            ":super",
            "object.super",
            "# super\n nil",
            "superclass",
            "defined?(superclass)",
        ] {
            assert_eq!(
                offenses(&format!(
                    "class Child < Parent; def initialize; {body}; end; end"
                ))
                .len(),
                1
            );
        }
        let source = "class Child < Parent; def initialize; end; def other; super; end; end";
        assert_eq!(
            offenses(source).len(),
            1,
            "sibling super is not a descendant"
        );
    }
    #[test]
    fn facts_are_postorder_bounded_and_include_typed_and_heredoc_children() {
        use crate::cops::walk::{self, NodeVisitor};
        struct Count(usize);
        impl<'pr> NodeVisitor<'pr> for Count {
            fn enter(&mut self, _: ruby_prism::Node<'pr>) {
                self.0 += 1;
            }
        }
        let mut source = "class Child < Parent;".to_string();
        for i in 0..100 {
            source.push_str(&format!(
                "def method_{i}(x = super()); consume(<<TEXT)\n#{{super()}}\nTEXT\nend;"
            ));
        }
        source.push_str("end");
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let root = parsed.node();
        let file = crate::source::SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let facts = file.missing_super_facts(&root);
        assert!(std::ptr::eq(facts, file.missing_super_facts(&root)));
        let mut count = Count(0);
        walk::walk(&mut count, &root);
        assert_eq!(facts.visits, count.0);
        assert_eq!(facts.with_super.len(), 100);
        let other = ruby_prism::parse(b"def initialize; nil; end");
        assert!(super::Facts::build(&other.node()).with_super.is_empty());
    }
    #[test]
    fn callback_contexts_and_exact_messages() {
        for method in [
            "inherited",
            "method_added",
            "method_removed",
            "method_undefined",
            "singleton_method_added",
            "singleton_method_removed",
            "singleton_method_undefined",
        ] {
            for (open, close) in [
                ("class Foo", "end"),
                ("module M", "end"),
                ("class << self", "end"),
            ] {
                for receiver in ["", "self.", "object."] {
                    let source = format!("{open}\n  def {receiver}{method}(*)\n  end\n{close}");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 3));
                    assert_eq!(
                        found[0].message,
                        "Call `super` to invoke callback defined in the parent class."
                    );
                    assert!(found[0].correction.is_none());
                }
            }
            for source in [
                format!("def {method}; end"),
                format!("def self.{method}; end"),
                format!("Class.new(Parent) {{ def {method}; end }}"),
                format!("class Foo; def self.{method}; do_something; super; end; end"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for name in [
            "included",
            "extended",
            "prepended",
            "method_missing",
            "respond_to_missing?",
            "initialize_copy",
            "initializes",
        ] {
            assert!(
                offenses(&format!(
                    "class Foo < Parent; def {name}; end; def self.{name}; end; end"
                ))
                .is_empty()
            );
        }
    }
}
