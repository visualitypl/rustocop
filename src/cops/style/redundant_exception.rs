//! `Style/RedundantException` — fixed pinned exception policy.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantException;
impl Cop for RedundantException {
    fn name(&self) -> &'static str {
        "Style/RedundantException"
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.receiver().is_some() || !matches!(call.name().as_slice(), b"raise" | b"fail") {
            return;
        }
        let args = ast::call_arguments(&call);
        let (message, compact) = match args.as_slice() {
            [class, message] if ast::is_global_const(class, &["RuntimeError"]) => (message, false),
            [exception] => {
                let Some(new) = exception.as_call_node() else {
                    return;
                };
                if new.name().as_slice() != b"new"
                    || new.is_safe_navigation()
                    || new.block().is_some_and(|b| b.as_block_node().is_some())
                    || !new
                        .receiver()
                        .is_some_and(|r| ast::is_global_const(&r, &["RuntimeError"]))
                {
                    return;
                }
                let args = ast::call_arguments(&new);
                if args.len() != 1 {
                    return;
                }
                // Report while the temporary argument list is alive.
                return report(self, node, &args[0], true, cx, file, out);
            }
            _ => return,
        };
        report(self, node, message, compact, cx, file, out);
    }
}
fn report(
    cop: &RedundantException,
    node: &Node<'_>,
    message: &Node<'_>,
    compact: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    if message.as_nil_node().is_some() {
        return;
    }
    let (line, col) = file.line_col(node.location().start_offset());
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        line,
        col,
        end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
        message: if compact {
            "Redundant `RuntimeError.new` call can be replaced with just the message."
        } else {
            "Redundant `RuntimeError` argument can be removed."
        }
        .into(),
        correction: cx
            .corrections_requested()
            .then(|| correction(node, message, compact, cx, file))
            .flatten(),
    });
}
// Removing a lazy non-String message changes coercion timing, and a dynamic
// message may be nil (whose default message is the exception class name).
// Correct only actual String literals; interpolations can run arbitrary code.
fn correction(
    node: &Node<'_>,
    message: &Node<'_>,
    compact: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    message.as_string_node()?;
    let call = node.as_call_node()?;
    if call.block().is_some() {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    // Reopened lexical namespaces and inherited constants may resolve to a
    // different RuntimeError even with no declaration in this source file.
    if cx.ancestors().any(|ancestor| {
        matches!(
            ancestor,
            Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. }
        )
    }) || file.exception_safety(root).changed
        || file.contains_heredoc(root, message)
    {
        return None;
    }
    let args = ast::call_arguments(&call);
    let range = if compact {
        args[0].location().start_offset()..args[0].location().end_offset()
    } else {
        args[0].location().start_offset()..message.location().end_offset()
    };
    if super::correction::contains_comment(file, range.clone()) {
        return None;
    }
    let text = std::str::from_utf8(message.location().as_slice()).ok()?;
    let correction = crate::offense::Correction {
        range,
        replacement: text.into(),
    };
    // Keeping all physical newlines protects backtraces, rescue/ensure source
    // locations, __LINE__, CRLF and neighboring directives without reparsing.
    let old = &file.bytes[correction.range.clone()];
    (memchr::memchr_iter(b'\n', old).count() == memchr::memchr_iter(b'\n', text.as_bytes()).count())
        .then_some(correction)
}

/// One lazy complete main-AST binding walk, shared by every candidate.
#[derive(Default)]
pub(crate) struct Safety {
    changed: bool,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    /// Whether exception constant/method binding may have changed in this source.
    pub(crate) fn changed(&self) -> bool {
        self.changed
    }

    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        self.changed |= matches!(
            node,
            Node::ConstantWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
                | Node::ConstantAndWriteNode { .. }
                | Node::ConstantOrWriteNode { .. }
                | Node::ConstantOperatorWriteNode { .. }
                | Node::ConstantPathAndWriteNode { .. }
                | Node::ConstantPathOrWriteNode { .. }
                | Node::ConstantPathOperatorWriteNode { .. }
                | Node::ConstantTargetNode { .. }
                | Node::ConstantPathTargetNode { .. }
                | Node::AliasMethodNode { .. }
                | Node::UndefNode { .. }
        );
        if let Some(class) = node.as_class_node() {
            self.changed |= class.superclass().is_some();
            self.changed |= ast::const_name(&class.constant_path()).is_none_or(|name| {
                name.split("::").any(|part| {
                    matches!(
                        part,
                        "RuntimeError"
                            | "Exception"
                            | "StandardError"
                            | "Object"
                            | "BasicObject"
                            | "Class"
                            | "Module"
                            | "String"
                    )
                })
            });
        }
        if let Some(module) = node.as_module_node() {
            self.changed |= ast::const_name(&module.constant_path()).is_none_or(|name| {
                name.split("::")
                    .any(|part| matches!(part, "RuntimeError" | "Kernel" | "Object"))
            });
        }
        if let Some(def) = node.as_def_node() {
            self.changed |= matches!(
                def.name().as_slice(),
                b"raise"
                    | b"fail"
                    | b"new"
                    | b"exception"
                    | b"initialize"
                    | b"to_s"
                    | b"const_missing"
                    | b"method_missing"
                    | b"inherited"
            );
        }
        if let Some(call) = node.as_call_node() {
            self.changed |= crate::cops::code_object_safety::mutation(call.name().as_slice())
                || matches!(
                    call.name().as_slice(),
                    b"eval"
                        | b"class_eval"
                        | b"module_eval"
                        | b"instance_eval"
                        | b"send"
                        | b"__send__"
                        | b"public_send"
                        | b"method"
                        | b"public_method"
                        | b"instance_method"
                        | b"singleton_method"
                        | b"to_proc"
                );
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_exception_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn canonical_string_corrections() {
        for keyword in ["raise", "fail"] {
            for class in ["RuntimeError", "::RuntimeError"] {
                for (before, after) in [
                    (
                        format!("{keyword} {class}, \"message\""),
                        format!("{keyword} \"message\""),
                    ),
                    (
                        format!("{keyword}({class}, \"message\")"),
                        format!("{keyword}(\"message\")"),
                    ),
                    (
                        format!("{keyword} {class}.new \"message\""),
                        format!("{keyword} \"message\""),
                    ),
                    (
                        format!("{keyword} {class}.new(\"message\")"),
                        format!("{keyword} \"message\""),
                    ),
                ] {
                    crate::cops::testing::assert_correction(&RedundantException, &before, &after);
                }
            }
        }
    }
    #[test]
    fn non_string_interpolation_blocks_comments_and_newlines_remain_unedited() {
        for source in [
            "raise RuntimeError, Object.new",
            "raise RuntimeError.new(Object.new)",
            "raise RuntimeError, msg",
            "raise RuntimeError.new(a || b)",
            "raise RuntimeError, 1 + 2",
            "raise RuntimeError.new(foo.bar baz)",
            "raise RuntimeError, !foo",
            "raise RuntimeError, foo[1]",
            "raise RuntimeError, `command`",
            "raise RuntimeError, \"#{message}\"",
            "raise RuntimeError, 'x' do; break :result; end",
            "raise(RuntimeError, # retained\n'x')",
            "raise RuntimeError.new(\n'x'\n)",
            "raise RuntimeError, <<~MSG\n  hi\nMSG\n",
        ] {
            let found = correction_offenses(&RedundantException, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn binding_mutations_anywhere_withhold_only_the_edit() {
        for binding in [
            "RuntimeError = ArgumentError",
            "module M; RuntimeError = ArgumentError; end",
            "class RuntimeError; end",
            "class Exception; end",
            "module Kernel; end",
            "def raise(*a); a; end",
            "def self.fail(*a); a; end",
            "def RuntimeError.new(*a); super; end",
            "def RuntimeError.exception(*a); super; end",
            "alias custom raise",
            "Object.const_set(:RuntimeError, ArgumentError)",
            "RuntimeError.class_eval('def initialize(*a); super; end')",
            "RuntimeError.singleton_class.define_method(:exception) { |*a| ArgumentError.new }",
            "module M; refine Exception do; def to_s; 'changed'; end; end; end",
            "class A < Unknown; end",
            "require 'changes'",
            "public_send(:changes)",
        ] {
            let source = format!("{binding}\nraise RuntimeError, 'x'\n");
            let found = correction_offenses(&RedundantException, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn reopened_lexical_and_inherited_namespaces_are_not_constant_resolution_proofs() {
        for source in [
            "module Existing; raise RuntimeError, 'x'; end",
            "class Existing; def f; raise RuntimeError.new('x'); end; end",
            "class Existing < Base; raise RuntimeError, 'x'; end",
            "class << object; raise RuntimeError, 'x'; end",
        ] {
            let found = correction_offenses(&RedundantException, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn nil_receiver_namespace_backtrace_and_arity_lookalikes() {
        for source in [
            "raise RuntimeError, nil",
            "raise RuntimeError.new(nil)",
            "raise RuntimeError",
            "raise RuntimeError.new",
            "raise RuntimeError, 'x', caller",
            "raise RuntimeError.new('x'), 'other'",
            "raise Foo::RuntimeError, 'x'",
            "raise OtherError, 'x'",
            "Kernel.raise RuntimeError, 'x'",
            "self.fail RuntimeError.new('x')",
            "raise RuntimeError&.new('x')",
            "raise RuntimeError.new('x') { break :result }",
        ] {
            assert!(
                correction_offenses(&RedundantException, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn canonical_edits_preserve_literal_bytes_and_enclosing_expressions() {
        for (before, after) in [
            ("raise RuntimeError, %q(a\\nb)", "raise %q(a\\nb)"),
            (
                "raise(RuntimeError.new(\"\\x82\\n\"))",
                "raise(\"\\x82\\n\")",
            ),
            (
                "x ? raise(RuntimeError, 'bad') : good",
                "x ? raise('bad') : good",
            ),
            (
                "def f = raise(RuntimeError.new('bad'))",
                "def f = raise('bad')",
            ),
            (
                "-> { fail(::RuntimeError.new('bad')) }",
                "-> { fail('bad') }",
            ),
            ("raise RuntimeError, \"a\nb\"", "raise \"a\nb\""),
            (
                "raise RuntimeError.new(\"x\") # keep\r\np __LINE__\r\n__END__\r\npayload",
                "raise \"x\" # keep\r\np __LINE__\r\n__END__\r\npayload",
            ),
        ] {
            crate::cops::testing::assert_correction(&RedundantException, before, after);
        }
    }
    #[test]
    fn every_pinned_correction_is_canonical_or_an_individual_safety_hold() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_exception_pinned.json"
        ))
        .unwrap();
        let (mut fixed, mut withheld) = (0, 0);
        for case in cases {
            let Some(after) = case["correction"].as_str() else {
                continue;
            };
            let before = case["source"].as_str().unwrap();
            let offenses = correction_offenses(&RedundantException, before);
            assert_eq!(offenses.len(), 1);
            if offenses[0].correction.is_some() {
                crate::cops::testing::assert_correction(&RedundantException, before, after);
                fixed += 1;
            } else {
                assert!(
                    before.contains("#{message}")
                        || before.contains("`command`")
                        || before.contains("Object.new")
                        || before.contains("variable")
                        || before.contains("1 + 2")
                        || before.contains("a || b")
                        || before.contains("foo.bar baz")
                        || before.contains("!foo")
                        || before.contains("foo[1]"),
                    "unexplained hold: {before}"
                );
                withheld += 1;
            }
        }
        assert_eq!((fixed, withheld), (16, 11));
    }
    #[test]
    fn all_applicable_pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_exception_pinned.json"
        ))
        .unwrap();
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found = correction_offenses(&RedundantException, source);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            let expected: Vec<Vec<u32>> =
                serde_json::from_value(case["positions"].clone()).unwrap();
            assert_eq!(positions, expected, "{source}");
            let messages: Vec<_> = found.iter().map(|o| o.message.as_str()).collect();
            let expected: Vec<String> = serde_json::from_value(case["messages"].clone()).unwrap();
            assert_eq!(messages, expected, "{source}");
        }
    }
}
