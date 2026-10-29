//! `Style/MixinGrouping` — fixed separated declarations.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashSet;

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
                    .then(|| correction(&call, cx, file))
                    .flatten(),
            });
        }
    }
}

/// Correction-only proof, built once from the main AST. We deliberately accept
/// only a closed top-level declaration-only file: no user callbacks, superclass
/// hooks, metaprogramming, deferred code, autoloads, or unknown evaluations.
/// Arguments must resolve to earlier, uniquely declared, empty modules. Splitting
/// then preserves validation atomicity, dispatch, return self and ancestor order.
/// External monkey patches/preloaded project constants cannot be inferred.
#[derive(Default)]
pub(crate) struct Bindings {
    proven: HashSet<usize>,
    #[cfg(test)]
    pub(crate) visited: usize,
}
impl Bindings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut result = Self::default();
        let Some(program) = root.as_program_node() else {
            return result;
        };
        let mut names = HashSet::new();
        let mut empty_modules = HashSet::new();
        for node in program.statements().body().iter() {
            #[cfg(test)]
            {
                result.visited += 1;
            }
            let (path, body, is_module) = if let Some(module) = node.as_module_node() {
                (module.constant_path(), module.body(), true)
            } else if let Some(class) = node.as_class_node() {
                if class.superclass().is_some() {
                    return Self::default();
                }
                (class.constant_path(), class.body(), false)
            } else {
                return Self::default();
            };
            // Only unqualified root declarations: nested/qualified lookup and
            // reopened/singleton visibility are intentionally not inferred.
            let Some(name) = path.as_constant_read_node() else {
                return Self::default();
            };
            let name = name.name().as_slice().to_vec();
            if core_namespace(&name) || !names.insert(name.clone()) {
                return Self::default();
            }
            let children = super::module_function::body_elements(body);
            if children.is_empty() && is_module {
                empty_modules.insert(name.clone());
            }
            for child in children {
                #[cfg(test)]
                {
                    result.visited += 1;
                }
                let Some(call) = child.as_call_node() else {
                    return Self::default();
                };
                let args = ast::call_arguments(&call);
                if !super::mixin_usage::mixin_call(&call)
                    || args.is_empty()
                    || !args.iter().all(|arg| {
                        arg.as_constant_read_node().is_some_and(|arg| {
                            let arg_name = arg.name().as_slice();
                            arg_name != name && empty_modules.contains(arg_name)
                        })
                    })
                {
                    return Self::default();
                }
                if args.len() > 1 {
                    result.proven.insert(call.location().start_offset());
                }
            }
        }
        result
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
fn correction(
    call: &ruby_prism::CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let start = call.location().start_offset();
    let end = call.location().end_offset();
    if !file
        .mixin_grouping_bindings(cx.ancestors().last()?)
        .proven
        .contains(&start)
    {
        return None;
    }
    if file.bytes[start..end]
        .iter()
        .any(|b| matches!(b, b'\n' | b'\r' | b'#'))
    {
        return None;
    }
    let line = file.line_of(start);
    let indent = &file.bytes[file.line_start(line)..start];
    let tail = &file.bytes[end..file.line_start(line + 1)];
    if !indent.iter().all(|b| matches!(b, b' ' | b'\t'))
        || !tail.iter().all(u8::is_ascii_whitespace)
        || super::correction::touches_directive(file, start..end)
    {
        return None;
    }
    let newline = if tail.ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let indent = std::str::from_utf8(indent).ok()?;
    let method = std::str::from_utf8(call.name().as_slice()).ok()?;
    let mut replacement = String::new();
    for (i, arg) in ast::call_arguments(call).into_iter().rev().enumerate() {
        if i > 0 {
            replacement.push_str(newline);
            replacement.push_str(indent);
        }
        replacement.push_str(method);
        replacement.push_str(if call.opening_loc().is_some() {
            "("
        } else {
            " "
        });
        replacement.push_str(std::str::from_utf8(arg.location().as_slice()).ok()?);
        if call.opening_loc().is_some() {
            replacement.push(')');
        }
    }
    let correction = Correction {
        range: start..end,
        replacement,
    };
    super::correction::preserves_line_values(file, &correction).then_some(correction)
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
    fn withholds_each_unproven_pinned_correction_without_hiding_the_offense() {
        // All six applicable upstream correction expectations: unresolved
        // constants might autoload, fail validation, or have custom callbacks.
        for body in [
            "include Bar, Qux",
            "include Bar, Baz\ninclude Qux",
            "include(Bar, Module.new do\n  do_something\nend)",
            "extend Bar, Qux",
            "prepend Bar, Qux",
            "include Bar, Baz\nextend Qux",
        ] {
            let source = format!("class Foo\n{body}\nend\n");
            let found = crate::cops::testing::correction_offenses(&super::MixinGrouping, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn uncertain_bindings_callbacks_values_and_byte_sensitive_edits_are_withheld() {
        let prefix = "module Bar; end\nmodule Qux; end\n";
        for source in [
            "class Foo; include Bar, Qux; end".into(),
            format!("{prefix}class Foo < Parent\n  include Bar, Qux\nend"),
            format!(
                "{prefix}class Foo\n  def self.include(*args); super; end\n  include Bar, Qux\nend"
            ),
            "module Bar; def self.included(base); base.const_set(:Qux, 1); end; end\nmodule Qux; end\nclass Foo\n  include Bar, Qux\nend".into(),
            format!("{prefix}class Foo\n  include Bar, Qux\nend\nmodule Bar; end"),
            format!("{prefix}module Outer\n  class Foo\n    include Bar, Qux\n  end\nend"),
            format!("{prefix}module Outer::Foo\n  include Bar, Qux\nend"),
            format!("{prefix}class Foo\n  include Bar, 1\nend"),
            format!("{prefix}class Foo\n  include Bar, get_module\nend"),
            format!("{prefix}class Foo\n  include Bar, Qux # comment\nend"),
            format!("{prefix}class Foo\n  include Bar, Qux; nil\nend"),
            format!("{prefix}class Foo\n  include Bar, Qux\nend\np __LINE__"),
            "module Enumerable; end\nmodule Qux; end\nclass Foo\n  include Enumerable, Qux\nend".into(),
            format!("{prefix}class Object\n  include Bar, Qux\nend"),
            format!("{prefix}class Foo\n  include ::Bar, Qux\nend"),
            format!("{prefix}class Foo\n  include(Bar, Qux, &block)\nend"),
            format!("{prefix}class Foo\n  include(Bar,\n    Qux)\nend"),
            format!("{prefix}class Foo\n  include Bar, # keep\n    Qux\nend"),
        ] {
            let found = crate::cops::testing::correction_offenses(&super::MixinGrouping, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn core_class_reopenings_are_not_proven_fresh_namespaces() {
        for name in [
            "Encoding",
            "Array",
            "String",
            "IO",
            "Thread",
            "Enumerator",
            "Regexp",
            "Exception",
            "Random",
            "Struct",
            "Fiber",
            "Ractor",
            "Data",
            "Set",
        ] {
            let source = format!(
                "module Converter; end\nmodule Bar; end\nclass {name}\n  include Converter, Bar\nend\n"
            );
            let found = crate::cops::testing::correction_offenses(&super::MixinGrouping, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn first_immediate_begin_in_superclass_precedes_class_body() {
        for source in [
            "class C < (include A, B; Object)\nend\n",
            "class C < (include A, B)\nend\n",
            "class C < (include A, B; Object)\n  extend D, E\nend\n",
            "class C < (include A, B; Object)\n  extend D, E\n  other\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 12), "{source}");
            assert_eq!(
                found[0].message,
                "Put `include` mixins in separate statements."
            );
            let found = crate::cops::testing::correction_offenses(&super::MixinGrouping, source);
            assert!(found[0].correction.is_none(), "{source}");
        }
        // Only one begin layer is selected, not a recursive superclass walk.
        for source in [
            "class C < ((include A, B; Object))\nend\n",
            "class C < ((include A, B; Object))\n  extend D, E\n  other\nend\n",
            "class C < (Object)\n  extend D, E\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
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
