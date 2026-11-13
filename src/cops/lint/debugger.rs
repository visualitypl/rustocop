//! `Lint/Debugger` — flags debugger entry points left in source.
//!
//! Covers RuboCop's default `DebuggerMethods` and `DebuggerRequires`, with
//! fixed lists rather than configuration parameters. Receivers are matched
//! structurally, including `Kernel` / `::Kernel` and `Pry` / `::Pry`.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/Debugger` cop.
pub struct Debugger;

/// Debugger methods called on `binding`.
const BINDING_METHODS: [&[u8]; 7] = [
    b"pry",
    b"remote_pry",
    b"pry_remote",
    b"console",
    b"irb",
    b"b",
    b"break",
];

/// Debugger methods called with no receiver.
const BARE_METHODS: [&[u8]; 5] = [b"byebug", b"remote_byebug", b"pry", b"debugger", b"jard"];

/// Capybara methods called bare or on `page`.
const CAPYBARA_METHODS: [&[u8]; 4] = [
    b"save_and_open_page",
    b"save_and_open_screenshot",
    b"save_page",
    b"save_screenshot",
];

impl Cop for Debugger {
    fn name(&self) -> &'static str {
        "Lint/Debugger"
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
        if call.is_safe_navigation() || (!debugger_method(&call) && !debugger_require(&call)) {
            return;
        }
        if assumed_usage_context(&call, cx) {
            return;
        }
        // Prism includes a literal block in the call's location; RuboCop's
        // send source stops before it (but includes a block argument).
        let end = ast::source_end(node);
        let start = call.location().start_offset();
        let entry_point = super::ambiguous_block_association::reference_source(
            &call.location().as_slice()[..end - start],
            file,
        );
        let (line, col) = file.line_col(call.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Remove debugger entry point `{entry_point}`."),
            line,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            col,
        });
    }
}

fn assumed_usage_context(call: &CallNode<'_>, cx: &NodeContext<'_, '_>) -> bool {
    // A call with a literal block has a synthetic block parent in RuboCop.
    // Its arguments are still checked normally, but the call itself is not
    // assumed to be a value, even when another call consumes it.
    if !ast::call_arguments(call).is_empty() || has_literal_block(call) {
        return false;
    }

    let mut has_call = false;
    let mut has_boundary = false;
    let call_node = call.as_node();
    let mut child = &call_node;
    for ancestor in cx.ancestors() {
        has_call |= call_parent(ancestor, child);
        if let Some(call) = ancestor.as_call_node() {
            has_boundary |= has_literal_block(&call);
        }
        has_boundary |= matches!(ancestor, Node::BlockNode { .. } | Node::LambdaNode { .. })
            || ancestor
                .as_begin_node()
                .is_some_and(|begin| begin.begin_keyword_loc().is_some());
        child = ancestor;
    }
    if !has_call {
        return false;
    }

    // Prism's argument list is transparent in RuboCop's AST.
    let assumed_argument = cx
        .ancestors()
        .find(|ancestor| ancestor.as_arguments_node().is_none())
        .is_some_and(|parent| {
            call_parent(parent, &call_node)
                || ast::is_literal(parent)
                || parent.as_assoc_node().is_some()
        });
    assumed_argument || !has_boundary
}

/// Whether this Prism edge represents a RuboCop send/csend ancestor.
fn call_parent(parent: &Node<'_>, child: &Node<'_>) -> bool {
    // Prism nests a block under its call; RuboCop makes the send and block
    // body siblings. That send is not an ancestor of anything in the block.
    if parent.as_call_node().is_some() {
        return child.as_block_node().is_none();
    }
    // Compound assignments and multiple-assignment targets have a synthetic
    // send on the left-hand side in RuboCop. The RHS is not its descendant.
    let value = match parent {
        Node::CallAndWriteNode { .. } => parent.as_call_and_write_node().unwrap().value(),
        Node::CallOrWriteNode { .. } => parent.as_call_or_write_node().unwrap().value(),
        Node::CallOperatorWriteNode { .. } => parent.as_call_operator_write_node().unwrap().value(),
        Node::IndexAndWriteNode { .. } => parent.as_index_and_write_node().unwrap().value(),
        Node::IndexOrWriteNode { .. } => parent.as_index_or_write_node().unwrap().value(),
        Node::IndexOperatorWriteNode { .. } => {
            parent.as_index_operator_write_node().unwrap().value()
        }
        Node::CallTargetNode { .. } | Node::IndexTargetNode { .. } => return true,
        _ => return false,
    };
    child.location().end_offset() <= value.location().start_offset()
}

fn has_literal_block(call: &CallNode<'_>) -> bool {
    call.block()
        .is_some_and(|block| block.as_block_node().is_some())
}

fn debugger_require(call: &CallNode<'_>) -> bool {
    if call.name().as_slice() != b"require" {
        return false;
    }
    let arguments = ast::call_arguments(call);
    let [argument] = arguments.as_slice() else {
        return false;
    };
    argument
        .as_string_node()
        .is_some_and(|string| matches!(string.unescaped(), b"debug/open" | b"debug/start"))
}

fn debugger_method(call: &CallNode<'_>) -> bool {
    let name = call.name().as_slice();
    let Some(receiver) = call.receiver() else {
        // Shadowed locals are LocalVariableReadNodes, not calls.
        return BARE_METHODS.contains(&name) || CAPYBARA_METHODS.contains(&name);
    };
    if matches!(name, b"byebug" | b"remote_byebug" | b"debugger") {
        return ast::is_global_const(&receiver, &["Kernel"]);
    }
    if name == b"rescue" {
        return ast::is_global_const(&receiver, &["Pry"]);
    }
    let Some(receiver) = receiver.as_call_node() else {
        return false;
    };
    if receiver.is_safe_navigation() {
        return false;
    }
    if CAPYBARA_METHODS.contains(&name) {
        return receiver.name().as_slice() == b"page" && receiver.receiver().is_none();
    }
    BINDING_METHODS.contains(&name)
        && receiver.name().as_slice() == b"binding"
        && receiver.receiver().is_none_or(|namespace| {
            name != b"console" && ast::is_global_const(&namespace, &["Kernel"])
        })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/lint/debugger_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/Debugger")
            .collect()
    }

    #[test]
    fn flags_binding_pry() {
        let found = offenses("def show\n  binding.pry\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 2);
        assert_eq!(found[0].col, 3);
        assert_eq!(
            found[0].message,
            "Remove debugger entry point `binding.pry`."
        );
    }

    fn assert_entry_point(source: &str, entry_point: &str) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "should flag {source}");
        assert_eq!((found[0].line, found[0].col), (2, 3), "{source}");
        assert_eq!(
            found[0].message,
            format!("Remove debugger entry point `{entry_point}`."),
            "{source}"
        );
    }

    #[test]
    fn ignores_no_argument_calls_used_by_another_call() {
        for method in [
            "byebug",
            "pry",
            "binding.pry",
            "Kernel.byebug",
            "page.save_page",
        ] {
            for source in [
                format!("consume({method})"),
                format!("object&.consume({method})"),
                format!("{method}.inspect"),
                format!("{method}&.inspect"),
                format!("self.value = {method}"),
                format!("it {{ consume({method}) }}"),
                format!("it {{ consume(k: {method}) }}"),
                format!("it {{ consume([k, {method}]) }}"),
            ] {
                assert!(offenses(&source).is_empty(), "should not flag {source}");
            }
        }
    }

    #[test]
    fn ignores_indirect_usage_without_block_or_begin_boundaries() {
        for source in [
            "consume((byebug))",
            "consume(x = byebug)",
            "consume(byebug ? x : y)",
            "consume(byebug && x)",
            "consume(defined?(byebug))",
            "consume(*byebug)",
            "consume(&byebug)",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source}");
        }
    }

    #[test]
    fn preserves_findings_in_blocks_lambdas_and_begin_end() {
        // Custom-method specs, using the fixed default `byebug` instead.
        for (source, line, col) in [
            ("x.y = do_something { byebug }", 1, 22),
            ("x.y = do_something do\n  z(_1)\n  byebug\nend", 3, 3),
            ("x.y = do_something do\n  z(it)\n  byebug\nend", 3, 3),
            ("x.y = -> { byebug }", 1, 12),
            ("x.y = lambda { byebug }", 1, 16),
            ("x.y = proc { byebug }", 1, 14),
            ("x.y = Proc.new { byebug }", 1, 18),
            ("do_something(\n  begin\n    byebug\n  end\n)", 3, 5),
            // A block's call is a sibling of its body in RuboCop, not an
            // ancestor: a literal parent alone must not suppress a finding.
            ("consume { [byebug] }", 1, 12),
            ("consume { { k: byebug } }", 1, 16),
            ("consume { x = byebug }", 1, 15),
            ("consume { (byebug).inspect }", 1, 12),
            ("consume { consume((byebug)) }", 1, 20),
            ("consume { consume(*byebug) }", 1, 20),
            ("consume { consume(&byebug) }", 1, 20),
            ("consume { \"#{byebug}\" }", 1, 14),
            ("consume(-> { byebug })", 1, 14),
            ("consume(->(x = byebug) { x })", 1, 16),
            // A debugger call with its own literal block has a block parent.
            ("consume(byebug { foo })", 1, 9),
            ("byebug { foo }.inspect", 1, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "should flag {source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
            assert_eq!(
                found[0].message, "Remove debugger entry point `byebug`.",
                "{source}"
            );
        }
    }

    #[test]
    fn ignores_assignment_receivers_but_not_compound_assignment_values() {
        for assignment in ["||=", "&&=", "+="] {
            for source in [
                format!("byebug.value {assignment} x"),
                format!("byebug[0] {assignment} x"),
                format!("object[byebug] {assignment} x"),
                format!("consume {{ byebug.value {assignment} x }}"),
                format!("consume {{ object[byebug] {assignment} x }}"),
            ] {
                assert!(offenses(&source).is_empty(), "should not flag {source}");
            }
            let source = format!("object.value {assignment} byebug");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            let col = if assignment == "+=" { 17 } else { 18 };
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(found[0].message, "Remove debugger entry point `byebug`.");
        }
        for source in [
            "byebug.value, other = values",
            "object[byebug], other = values",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source}");
        }
    }

    #[test]
    fn preserves_argument_bearing_calls_used_as_values() {
        for (source, entry, col) in [
            ("consume(byebug(foo))", "byebug(foo)", 9),
            ("consume(pry foo)", "pry foo", 9),
            ("consume(byebug(&block))", "byebug(&block)", 9),
            ("consume(require 'debug/start')", "require 'debug/start'", 9),
            ("byebug(foo).inspect", "byebug(foo)", 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "should flag {source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("Remove debugger entry point `{entry}`."),
                "{source}"
            );
        }
    }

    #[test]
    fn ignores_direct_value_parents_even_across_boundaries() {
        for source in [
            "consume(begin; [byebug]; end)",
            "consume(begin; { k: pry }; end)",
            "consume { byebug.inspect }",
            "consume { self.value = pry }",
            "consume(proc { [byebug] })",
            "consume { consume(byebug) { foo } }",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source}");
        }
    }

    #[test]
    fn preserves_value_contexts_without_call_ancestors() {
        for (source, col) in [
            ("[byebug]", 2),
            ("{ k: byebug }", 6),
            ("x = byebug", 5),
            ("(byebug)", 2),
            ("super(byebug)", 7),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "should flag {source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(found[0].message, "Remove debugger entry point `byebug`.");
        }
    }

    #[test]
    fn flags_default_bare_methods_with_arguments() {
        for method in ["byebug", "remote_byebug", "pry", "debugger", "jard"] {
            for suffix in ["", " foo", "(foo)"] {
                let call = format!("{method}{suffix}");
                assert_entry_point(&format!("def example\n  {call}\nend\n"), &call);
            }
        }
    }

    #[test]
    fn flags_all_default_methods() {
        for source in [
            "binding.irb",
            "binding.pry",
            "binding.remote_pry",
            "binding.pry_remote",
            "binding.console",
            "byebug",
            "debugger",
            "remote_byebug",
            "pry",
            "jard",
        ] {
            assert_eq!(offenses(source).len(), 1, "should flag {source}");
        }
    }

    #[test]
    fn flags_default_binding_receivers() {
        for method in ["irb", "b", "break", "pry", "remote_pry", "pry_remote"] {
            for receiver in ["binding", "Kernel.binding", "::Kernel.binding"] {
                for suffix in ["", " foo"] {
                    let call = format!("{receiver}.{method}{suffix}");
                    assert_entry_point(&format!("def example\n  {call}\nend\n"), &call);
                }
            }
        }
        assert_entry_point("def example\n  binding.console\nend\n", "binding.console");
    }

    #[test]
    fn flags_default_kernel_receivers() {
        for method in ["byebug", "remote_byebug", "debugger"] {
            for receiver in ["Kernel", "::Kernel"] {
                let call = format!("{receiver}.{method}");
                assert_entry_point(&format!("def example\n  {call}\nend\n"), &call);
            }
        }
    }

    #[test]
    fn flags_capybara_methods() {
        for method in [
            "save_and_open_page",
            "save_and_open_screenshot",
            "save_page",
            "save_screenshot",
        ] {
            for receiver in ["", "page."] {
                for suffix in ["", " foo"] {
                    let call = format!("{receiver}{method}{suffix}");
                    assert_entry_point(&format!("def example\n  {call}\nend\n"), &call);
                }
            }
        }
    }

    #[test]
    fn flags_pry_rescue_without_quoting_the_block() {
        for receiver in ["Pry", "::Pry"] {
            let call = format!("{receiver}.rescue");
            assert_entry_point(&format!("def example\n  {call} {{ puts 1 }}\nend\n"), &call);
        }
        assert!(offenses("begin\nrescue StandardError\nend\n").is_empty());
    }

    #[test]
    fn ignores_receiver_lookalikes() {
        for source in [
            "code.byebug",
            "code.remote_byebug",
            "code.debugger",
            "code.pry",
            "code.remote_pry",
            "code.pry_remote",
            "code.irb",
            "code.console",
            "code.jard",
            "code.save_and_open_page",
            "code.save_and_open_screenshot",
            "code.save_page",
            "code.save_screenshot",
            "code.rescue",
            "code.binding.pry",
            "self.binding.irb",
            "Other.binding.b",
            "Kernel.binding.console",
            "Kernel.pry",
            "Other::Kernel.debugger",
            "Other::Pry.rescue",
            "page = Object.new; page.save_page",
            "binding = Object.new; binding.pry",
            "console",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source}");
        }
    }

    #[test]
    fn flags_default_debugger_requires() {
        for path in ["debug/start", "debug/open"] {
            for call in [
                format!("require '{path}'"),
                format!("require(\"{path}\")"),
                format!("Kernel.require '{path}'"),
                format!("::Kernel.require '{path}'"),
                format!("loader.require '{path}'"),
                format!("require %q({path})"),
            ] {
                assert_entry_point(&format!("# debugger require\n  {call}\n"), &call);
            }
        }
        assert_entry_point(
            "# escaped path\n  require \"debug\\x2fstart\"\n",
            "require \"debug\\x2fstart\"",
        );
    }

    #[test]
    fn ignores_non_debugger_requires() {
        for source in [
            "require",
            "require 'debug'",
            "require 'debug/session'",
            "require 'debug/start_extra'",
            "require 'pry'",
            "require 'byebug'",
            "require 'debug/start', 'other'",
            "require 'debug/start', &block",
            "require path",
            "require :debug",
            "require *['debug/start']",
            "require \"debug/#{mode}\"",
            "require_relative 'debug/start'",
            "autoload :Debug, 'debug/start'",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source}");
        }
    }

    #[test]
    fn ignores_debugger_names_in_comments_strings_and_definitions() {
        assert!(
            offenses(
                "# Kernel.remote_byebug\n# binding.break\n# page.save_page\n\
             # require 'debug/start'\n'Pry.rescue'\n\
             def jard; end\ndef debugger; end\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn ignores_safe_navigation_dispatch_and_receiver_chain_lookalikes() {
        for source in [
            "binding&.pry",
            "page&.save_page",
            "Kernel&.binding&.pry",
            "Kernel&.binding.pry",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("binding.pry");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(
            found[0].message,
            "Remove debugger entry point `binding.pry`."
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "binding.foo\n",
            "remote_pry\n",
            "other.pry\n",
            "binding.pryer\n",
            "my_debugger = 1; my_debugger\n",
            "save_file\n",
            "x = debugger_like\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source}");
        }
    }

    #[test]
    fn ignores_local_variable_shadowing() {
        // `debugger` bound as a local variable is a variable read, not a call.
        assert!(offenses("debugger = 1\nx = debugger\n").is_empty());
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("# rustocop:disable Lint/Debugger\nbinding.pry\n").is_empty());
        assert!(offenses("# rubocop:disable Lint/Debugger\nbinding.pry\n").is_empty());
        assert!(offenses("# rustocop:disable Lint\nbinding.pry\n").is_empty());
        assert!(offenses("# rustocop:disable all\nbinding.pry\n").is_empty());
    }
}
