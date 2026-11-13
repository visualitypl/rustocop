//! `Style/SignalException` — flags `fail` used to signal an exception:
//! rustocop's fixed style is `raise` (RuboCop's `only_raise`).
//!
//! Matches RuboCop's semantics for that style: a receiverless `fail`, or
//! `Kernel.fail` / `::Kernel.fail`, is reported at the method name, unless
//! the file defines its own `fail` method (`def fail` or `def obj.fail`
//! anywhere), in which case `fail` may be that method. Calls on any other
//! receiver, and safe-navigation calls, are accepted.

use ruby_prism::{CallNode, DefNode, Node, Visit};

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The `Style/SignalException` cop.
pub struct SignalException;

impl Cop for SignalException {
    fn name(&self) -> &'static str {
        "Style/SignalException"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Whether any `fail` call offends depends on the whole file (a custom
        // `fail` method), so one walk from the root collects both.
        let Some(program) = node.as_program_node() else {
            return;
        };
        if !file.contains(b"fail") {
            return;
        }
        let mut search = FailSearch {
            custom_fail_defined: false,
            calls: Vec::new(),
        };
        search.visit_program_node(&program);
        if search.custom_fail_defined {
            return;
        }
        for offset in search.calls {
            let (line, col) = file.line_col(offset);
            out.push(Offense {
                correction: Some(Correction {
                    range: offset..offset + 4,
                    replacement: "raise".into(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: "Always use `raise` to signal exceptions.".to_string(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

/// Collects `fail` calls and whether the file defines a `fail` method.
struct FailSearch {
    custom_fail_defined: bool,
    /// Start offsets of the offending `fail` selectors.
    calls: Vec<usize>,
}

impl<'pr> Visit<'pr> for FailSearch {
    fn visit_def_node(&mut self, node: &DefNode<'pr>) {
        if node.name().as_slice() == b"fail" {
            self.custom_fail_defined = true;
        }
        ruby_prism::visit_def_node(self, node);
    }

    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if is_kernel_fail(node)
            && let Some(selector) = node.message_loc()
        {
            self.calls.push(selector.start_offset());
        }
        ruby_prism::visit_call_node(self, node);
    }
}

/// A `fail` with no receiver, or on `Kernel` / `::Kernel` (not `&.`).
fn is_kernel_fail(call: &CallNode<'_>) -> bool {
    if call.name().as_slice() != b"fail" || call.is_safe_navigation() {
        return false;
    }
    match call.receiver() {
        None => true,
        Some(receiver) => crate::cops::ast::is_global_const(&receiver, &["Kernel"]),
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/style/signal_exception_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the
    //! `only_raise` style.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SignalException")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    #[test]
    fn corrects_fail_selectors_only() {
        // RuboCop expect_correction examples (MIT), only_raise style.
        for (before, after) in [
            (
                "begin\n  fail\nrescue Exception\n  #do nothing\nend\n",
                "begin\n  raise\nrescue Exception\n  #do nothing\nend\n",
            ),
            ("Kernel.fail\n", "Kernel.raise\n"),
            (
                "::Kernel.fail(\"fail\") # fail\r\n",
                "::Kernel.raise(\"fail\") # fail\r\n",
            ),
            ("fail unless ready\n", "raise unless ready\n"),
        ] {
            crate::cops::testing::assert_correction(&SignalException, before, after);
        }
    }

    #[test]
    fn flags_fail_with_message() {
        let found = offenses("def test\n  fail\nrescue Exception\n  #do nothing\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 3));
        assert_eq!(found[0].message, "Always use `raise` to signal exceptions.");
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        assert_eq!(
            positions("begin\n  fail\nrescue Exception\n  #do nothing\nend\n"),
            [(2, 3)]
        );
        assert_eq!(
            positions("begin\n  raise\nrescue Exception\n  fail\nend\n"),
            [(4, 3)]
        );
        assert_eq!(positions("Kernel.fail\n"), [(1, 8)]);
    }

    #[test]
    fn flags_other_fail_forms() {
        assert_eq!(positions("::Kernel.fail\n"), [(1, 10)]);
        assert_eq!(positions("fail ArgumentError, \"bad\"\n"), [(1, 1)]);
        assert_eq!(positions("fail(\"bad\")\n"), [(1, 1)]);
        assert_eq!(positions("x = 1 if y\nfail unless x\n"), [(2, 1)]);
        assert_eq!(
            positions("foo { fail }\nbar do\n  fail\nend\n"),
            [(1, 7), (3, 3)]
        );
    }

    #[test]
    fn accepts_rubocop_spec_examples() {
        for source in [
            "class A\n  def fail(arg)\n  end\n  def other_method\n    fail \"message\"\n  end\nend\n",
            "class A\n  def self.fail(arg)\n  end\n  def self.other_method\n    fail \"message\"\n  \
             end\nend\n",
            "test.fail\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn accepts_lookalikes() {
        for source in [
            "raise\n",
            "raise ArgumentError\n",
            "Kernel.raise\n",
            "Kernel&.fail\n",
            "Foo::Kernel.fail\n",
            "self.fail\n",
            "failure\n",
            "fail = 1\nfail\n",
            ":fail\n",
            "\"fail\"\n",
            // A custom `fail` defined after its use still counts.
            "fail \"x\"\ndef fail(message)\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should accept {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("fail # rubocop:disable Style/SignalException\n").is_empty());
    }
}
