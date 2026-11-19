//! `Style/StaticClass` — opt-in, unsafe suggestions only; never change class identity.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

/// Suggest modules without rewriting unsafe class/singleton semantics.
pub struct StaticClass;

impl Cop for StaticClass {
    fn name(&self) -> &'static str {
        "Style/StaticClass"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_class_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(class) = node.as_class_node() else {
            return;
        };
        if class.superclass().is_some() {
            return;
        }
        let elements = elements(class.body());
        if elements.is_empty() || !elements.iter().all(convertible_element) {
            return;
        }
        // `class` becomes `module` with `module_function`; `def self.x`
        // loses `self.`, and `class << self` its opening and `end`.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let span = |l: &ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
                let name_end = class.constant_path().location().end_offset();
                let mut edits = vec![
                    (span(&class.class_keyword_loc()), "module".to_owned()),
                    (name_end..name_end, "\nmodule_function\n".to_owned()),
                ];
                for element in &elements {
                    if let Some(def) = element.as_def_node()
                        && let Some(receiver) = def.receiver()
                    {
                        edits.push((
                            receiver.location().start_offset()..def.name_loc().start_offset(),
                            String::new(),
                        ));
                    } else if let Some(sclass) = element.as_singleton_class_node() {
                        edits.push((
                            sclass.class_keyword_loc().start_offset()
                                ..sclass.expression().location().end_offset(),
                            String::new(),
                        ));
                        edits.push((span(&sclass.end_keyword_loc()), String::new()));
                    }
                }
                crate::offense::Correction::from_edits(&file.bytes, edits)
            })
            .flatten();
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Prefer modules to classes with only class methods.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction,
        });
    }
}

fn convertible_element(node: &Node<'_>) -> bool {
    if let Some(singleton) = node.as_singleton_class_node() {
        return elements(singleton.body()).iter().all(|child| {
            child
                .as_def_node()
                .is_some_and(|method| method.receiver().is_none())
                || equals_assignment(child)
        });
    }
    // Visibility calls (inline, named or block form) cannot be eligible
    // elements themselves, so no accepted body can contain a non-public def.
    // Do not infer visibility or class identity across reopened definitions.
    node.as_def_node()
        .is_some_and(|method| method.receiver().is_some())
        || equals_assignment(node)
        || node.as_call_node().is_some_and(|call| {
            call.name().as_slice() == b"extend"
                && !call.is_safe_navigation()
                && !call
                    .block()
                    .is_some_and(|block| block.as_block_node().is_some())
        })
}

fn equals_assignment(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::MultiWriteNode { .. }
    )
}

// Parser collapses the implicit statement list, but keeps explicit grouping.
// class_elements unwraps only one :begin, whether implicit or parenthesized.
// Each disjoint direct body is inspected once; nested classes are dispatched
// separately by the shared complete walker, never recursively searched here.
fn elements<'pr>(body: Option<Node<'pr>>) -> Vec<Node<'pr>> {
    let Some(body) = body else {
        return Vec::new();
    };
    let nodes: Vec<_> = body
        .as_statements_node()
        .map(|s| s.body().iter().collect())
        .unwrap_or_else(|| vec![body]);
    if let [only] = nodes.as_slice()
        && let Some(group) = only.as_parentheses_node()
    {
        return group
            .body()
            .and_then(|body| body.as_statements_node())
            .map(|body| body.body().iter().collect())
            .unwrap_or_default();
    }
    nodes
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/static_class_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(&format!("# rustocop:enable Style/StaticClass\n{source}"))
            .into_iter()
            .filter(|o| o.cop == "Style/StaticClass")
            .collect()
    }
    #[test]
    fn flags_static_methods_assignments_singleton_bodies_and_extend() {
        for body in [
            "def self.class_method; end",
            "def other.class_method; end",
            "def self.class_method; end\nclass << self; def other_class_method; end; end",
            "CONST = 1\ndef self.class_method; end",
            "extend M\ndef self.class_method; end",
            "extend M",
            "CONST = 1",
            "class << self; end",
            "class << other; def foo; end; @a = 1; end",
            "@a = 1; @@b = 2; $c = 3; d = 4; X::Y = 5; a, b = values",
            "(def self.foo; end; CONST = 1)",
        ] {
            let source = format!("class C\n{body}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, 1, source.lines().count() as u32 + 1)
            );
            assert_eq!(
                found[0].message,
                "Prefer modules to classes with only class methods."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn ignores_instance_methods_macros_superclasses_modules_and_empty_classes() {
        for source in [
            "class C; end",
            "module C; def self.foo; end; end",
            "class C < B; def self.foo; end; end",
            "class C; def self.foo; end; def bar; end; end",
            "class C; def self.foo; end; macro_method; end",
            "class C; class << self; macro_method; end; end",
            "class C; include M; def self.foo; end; end",
            "class C; def self.foo; end; prepend M; end",
            "class C; def self.foo; end; attr_reader :foo; end",
            "class C; @@a ||= 1; end",
            "class C; object.value = 1; end",
            "class C; class Inner; def self.foo; end; end; end",
        ] {
            let found = offenses(source);
            // Only Inner is static in the final example, not its enclosing C.
            let expected = usize::from(source.contains("class Inner"));
            assert_eq!(found.len(), expected, "{source}");
        }
    }
    #[test]
    fn visibility_and_mixin_order_disqualify_without_leaking_across_scopes() {
        for body in [
            "private\ndef self.foo; end",
            "def self.foo; end\nprivate",
            "private_class_method def self.foo; end",
            "class << self; private; def foo; end; end",
            "class << self; def foo; end; private :foo; end",
            "class << self; public def foo; end; end",
            "extend M\ninclude N\ndef self.foo; end",
            "def self.foo; end\ninclude N\nextend M",
            "class << self; extend M; def foo; end; end",
            "(def self.foo; end)\nCONST = 1",
            "begin; def self.foo; end; end",
        ] {
            assert!(
                offenses(&format!("class C\n{body}\nend")).is_empty(),
                "{body}"
            );
        }
        let found =
            offenses("class C; private; def foo; end; end\nclass C; def self.bar; end; end");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 1));
        assert_eq!(
            offenses("module M; class ::C; def self.foo; end; end; end").len(),
            1
        );
    }
    #[test]
    fn qualified_dynamic_and_nested_declarations_report_the_class_not_its_methods() {
        for header in ["class ::C", "class Namespace::C", "class factory::C"] {
            let source = format!("module Outer\n  {header}\n    def self.foo; end\n  end\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 5));
            assert_eq!(
                found[0].message,
                "Prefer modules to classes with only class methods."
            );
        }
    }

    #[test]
    fn wide_singleton_bodies_have_bounded_direct_child_work() {
        let source = format!(
            "class C\nclass << self\n{}end\nend",
            "def foo; end\n".repeat(2000)
        );
        assert_eq!(offenses(&source).len(), 1);
    }
}
