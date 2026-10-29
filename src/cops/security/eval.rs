//! `Security/Eval` — dynamic arguments to `Kernel#eval` and `Binding#eval`.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// The `Security/Eval` cop. Detection only: evaluating source is never safe to fix.
pub struct Eval;

impl Cop for Eval {
    fn name(&self) -> &'static str {
        "Security/Eval"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"eval" || call.is_safe_navigation() {
            return;
        }
        if call.receiver().is_some_and(|receiver| {
            !ast::is_global_const(&receiver, &["Kernel"])
                && !receiver.as_call_node().is_some_and(|binding| {
                    binding.name().as_slice() == b"binding"
                        && binding.receiver().is_none()
                        && binding.arguments().is_none()
                        && binding.block().is_none()
                })
        }) {
            return;
        }
        let Some(code) = ast::call_arguments(&call).into_iter().next() else {
            return;
        };
        // For the literal types modeled by Prism, the shared recursive-basic
        // predicate also implements recursive_literal?: composite types are
        // always recursively checked in both upstream predicates.
        if code.as_string_node().is_some()
            || code.as_source_file_node().is_some()
            || (code.as_interpolated_string_node().is_some()
                && ast::is_recursive_basic_literal(&code))
        {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "The use of `eval` is a serious security risk.".into(),
            line,
            end_line: line,
            col,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/security/eval_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    //! All fixtures are source-only; never execute eval inputs.

    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Security/Eval")
            .collect()
    }

    #[test]
    fn covers_all_pinned_source_documentation_examples() {
        // Also port the seven source-file examples (same upstream MIT license).
        for source in [
            "eval(something)",
            "binding.eval(something)",
            "Kernel.eval(something)",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "obj.public_send(method_name)",
            "obj.send(method_name, *args)",
            "eval(\"1 + 1\")",
            "binding.eval(\"foo\")",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn accepts_all_pinned_no_offense_examples() {
        for source in [
            "eval = something",
            "something.eval",
            "eval(\"puts 1\")",
            "eval",
            "eval \"something\\nsomething2\"",
            "eval \"something#{2}\"",
            "eval(\"puts 1\", binding, \"test.rb\", 1)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn matches_only_the_pinned_receivers() {
        for source in [
            "self.eval(code)",
            "object.eval(code)",
            "Binding.eval(code)",
            "Other::Kernel.eval(code)",
            "self.binding.eval(code)",
            "Kernel.binding.eval(code)",
            "::Kernel.binding.eval(code)",
            "binding(arg).eval(code)",
            "binding(&block).eval(code)",
            "binding { nil }.eval(code)",
            "(binding).eval(code)",
            "Kernel&.eval(code)",
            "binding&.eval(code)",
            "binding = object; binding.eval(code)",
            "eval = object; eval",
            "object.instance_eval(code)",
            "object.class_eval(code)",
            "send(:eval, code)",
            "# eval(code)\n'Kernel.eval(code)'",
            "def eval(code); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "binding().eval(code)",
            "Kernel::eval(code)",
            "::Kernel::eval(code)",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn accepts_only_plain_strings_or_recursive_literal_interpolation() {
        for source in [
            "eval('')",
            "eval(%q(puts 1))",
            "eval(?a)",
            "eval(__FILE__)",
            "eval('a' 'b')",
            "eval(\"a\" \"#{2}\")",
            "eval(<<~CODE)\n  puts 1\nCODE\n",
            "eval(<<~CODE)\n  #{2}\nCODE\n",
            "eval(\"#{nil}#{true}#{false}#{:symbol}#{1.2}#{1r}#{2i}\")",
            "eval(\"#{[1, {a: 2}, (1..3), /a/]}\")",
            "eval(\"#{(1 == 2) || (3 * 4)}#{!nil}#{1 <=> 2}\")",
            "eval(\"#{1; 2}\")",
            "eval(\"#{'a' \"#{2}\"}\")",
            // Pinned recursive_literal? regards literal command strings as
            // literals too. This is syntactic parity, not a safety guarantee.
            "eval(\"#{`never_execute_this`}\")",
            "eval(\"#{__FILE__}#{__LINE__}\")",
            "eval(\"literal\", dynamic_binding, filename, line)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "eval(:symbol)",
            "eval(nil)",
            "eval(123)",
            "eval(__LINE__)",
            "eval(__ENCODING__)",
            "eval(['literal'])",
            "eval('a' + 'b')",
            "eval(('literal'))",
            "eval(\"#{foo}\")",
            "eval(\"#@code\")",
            "eval(\"#{[foo]}\")",
            "eval(\"#{1 + 2}\")",
            "eval(\"#{1&.==(2)}\")",
            "eval(\"#{__ENCODING__}\")",
            "eval(\"#{begin; 1; end}\")",
            "eval(<<~CODE)\n  #{code}\nCODE\n",
            "eval(*args)",
            "eval(**kwargs)",
            "eval(code: 'literal')",
            "eval(&block)",
            "eval(code) { :block }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn selector_highlight_is_independent_of_multiline_arguments_and_blocks() {
        for (source, line, col) in [
            (
                "# source only\n  Kernel\n    .eval(\n      code\n    ) { foo }\n",
                3,
                6,
            ),
            ("\"é\"; binding.eval(code)", 1, 14),
            ("eval(<<~CODE)\n#{code}\nCODE\n", 1, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line),
                "{source}"
            );
        }
        for prefix in ["rustocop", "rubocop"] {
            // The argument/body is outside the selector-only highlight.
            let source = format!("eval(\n  code # {prefix}:disable Security/Eval\n)\n");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }

    #[test]
    fn bounded_sibling_calls_report_each_selector_once() {
        let source = "object.eval(code)\neval(\"#{2}\")\nbinding.eval(code)\n".repeat(1000);
        let found = offenses(&source);
        assert_eq!(found.len(), 1000);
        for (index, offense) in found.iter().enumerate() {
            assert_eq!((offense.line, offense.col), ((index as u32 + 1) * 3, 9));
            assert!(offense.correction.is_none());
        }
    }

    #[test]
    fn flags_pinned_dynamic_arguments_at_the_selector() {
        for (source, col) in [
            ("eval(something)", 1),
            ("eval something", 1),
            ("binding.eval something", 9),
            ("Kernel.eval something", 8),
            ("::Kernel.eval something", 10),
            ("eval \"something#{foo}\"", 1),
            ("eval(something, binding, \"test.rb\", 1)", 1),
            ("eval something, binding, \"test.rb\", 1", 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                "The use of `eval` is a serious security risk."
            );
            assert_eq!(found[0].severity, crate::offense::Severity::Error);
            assert!(found[0].correction.is_none());
        }
    }
}
