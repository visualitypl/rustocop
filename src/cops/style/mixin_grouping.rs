//! `Style/MixinGrouping` — fixed separated declarations.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MixinGrouping;
impl Cop for MixinGrouping {
    fn name(&self) -> &'static str {
        "Style/MixinGrouping"
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
        let children = if let Some(class) = node.as_class_node() {
            // Upstream selects the first immediate :begin child, with the
            // superclass preceding the body. Unwrap exactly one parentheses
            // layer here: nested begins are not searched recursively.
            if let Some(group) = class.superclass().and_then(|s| s.as_parentheses_node()) {
                group
                    .body()
                    .and_then(|body| body.as_statements_node())
                    .map(|s| s.body().iter().collect())
                    .unwrap_or_default()
            } else {
                let body = class.body();
                let has_begin_body = body
                    .as_ref()
                    .and_then(Node::as_statements_node)
                    .is_some_and(|s| {
                        let nodes: Vec<_> = s.body().iter().collect();
                        nodes.len() > 1
                            || nodes
                                .first()
                                .is_some_and(|n| n.as_parentheses_node().is_some())
                    });
                let mut children = super::module_function::body_elements(body);
                // Without a begin child, inspect the direct class children.
                if !has_begin_body && let Some(superclass) = class.superclass() {
                    children.insert(0, superclass);
                }
                children
            }
        } else if let Some(module) = node.as_module_node() {
            super::module_function::body_elements(module.body())
        } else {
            return;
        };
        for child in children {
            let Some(call) = child.as_call_node() else {
                continue;
            };
            if !super::mixin_usage::mixin_call(&call)
                || crate::cops::ast::call_arguments(&call).len() < 2
            {
                continue;
            }
            let (line, col) = file.line_col(child.location().start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Put `{}` mixins in separate statements.",
                    String::from_utf8_lossy(call.name().as_slice())
                ),
                line,
                col,
                end_line: file.last_line_of(child.location()),
                correction: cx
                    .corrections_requested()
                    .then(|| correction(&call, file))
                    .flatten(),
            });
        }
    }
}

pub(super) fn core_namespace(name: &[u8]) -> bool {
    // Root Ruby core classes/modules (including deprecated integer aliases)
    // and default startup namespaces are not fresh declarations. In particular,
    // a reopened class can shadow root arguments with nested constants such as
    // Encoding::Converter, invalidating the proof of validation atomicity.
    // SyntaxSuggest is a default module/autoload (also without RubyGems);
    // ordinary RubyGems startup can load Monitor. Neither needs a project
    // preload. Keep these shared by all declaration/constructor proofs.
    // Project/preloaded namespace collisions remain outside source-only proof.
    matches!(
        name,
        b"ArgumentError"
            | b"Array"
            | b"BasicObject"
            | b"Bignum"
            | b"Binding"
            | b"Class"
            | b"ClosedQueueError"
            | b"Comparable"
            | b"Complex"
            | b"ConditionVariable"
            | b"Data"
            | b"DidYouMean"
            | b"Dir"
            | b"EOFError"
            | b"Encoding"
            | b"EncodingError"
            | b"Enumerable"
            | b"Enumerator"
            | b"Errno"
            | b"ErrorHighlight"
            | b"Exception"
            | b"FalseClass"
            | b"Fiber"
            | b"FiberError"
            | b"File"
            | b"FileTest"
            | b"Fixnum"
            | b"Float"
            | b"FloatDomainError"
            | b"FrozenError"
            | b"GC"
            | b"Gem"
            | b"Hash"
            | b"IO"
            | b"IOError"
            | b"IndexError"
            | b"Integer"
            | b"Interrupt"
            | b"Kernel"
            | b"KeyError"
            | b"LoadError"
            | b"LocalJumpError"
            | b"Marshal"
            | b"MatchData"
            | b"Math"
            | b"Method"
            | b"Module"
            | b"Monitor"
            | b"MonitorMixin"
            | b"Mutex"
            | b"NameError"
            | b"NilClass"
            | b"NoMatchingPatternError"
            | b"NoMatchingPatternKeyError"
            | b"NoMemoryError"
            | b"NoMethodError"
            | b"NotImplementedError"
            | b"Numeric"
            | b"Object"
            | b"ObjectSpace"
            | b"Proc"
            | b"Process"
            | b"Queue"
            | b"Ractor"
            | b"Random"
            | b"Range"
            | b"RangeError"
            | b"Rational"
            | b"RbConfig"
            | b"Refinement"
            | b"Regexp"
            | b"RegexpError"
            | b"RubyVM"
            | b"RuntimeError"
            | b"ScriptError"
            | b"SecurityError"
            | b"Set"
            | b"Signal"
            | b"SignalException"
            | b"SizedQueue"
            | b"StandardError"
            | b"StopIteration"
            | b"String"
            | b"Struct"
            | b"Symbol"
            | b"SyntaxError"
            | b"SyntaxSuggest"
            | b"SystemCallError"
            | b"SystemExit"
            | b"SystemStackError"
            | b"Thread"
            | b"ThreadError"
            | b"ThreadGroup"
            | b"Time"
            | b"TracePoint"
            | b"TrueClass"
            | b"TypeError"
            | b"UnboundMethod"
            | b"UncaughtThrowError"
            | b"UnicodeNormalize"
            | b"Warning"
            | b"ZeroDivisionError"
    )
}
/// RuboCop's `separate_mixins`.
fn correction(call: &ruby_prism::CallNode<'_>, file: &SourceFile) -> Option<Correction> {
    let start = call.location().start_offset();
    let line_start = file.line_start(file.line_of(start));
    let indent = " ".repeat(
        std::str::from_utf8(&file.bytes[line_start..start])
            .ok()?
            .chars()
            .count(),
    );
    let method = std::str::from_utf8(call.name().as_slice()).ok()?;
    let parenthesized = call.opening_loc().is_some_and(|o| o.as_slice() == b"(");
    let calls = ast::call_arguments(call)
        .into_iter()
        .rev()
        .map(|arg| {
            let arg = std::str::from_utf8(arg.location().as_slice()).ok()?;
            Some(if parenthesized {
                format!("{method}({arg})")
            } else {
                format!("{method} {arg}")
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Correction {
        range: start..call.location().end_offset(),
        replacement: calls.join(&format!("\n{indent}")),
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/mixin_grouping_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/MixinGrouping")
            .collect()
    }
    #[test]
    fn flags_direct_class_and_module_macros() {
        for kind in ["class", "module"] {
            for method in ["include", "extend", "prepend"] {
                for body in [
                    format!("{method} Bar, Qux"),
                    format!("{method} Bar, Baz\n  {method} Qux"),
                    format!("{method}(Bar, Module.new do\n    do_something\n  end)"),
                    format!("{method} Bar, Baz\n  extend Qux"),
                ] {
                    let s = format!("{kind} Foo\n  {body}\nend\n");
                    let found = offenses(&s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert_eq!((found[0].line, found[0].col), (2, 3));
                    assert_eq!(
                        found[0].message,
                        format!("Put `{method}` mixins in separate statements.")
                    );
                }
            }
        }
    }
    #[test]
    fn canonical_corrections_for_proven_empty_modules_preserve_parentheses_and_order() {
        for method in ["include", "extend", "prepend"] {
            for (before, after) in [
                (
                    format!("{method} Bar, Qux"),
                    format!("{method} Qux\n  {method} Bar"),
                ),
                (
                    format!("{method}(Bar, Qux)"),
                    format!("{method}(Qux)\n  {method}(Bar)"),
                ),
            ] {
                for kind in ["class", "module"] {
                    let prefix = "module Bar; end\nmodule Qux; end\n";
                    crate::cops::testing::assert_correction(
                        &super::MixinGrouping,
                        &format!("{prefix}{kind} Foo\n  {before}\nend\n"),
                        &format!("{prefix}{kind} Foo\n  {after}\nend\n"),
                    );
                }
            }
        }
    }
    #[test]
    fn safe_counterparts_of_multiple_and_mixed_pinned_calls() {
        let prefix = "module Bar; end\nmodule Baz; end\nmodule Qux; end\n";
        for (before, after) in [
            (
                "  include Bar, Baz\n  include Qux",
                "  include Baz\n  include Bar\n  include Qux",
            ),
            (
                "  include Bar, Baz\n  extend Qux",
                "  include Baz\n  include Bar\n  extend Qux",
            ),
            (
                "  prepend Bar, Baz, Qux",
                "  prepend Qux\n  prepend Baz\n  prepend Bar",
            ),
            (
                "  include Bar, Baz\n  extend Bar, Qux",
                "  include Baz\n  include Bar\n  extend Qux\n  extend Bar",
            ),
        ] {
            crate::cops::testing::assert_correction(
                &super::MixinGrouping,
                &format!("{prefix}class Foo\n{before}\nend\n"),
                &format!("{prefix}class Foo\n{after}\nend\n"),
            );
        }
    }
    #[test]
    fn block_arguments_are_arguments_and_single_body_classes_expose_superclass_macros() {
        for source in [
            "class C; include M, &block; end",
            "class C < include(M,N); end",
            "class C < include(M,N); value; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("class C < include(M,N); value; other; end").is_empty());
    }
    #[test]
    fn ignores_lookalikes_and_indirect_scopes() {
        for s in [
            "expect(foo).to include { { bar: baz } }",
            "include Bar, Qux",
            "class Foo; include; extend; prepend; end",
            "class Foo; include Bar; include Qux; end",
            "class Foo; self.include Bar, Qux; end",
            "class Foo; Other.extend Bar, Qux; end",
            "class Foo; if x; include Bar, Qux; end; end",
            "class Foo; def f; include Bar, Qux; end; end",
            "class Foo; class << self; include Bar, Qux; end; end",
            "class Foo; include(Bar, Qux) {}; end",
            "Class.new { include Bar, Qux }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        assert_eq!(offenses("module A::B; (include Bar, Qux); end").len(), 1);
        assert_eq!(
            offenses("class A; class B; include Bar, Qux; end; end").len(),
            1
        );
    }
}
