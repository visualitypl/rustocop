//! `Style/ColonMethodCall` — prefer `.` for method dispatch.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct ColonMethodCall;

#[cfg(test)]
thread_local! {
    static ROOT_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Cop for ColonMethodCall {
    fn name(&self) -> &'static str {
        "Style/ColonMethodCall"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism folds the synthetic send on an assignment LHS into its
        // write/target node. It must be checked once, just like a plain call.
        macro_rules! parts {
            ($access:ident, $name:ident) => {{
                let call = node.$access().unwrap();
                (call.call_operator_loc(), call.$name(), call.receiver())
            }};
        }
        let (operator, name, receiver) = match node {
            Node::CallNode { .. } => parts!(as_call_node, name),
            Node::CallAndWriteNode { .. } => parts!(as_call_and_write_node, read_name),
            Node::CallOrWriteNode { .. } => parts!(as_call_or_write_node, read_name),
            Node::CallOperatorWriteNode { .. } => parts!(as_call_operator_write_node, read_name),
            Node::CallTargetNode { .. } => {
                let call = node.as_call_target_node().unwrap();
                (
                    Some(call.call_operator_loc()),
                    call.name(),
                    Some(call.receiver()),
                )
            }
            _ => return,
        };
        let Some(operator) = operator else { return };
        if operator.as_slice() != b"::"
            || name.as_slice().first().is_some_and(u8::is_ascii_uppercase)
            || receiver.is_some_and(java_interop)
        {
            return;
        }
        let (line, col) = file.line_col(operator.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use `::` for method calls.".into(),
            line,
            col,
            end_line: line,
            correction: cx.corrections_requested().then(|| Correction {
                range: operator.start_offset()..operator.end_offset(),
                replacement: ".".into(),
            }),
        });
    }
}

// Match the syntactic unqualified Java root, not an inferred runtime binding.
// Calls and singleton definitions expose `receiver` in the reference AST:
// constant namespaces and parentheses stop the chain, even `Java::Foo`.
// Receiver chains are bounded by the pipeline's fixed AST depth budget.
fn java_interop(mut receiver: Node<'_>) -> bool {
    loop {
        #[cfg(test)]
        ROOT_STEPS.with(|steps| steps.set(steps.get() + 1));
        let parent = if let Some(call) = receiver.as_call_node() {
            call.receiver()
        } else if let Some(definition) = receiver.as_def_node() {
            definition.receiver().map(|receiver| {
                // The reference AST drops the definition receiver's syntactic
                // parentheses, but retains any additional grouping boundary.
                let Some(body) = receiver
                    .as_parentheses_node()
                    .and_then(|group| group.body())
                else {
                    return receiver;
                };
                if let Some(statements) = body.as_statements_node() {
                    if statements.body().len() != 1 {
                        return receiver;
                    }
                    statements.body().first().unwrap()
                } else {
                    body
                }
            })
        } else {
            None
        };
        let Some(parent) = parent else {
            return receiver
                .as_constant_read_node()
                .is_some_and(|constant| constant.name().as_slice() == b"Java");
        };
        receiver = parent;
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/colon_method_call_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ColonMethodCall")
            .collect()
    }

    #[test]
    fn pinned_method_calls_report_the_dispatch_token() {
        for (source, col) in [
            ("test::method_name\n", 5),
            ("test::method_name(arg)\n", 5),
            ("Class::method_name\n", 6),
            ("Class::method_name(arg, arg2)\n", 6),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(found[0].message, "Do not use `::` for method calls.");
            assert!(
                found[0].correction.is_none(),
                "normal lint stays correction-free"
            );
        }
    }

    #[test]
    fn nested_dispatches_keep_constant_lookup_and_exact_positions() {
        let before = "# nested\n  outer::inner(::Root::build())\n";
        let found = offenses(before);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col), (2, 8));
        assert_eq!((found[1].line, found[1].col), (2, 22));
        assert_correction(
            &ColonMethodCall,
            before,
            "# nested\n  outer.inner(::Root.build())\n",
        );
    }

    #[test]
    fn pinned_canonical_corrections() {
        for before in [
            "test::method_name\n",
            "test::method_name(arg)\n",
            "Class::method_name\n",
            "Class::method_name(arg, arg2)\n",
        ] {
            assert_correction(&ColonMethodCall, before, &before.replace("::", "."));
        }
    }

    #[test]
    fn assignment_dispatch_in_prism_write_and_target_nodes() {
        for expression in [
            "object::value = rhs",
            "object::value ||= rhs",
            "object::value &&= rhs",
            "object::value += rhs",
            "object::value, other = rhs",
        ] {
            let source = format!("# assignments\n  {expression}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 9, 2));
            assert_eq!(found[0].message, "Do not use `::` for method calls.");
            assert_correction(&ColonMethodCall, &source, &source.replace("::", "."));
        }
    }

    #[test]
    fn dispatch_forms_preserve_every_byte_outside_the_operator() {
        for before in [
            "object::method_name arg, keyword: value\n",
            "object::method_name *args, **keywords, &nil\n",
            "def f(&block); object::method_name *args, **keywords, &block; end\n",
            "consume(object::method_name arg)\n",
            "object::method_name arg { effect }\n",
            "consume object::method_name arg do effect end\n",
            "object::method_name(<<~TEXT)\n  value\nTEXT\n",
            "object::method_name <<~TEXT\n  #{effect}\nTEXT\n",
            "object:: # dispatch comment\n method_name(arg)\n",
            concat!("object::\\", "\n method_name arg\n"),
            "object::(arg)\n",
            "object::[](index)\n",
            "object::[]=(index, value)\n",
            "object::+(arg)\n",
            "object::!\n",
            "1::to_s\n",
            "defined?(object::method_name)\n",
            "object&.child::method_name(arg)\n",
            "object::method_name&.child(arg)\n",
            "object::method_name::VALUE\n",
            "self::method_name\n",
            "Tip::Äbc(arg)\n",
        ] {
            assert_eq!(offenses(before).len(), 1, "{before}");
            // A constant suffix must keep its lookup operator.
            let after = before.replacen("::", ".", 1);
            assert_correction(&ColonMethodCall, before, &after);
        }
    }

    #[test]
    fn unknown_proc_acquisition_retains_the_offense_without_an_edit() {
        let found = correction_offenses(
            &ColonMethodCall,
            "object::method_name *args, **keywords, &block\n",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn distinguishes_java_root_syntax_from_names_and_values() {
        for before in [
            "Java::Foo::method_name\n",
            "Java::Foo::value ||= rhs\n",
            "Java::Foo::value, other = rhs\n",
            "Other::Java::int\n",
            "::Java::int\n",
            "(Java)::int\n",
            "Java()::int\n",
            "java::int\n",
            "JavaLike::int\n",
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            let start = before.rfind("::").unwrap();
            assert_eq!((found[0].line, found[0].col), (1, (start + 1) as u32));
            let mut after = before.to_string();
            after.replace_range(start..start + 2, ".");
            assert_correction(&ColonMethodCall, before, &after);
        }
        for source in [
            "Java.foo::bar",
            "Java&.foo::bar",
            "Java::com::value ||= rhs",
            "Java::com::value, other = rhs",
            "module M; Java::int; end",
            "Java = Object.new; Java::int",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn java_root_exemption_traverses_singleton_definition_receivers() {
        for source in [
            "def Java.foo; end::bar",
            "def Java.foo; end::bar::baz",
            "def Java.foo; end.bar::baz",
            "def (Java).foo; end::bar",
            "def (Java.foo).bar; end::baz",
        ] {
            assert_eq!(crate::parser::parse(source.as_bytes()).errors().count(), 0);
            assert!(offenses(source).is_empty(), "{source}");
            assert!(
                crate::cops::testing::correction_offenses(&ColonMethodCall, source).is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn singleton_definition_receivers_preserve_java_root_boundaries() {
        for source in [
            "def (Java::Foo).foo; end::bar",
            "def (::Java).foo; end::bar",
            "def (Other::Java).foo; end::bar",
            "def ((Java)).foo; end::bar",
            "(def Java.foo; end)::bar",
            "def Other.foo; end::bar",
            "def foo; end::bar",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            let start = source.rfind("::").unwrap();
            assert_eq!((found[0].line, found[0].col), (1, (start + 1) as u32));
            assert_eq!(found[0].message, "Do not use `::` for method calls.");
            let mut after = source.to_string();
            after.replace_range(start..start + 2, ".");
            assert_correction(&ColonMethodCall, source, &after);
        }
    }

    #[test]
    fn ignores_safe_navigation_constants_uppercase_methods_and_text() {
        for source in [
            "object.method_name(arg)",
            "object&.method_name(arg)",
            "::Constant",
            "Object::Constant = 1",
            "Object::Constant ||= 1",
            "Object::Constructor(arg)",
            "object::UPPER(arg)",
            "method_name arg",
            "def object.method_name; end",
            "object[index] = value",
            "object[index] ||= value",
            "# object::method_name\n'object::method_name'\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn repeated_java_chains_have_bounded_main_ast_work() {
        use crate::{cops::testing::correction_offenses, parser};
        for count in [100, 2000] {
            for (chain, steps) in [
                ("Java::com::method_name\n", 3),
                ("def Java.foo; end::bar::baz\n", 5),
            ] {
                let source = chain.repeat(count);
                assert_eq!(parser::parse(source.as_bytes()).errors().count(), 0);
                ROOT_STEPS.with(|steps| steps.set(0));
                assert!(correction_offenses(&ColonMethodCall, &source).is_empty());
                assert_eq!(ROOT_STEPS.with(|steps| steps.get()), steps * count);
            }
        }
    }

    #[test]
    fn pinned_constant_constructor_and_java_lookalikes() {
        for source in [
            "Tip::Top::SOME_CONST",
            "Tip::Top.some_method",
            "Tip::Top.some_method[3]",
            "Tip::Top(some_arg)",
            "Java::int",
            "Java::com",
            "Java::com::something_method",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
