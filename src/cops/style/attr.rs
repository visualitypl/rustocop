//! `Style/Attr` — prefer explicit attribute families.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Attr;
impl Cop for Attr {
    fn name(&self) -> &'static str {
        "Style/Attr"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
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
        let args = crate::cops::ast::call_arguments(&call);
        // In the reference AST the send is inside its own BlockNode. Prism
        // attaches that block to the call instead, so it is not an ancestor.
        if call.receiver().is_some()
            || call.name().as_slice() != b"attr"
            || args.is_empty()
            || super::accessors::literal_block(&call)
        {
            return;
        }
        let mut ancestors = cx.ancestors();
        if let Some(scope) = ancestors.find(|n| {
            matches!(
                n,
                Node::ClassNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
            )
        }) {
            let eval = scope.as_block_node().is_some()
                && ancestors
                    .next()
                    .and_then(Node::as_call_node)
                    .is_some_and(|c| {
                        !c.is_safe_navigation()
                            && matches!(c.name().as_slice(), b"class_eval" | b"module_eval")
                    });
            if (scope.as_class_node().is_none() && !eval)
                || cx
                    .ancestors()
                    .last()
                    .is_some_and(|root| file.custom_attrs(root).contains(scope))
            {
                return;
            }
        }
        let family = if args.last().is_some_and(|a| a.as_true_node().is_some()) {
            "attr_accessor"
        } else {
            "attr_reader"
        };
        let selector = call.message_loc().unwrap();
        let (line, col) = file.line_col(selector.start_offset());
        let correction = cx.corrections_requested().then(|| {
            let mut edits = vec![crate::offense::Correction {
                range: selector.start_offset()..selector.end_offset(),
                replacement: family.into(),
            }];
            if let [name, setter, ..] = args.as_slice()
                && matches!(setter, Node::TrueNode { .. } | Node::FalseNode { .. })
            {
                edits.push(crate::offense::Correction {
                    range: name.location().end_offset()..setter.location().end_offset(),
                    replacement: String::new(),
                });
            }
            super::correction::combine(file, edits)
        });
        let correction = correction.flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Do not use `attr`. Use `{family}` instead."),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/attr_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/Attr")
            .collect()
    }
    #[test]
    fn explicit_family_messages_and_selector_positions() {
        for (args, family) in [
            (":name", "attr_reader"),
            (":name, false", "attr_reader"),
            (":name, true", "attr_accessor"),
            (":foo, :bar", "attr_reader"),
            (":foo, flag", "attr_reader"),
        ] {
            let found = offenses(&format!("class Foo\n  attr({args})\nend"));
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("Do not use `attr`. Use `{family}` instead.")
            );
        }
    }
    #[test]
    fn scopes_custom_methods_and_eval_blocks() {
        for source in [
            "func(attr)",
            "x.attr arg",
            "class Foo; def attr(*args); end; def a; attr(1); end; end",
            "class Foo; factory { attr :name }; end",
            "-> { attr :name }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "attr :name",
            "module Foo; attr :name; end",
            "class << self; attr :name; end",
            "SomeClass.class_eval { attr :name }",
            "SomeClass.module_eval { attr :name }",
            "class Foo; def a; attr :name; end; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn attached_literal_blocks_are_exempt_contexts() {
        for source in [
            "class Foo\n  attr(:one) {}\nend\n",
            "class Foo\n  attr(:one) do\n  end\nend\n",
            "attr(:one) {}",
            "Foo.class_eval { attr(:one) {} }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
            assert!(
                crate::cops::testing::correction_offenses(&super::Attr, source).is_empty(),
                "{source}"
            );
        }
        // A block pass is not an ordinary literal-block context.
        assert_eq!(offenses("class Foo; attr(:one, &block); end").len(), 1);
    }
    #[test]
    fn canonical_nonboolean_corrections_keep_parentheses() {
        for (before, after) in [
            ("attr :name", "attr_reader :name"),
            ("attr(:foo, :bar)", "attr_reader(:foo, :bar)"),
        ] {
            crate::cops::testing::assert_correction(
                &super::Attr,
                &format!("class Foo\n  {before}\nend\n"),
                &format!("class Foo\n  {after}\nend\n"),
            );
        }
    }
    #[test]
    fn nearest_class_or_block_controls_custom_method_exemption() {
        for source in [
            "class Foo; class Bar; def attr; end; end; attr :a; end",
            "Foo.class_eval { def attr; end; attr :a }",
            "class Foo; factory { attr :a }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // A singleton definition is a defs, not a def, in upstream's AST.
        assert_eq!(
            offenses("class Foo; def self.attr; end; attr :a; end").len(),
            1
        );
        assert_eq!(offenses("module Foo; def attr; end; attr :a; end").len(), 1);
        assert_eq!(
            offenses("class Foo; factory { X.class_eval { attr :a } }; end").len(),
            1
        );
    }
    #[test]
    fn safe_navigation_eval_blocks_are_not_the_bare_eval_context() {
        for source in [
            "Foo&.class_eval { attr :a }",
            "Foo&.module_eval { attr :a }",
            "class Foo; self&.class_eval { attr :a }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
