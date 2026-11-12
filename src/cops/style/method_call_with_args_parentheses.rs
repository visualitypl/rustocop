//! `Style/MethodCallWithArgsParentheses`: fixed `require_parentheses`.
//! Atomic edits retain argument association, block ownership and physical lines.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct MethodCallWithArgsParentheses;

#[cfg(test)]
thread_local! {
    static CORRECTION_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Cop for MethodCallWithArgsParentheses {
    fn name(&self) -> &'static str {
        "Style/MethodCallWithArgsParentheses"
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
        let missing = if let Some(call) = node.as_call_node() {
            !ast::is_operator_method(call.name().as_slice())
                && call.equal_loc().is_none()
                && call.opening_loc().is_none()
                && (call.arguments().is_some()
                    || call
                        .block()
                        .is_some_and(|block| block.as_block_argument_node().is_some()))
                && !(call.receiver().is_none() && macro_scope(node, cx))
        } else if let Some(yield_node) = node.as_yield_node() {
            yield_node.arguments().is_some()
                && yield_node.lparen_loc().is_none()
                && !macro_scope(node, cx)
        } else {
            false
        };
        if !missing {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use parentheses for method calls with arguments.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, end, file))
                .flatten()
                .filter(|edit| super::correction::preserves_data_positions(file, cx, edit)),
        });
    }
}

/// A single replacement is essential: the engine must never select just one
/// delimiter, or combine an outer rewrite with a stale inner-argument edit.
/// Nested commands and other overlapping fixes defer to a later explicit pass.
/// Work is local to this argument span (AST depth is bounded by the pipeline).
/// Callers must also apply `preserves_data_positions` with the main-AST context.
pub(super) fn correction(node: &Node<'_>, end: usize, file: &SourceFile) -> Option<Correction> {
    let (selector, arguments, block) = if let Some(call) = node.as_call_node() {
        (call.message_loc()?, call.arguments(), call.block())
    } else if let Some(call) = node.as_super_node() {
        (call.keyword_loc(), call.arguments(), call.block())
    } else {
        let yielded = node.as_yield_node()?;
        (yielded.keyword_loc(), yielded.arguments(), None)
    };
    let first = arguments
        .as_ref()
        .and_then(|args| args.arguments().first())
        .or(block)?;
    let start = selector.end_offset();
    #[cfg(test)]
    CORRECTION_BYTES.with(|bytes| bytes.set(bytes.get() + end - start));
    let gap = file.bytes.get(start..first.location().start_offset())?;
    // Remove only horizontal whitespace. Backslash continuations, comments,
    // CRLF and all physical newlines stay in place, preserving __LINE__.
    let trim = gap.iter().take_while(|b| matches!(b, b' ' | b'\t')).count();
    let mut body_start = start + trim;
    let merge = arguments
        .as_ref()
        .is_some_and(|args| args.arguments().len() == 1)
        && gap.iter().all(|b| matches!(b, b' ' | b'\t'))
        && first.location().end_offset() == end
        && mergeable_group(&first, file);
    if merge {
        body_start = first.as_parentheses_node()?.opening_loc().end_offset();
    }
    // Corrections own UTF-8 text. Never lossy-decode legacy argument bytes;
    // ASCII spans in legacy files and arbitrary bytes outside the span work.
    let body = std::str::from_utf8(file.bytes.get(body_start..end)?).ok()?;
    Some(Correction {
        range: start..end,
        replacement: if merge {
            format!("({body}")
        } else {
            format!("({body})")
        },
    })
}

/// A grouped argument can supply the invocation delimiters only when Ruby
/// permits its expression directly in an argument list. Keep semantic keyword
/// operators, modifiers, pattern matching and statement/assignment sequences
/// grouped, matching the pinned correction examples.
fn mergeable_group(node: &Node<'_>, file: &SourceFile) -> bool {
    let Some(group) = node.as_parentheses_node() else {
        return false;
    };
    let Some(body) = group.body().and_then(|body| body.as_statements_node()) else {
        return false;
    };
    if body.body().len() != 1 {
        return false;
    }
    let expression = body.body().first().unwrap();
    // Prism omits empty statements: even a one-expression group can contain
    // leading/trailing semicolons, which are invalid invocation delimiters.
    // Inspect only the margins, not semicolons inside the expression (strings,
    // begin/end, etc.). Conservatively retain grouping for comment semicolons.
    if [
        group.opening_loc().end_offset()..expression.location().start_offset(),
        expression.location().end_offset()..group.closing_loc().start_offset(),
    ]
    .iter()
    .any(|range| file.bytes[range.clone()].contains(&b';'))
    {
        return false;
    }
    match expression {
        Node::AndNode { .. } => {
            expression.as_and_node().unwrap().operator_loc().as_slice() == b"&&"
        }
        Node::OrNode { .. } => expression.as_or_node().unwrap().operator_loc().as_slice() == b"||",
        Node::IfNode { .. } => expression
            .as_if_node()
            .unwrap()
            .if_keyword_loc()
            .is_none_or(|loc| loc.start_offset() == expression.location().start_offset()),
        Node::UnlessNode { .. } => {
            expression
                .as_unless_node()
                .unwrap()
                .keyword_loc()
                .start_offset()
                == expression.location().start_offset()
        }
        Node::WhileNode { .. } => {
            expression
                .as_while_node()
                .unwrap()
                .keyword_loc()
                .start_offset()
                == expression.location().start_offset()
        }
        Node::UntilNode { .. } => {
            expression
                .as_until_node()
                .unwrap()
                .keyword_loc()
                .start_offset()
                == expression.location().start_offset()
        }
        Node::CallNode { .. } => !expression
            .as_call_node()
            .unwrap()
            .message_loc()
            .is_some_and(|loc| loc.as_slice() == b"not"),
        // Statement-only forms need the expression group: e.g. END is valid
        // in sink((END {})), but not directly in sink(END {}).
        Node::AliasMethodNode { .. }
        | Node::AliasGlobalVariableNode { .. }
        | Node::UndefNode { .. }
        | Node::PostExecutionNode { .. }
        | Node::RescueModifierNode { .. }
        | Node::MultiWriteNode { .. }
        | Node::MatchPredicateNode { .. }
        | Node::MatchRequiredNode { .. } => false,
        _ => true,
    }
}

/// RuboCop's macro scope follows only class-like nodes and transparent
/// wrappers, not arbitrary enclosing classes/blocks. Ordinary arguments,
/// assignments, loops and conditions are not macro bodies; direct arguments
/// of pinned class constructors are an upstream exception. The Program and
/// Statements nodes are Prism-only; a block's owning call is a sibling of its
/// body in RuboCop, so that edge is transparent here too.
pub(super) fn macro_scope(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    let mut ancestors = cx.ancestors().peekable();
    while let Some(parent) = ancestors.next() {
        match parent {
            Node::ProgramNode { .. }
            | Node::ClassNode { .. }
            | Node::ModuleNode { .. }
            | Node::SingletonClassNode { .. } => return true,
            Node::StatementsNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::BlockNode { .. }
            | Node::LambdaNode { .. }
            | Node::ElseNode { .. } => {}
            Node::BeginNode { .. } => {
                // Prism keeps the protected body directly under Begin, while
                // RuboCop inserts rescue/ensure nodes which break macro scope.
                let begin = parent.as_begin_node().unwrap();
                if begin.rescue_clause().is_some() || begin.ensure_clause().is_some() {
                    return false;
                }
            }
            Node::IfNode { .. } => {
                if same_location(&parent.as_if_node().unwrap().predicate(), child) {
                    return false;
                }
            }
            Node::UnlessNode { .. } => {
                if same_location(&parent.as_unless_node().unwrap().predicate(), child) {
                    return false;
                }
            }
            Node::CallNode { .. } => {
                if class_constructor(parent) {
                    return true;
                }
                if child.as_block_node().is_none() {
                    return false;
                }
            }
            Node::ArgumentsNode { .. } => {
                return ancestors.peek().is_some_and(|next| class_constructor(next));
            }
            _ => return false,
        }
        child = parent;
    }
    true
}

fn same_location(left: &Node<'_>, right: &Node<'_>) -> bool {
    left.location().start_offset() == right.location().start_offset()
        && left.location().end_offset() == right.location().end_offset()
}

pub(super) fn class_constructor(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    let Some(receiver) = call.receiver() else {
        return false;
    };
    // The pinned class-constructor pattern is a send, not a csend.
    if call
        .call_operator_loc()
        .is_some_and(|loc| loc.as_slice() == b"&.")
    {
        return false;
    }
    match call.name().as_slice() {
        b"new" => ["Class", "Module", "Struct"]
            .iter()
            .any(|name| ast::is_global_const(&receiver, &[*name])),
        b"define" => ast::is_global_const(&receiver, &["Data"]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/method_call_with_args_parentheses_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::{offense::Offense, pipeline::lint_str_with};

    fn offenses(source: &str) -> Vec<Offense> {
        // Avoid mistaking a parser rejection for a negative matcher result.
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "invalid test source: {source}"
        );
        lint_str_with(&[&MethodCallWithArgsParentheses], source)
    }

    fn assert_correction(cop: &dyn Cop, before: &str, after: &str) {
        crate::cops::testing::assert_correction(cop, before, after);
        let original = ruby_prism::parse(before.as_bytes());
        let fixed = ruby_prism::parse(after.as_bytes());
        assert_eq!(
            ast::fingerprint_ignoring_grouping(&original.node(), before.as_bytes()),
            ast::fingerprint_ignoring_grouping(&fixed.node(), after.as_bytes()),
            "changed argument/block AST: {before} -> {after}",
        );
    }

    fn assert_positions(source: &str, positions: &[(u32, u32)]) {
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            positions,
            "{source}"
        );
        for offense in found {
            assert_eq!(
                offense.message,
                "Use parentheses for method calls with arguments."
            );
            assert!(offense.correction.is_none());
        }
    }

    #[test]
    fn data_byte_guard_keeps_call_yield_and_block_pass_findings() {
        use crate::cops::testing::correction_offenses;
        for call in ["self.f 1", "self&.f 1", "yield 1", "self.f &block"] {
            let source =
                format!("def example\n  {call}\nend\np DATA.method(:tell).call\n__END__\nraw\n");
            let found = correction_offenses(&MethodCallWithArgsParentheses, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "Use parentheses for method calls with arguments."
            );
            assert!(found[0].correction.is_none(), "{source}");
        }
        // Merging a group removes a space, so it too changes DATA's position.
        let source = "self.f (1)\np DATA.pos\n__END__\nraw\n";
        assert!(
            correction_offenses(&MethodCallWithArgsParentheses, source)[0]
                .correction
                .is_none()
        );
        // An equal-length edit keeps DATA's position, but `DATA` with a data
        // section is a listed observer: held.
        crate::cops::testing::assert_listed_hold(
            &MethodCallWithArgsParentheses,
            "self.f  1\np DATA.pos\n__END__\nraw\n",
        );
    }

    #[test]
    fn corrects_pinned_require_parentheses_examples() {
        for (before, after) in [
            ("top.test (a and b)\n", "top.test((a and b))\n"),
            ("top.test (a or b)\n", "top.test((a or b))\n"),
            ("top.test (not a)\n", "top.test((not a))\n"),
            ("top.test (a if b)\n", "top.test((a if b))\n"),
            ("top.test (a while b)\n", "top.test((a while b))\n"),
            ("top.test (a rescue b)\n", "top.test((a rescue b))\n"),
            ("top.test (a in b)\n", "top.test((a in b))\n"),
            ("top.test (a, b = 1, 2)\n", "top.test((a, b = 1, 2))\n"),
            ("top.test (a; b)\n", "top.test((a; b))\n"),
            ("top.test (a ? b : c)\n", "top.test(a ? b : c)\n"),
            ("top.eq (1 + 2)\n", "top.eq(1 + 2)\n"),
            ("top.eq (1 + 2) + 3\n", "top.eq((1 + 2) + 3)\n"),
            ("top.eq (1 + 2), 3\n", "top.eq((1 + 2), 3)\n"),
            ("def f\n  Test a, b\nend\n", "def f\n  Test(a, b)\nend\n"),
            ("def f\n  yield a\nend\n", "def f\n  yield(a)\nend\n"),
            ("def f\n  eq (1 + 2)\nend\n", "def f\n  eq(1 + 2)\nend\n"),
            (
                "def f\n  eq 1, (2 + 3)\n  eq 1, 2, 3\nend\n",
                "def f\n  eq(1, (2 + 3))\n  eq(1, 2, 3)\nend\n",
            ),
            (
                "def f\n  foo bar.baz(abc, xyz)\nend\n",
                "def f\n  foo(bar.baz(abc, xyz))\nend\n",
            ),
            (
                "def f\n  foo bar.baz(abc, xyz), foo(baz)\nend\n",
                "def f\n  foo(bar.baz(abc, xyz), foo(baz))\nend\n",
            ),
            (
                "def f\n  raise NotImplementedError\nend\n",
                "def f\n  raise(NotImplementedError)\nend\n",
            ),
            ("def f\n  sleep 1\nend\n", "def f\n  sleep(1)\nend\n"),
        ] {
            assert_correction(&MethodCallWithArgsParentheses, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }

    #[test]
    fn unknown_kernel_test_arguments_retain_the_pinned_finding_without_an_edit() {
        // Pinned require_parentheses examples: unknown `a, b` can be Kernel's
        // metadata flag/path tuple even with a receiver. No value-flow proof.
        for (source, position) in [
            ("def f\n  test a, b\nend\n", (2, 3)),
            ("top.test a, b\n", (1, 1)),
            ("top&.test a, b\n", (1, 1)),
            (
                "def f(*args, **kwargs, &block); object.test *args, **kwargs, &block; end\n",
                (1, 33),
            ),
            ("object.test <<~TEXT, 1 # comment\n  value\nTEXT\n", (1, 1)),
            (
                "object.test <<~TEXT,\n  value\nTEXT\n  __LINE__ # tail\n",
                (1, 1),
            ),
        ] {
            let found =
                crate::cops::testing::correction_offenses(&MethodCallWithArgsParentheses, source);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), position);
            assert_eq!(
                found[0].message,
                "Use parentheses for method calls with arguments."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn corrections_retain_statement_only_arguments_and_separators() {
        for argument in [
            "alias copied original",
            "alias $copied $original",
            "undef original",
            "1;",
            ";1",
            ";1;",
            "1; # trailing separator\n",
            "# leading separator\n;1",
        ] {
            let before = format!("self.sink ({argument})\nobject.test 2\n");
            let after = format!("self.sink(({argument}))\nobject.test(2)\n");
            assert_correction(&MethodCallWithArgsParentheses, &before, &after);
            assert!(offenses(&after).is_empty(), "{after}");
        }
        // Semicolons within expressions are not statement separators belonging
        // to the outer group, and must not prevent delimiter reuse.
        for expression in ["\"a;b\"", "begin; 1; end"] {
            assert_correction(
                &MethodCallWithArgsParentheses,
                &format!("object.test ({expression})\n"),
                &format!("object.test({expression})\n"),
            );
        }
    }

    #[test]
    fn corrections_retain_post_execution_argument_grouping() {
        for argument in [
            "END {}",
            "END { 1 }",
            "# before END\nEND { 1 } # after END\n",
            "\n  END { # deferred body\n    __LINE__\n  }\n",
            "\r\n  END { # deferred body\r\n    __LINE__\r\n  } # after END\r\n",
        ] {
            let before = format!("self.sink ({argument})\nself.other 1\n");
            let after = format!("self.sink(({argument}))\nself.other(1)\n");
            assert_positions(&before, &[(1, 1), (before.lines().count() as u32, 1)]);
            assert_correction(&MethodCallWithArgsParentheses, &before, &after);
            assert!(offenses(&after).is_empty(), "{after}");
        }
    }

    #[test]
    fn corrections_preserve_command_precedence_and_block_ownership() {
        for (before, after) in [
            ("object.outer inner 1\n", "object.outer(inner 1)\n"),
            ("object.outer(inner 1)\n", "object.outer(inner(1))\n"),
            ("object.outer inner(1)\n", "object.outer(inner(1))\n"),
            (
                "object.outer inner 1 do; body; end\n",
                "object.outer(inner 1) do; body; end\n",
            ),
            (
                "object.outer inner(1) { body }\n",
                "object.outer(inner(1) { body })\n",
            ),
            (
                "object.outer 1 do; body; end\n",
                "object.outer(1) do; body; end\n",
            ),
            ("object.test a ? b : c\n", "object.test(a ? b : c)\n"),
            ("object.test a rescue b\n", "object.test(a) rescue b\n"),
            ("object.test a if b\n", "object.test(a) if b\n"),
            ("object.test a and b\n", "object.test(a) and b\n"),
            ("object.test a or b\n", "object.test(a) or b\n"),
            ("object.test key: value\n", "object.test(key: value)\n"),
            ("object.test key:\n", "object.test(key:)\n"),
            (
                "def f(*args, **kwargs, &block); object.consume *args, **kwargs, &block; end\n",
                "def f(*args, **kwargs, &block); object.consume(*args, **kwargs, &block); end\n",
            ),
            (
                "def f(&); object.test &; end\n",
                "def f(&); object.test(&); end\n",
            ),
            ("object[inner 1] = value\n", "object[inner(1)] = value\n"),
            ("object.foo += inner 1\n", "object.foo += inner(1)\n"),
            ("object.test /pattern/i\n", "object.test(/pattern/i)\n"),
            ("object.test -> { body }\n", "object.test(-> { body })\n"),
            ("object::test value\n", "object::test(value)\n"),
        ] {
            assert_correction(&MethodCallWithArgsParentheses, before, after);
        }
    }

    #[test]
    fn corrections_retain_heredoc_headers_bodies_comments_and_physical_lines() {
        for (before, after) in [
            (
                "object.test <<~TEXT\n  value\nTEXT\n",
                "object.test(<<~TEXT)\n  value\nTEXT\n",
            ),
            (
                "object.consume <<~TEXT, 1 # comment\n  value\nTEXT\n",
                "object.consume(<<~TEXT, 1) # comment\n  value\nTEXT\n",
            ),
            (
                "object.test <<~A, <<~B\n  a\nA\n  b\nB\n",
                "object.test(<<~A, <<~B)\n  a\nA\n  b\nB\n",
            ),
            (
                "object.test <<~TEXT.chomp do; body; end\n  value\nTEXT\n",
                "object.test(<<~TEXT.chomp) do; body; end\n  value\nTEXT\n",
            ),
            (
                "object.consume <<~TEXT,\n  value\nTEXT\n  __LINE__ # tail\n",
                "object.consume(<<~TEXT,\n  value\nTEXT\n  __LINE__) # tail\n",
            ),
            (
                "object.test 1, # keep comment\n  __LINE__\n",
                "object.test(1, # keep comment\n  __LINE__)\n",
            ),
            (
                "object.test \\\n  __LINE__\n",
                "object.test(\\\n  __LINE__)\n",
            ),
            (
                "object.test  (\n  __LINE__\n)\n",
                "object.test(\n  __LINE__\n)\n",
            ),
            (
                "object.test \\\n  (1 + 2)\n",
                "object.test(\\\n  (1 + 2))\n",
            ),
            (
                "object.test \t 1 # keep tail\n",
                "object.test(1) # keep tail\n",
            ),
            (
                "\u{feff}object.test 1,\r\n  __LINE__\r\n__END__\r\nraw data\r\n",
                "\u{feff}object.test(1,\r\n  __LINE__)\r\n__END__\r\nraw data\r\n",
            ),
        ] {
            assert_correction(&MethodCallWithArgsParentheses, before, after);
        }
    }

    #[test]
    fn flags_command_calls_and_safe_navigation() {
        for source in ["top.test a, b", "top&.test a, b", "top::test a, b"] {
            assert_positions(source, &[(1, 1)]);
        }
        for method in ["test", "Test", "raise", "sleep"] {
            assert_positions(&format!("def foo\n  {method} a\nend"), &[(2, 3)]);
        }
    }

    #[test]
    fn accepts_parentheses_no_arguments_operators_setters_and_super() {
        for source in [
            "top.test",
            "top.test()",
            "top.test(a, b)",
            "foo(:arg) do\n  bar\nend",
            "top.test + a",
            "top.test = a",
            "!test",
            "top.== a",
            "top[a]",
            "top[a] = b",
            "top&.test = a",
            "top.test += a",
            "super a",
            "super",
            "super(a)",
            "def f; yield; end",
            "def f; yield(); end",
            "def f; yield(a); end",
            "def x() = foo(\"bar\")",
            "def self.x() = foo()",
        ] {
            assert_positions(source, &[]);
        }
    }

    #[test]
    fn ignores_only_implicit_receiver_calls_in_macro_scope() {
        for source in [
            "bar :baz",
            "class Foo\n  bar :baz\nend",
            "module Foo\n  bar :baz\nend",
            "class << self\n  bar :baz\nend",
            "describe Foo do\n  bar :baz\nend",
            "if condition\n  bar :baz\nelse\n  bar :other\nend",
            "bar :baz if condition",
            "begin\n  bar :baz\nend",
            "(bar :baz)",
            "Class.new { bar :baz }",
            "Struct.new(:x) { bar :baz }",
            "def f; Class.new { bar :baz }; end",
        ] {
            assert_positions(source, &[]);
        }
        for source in [
            "class Foo\n  self.bar :baz\nend",
            "module Foo\n  object&.bar :baz\nend",
            "def foo\n  bar :baz\nend",
            "def self.foo\n  bar :baz\nend",
            "def foo\n  each { bar :baz }\nend",
            "consume(bar :baz)",
            "x = bar :baz",
            "[(bar :baz)]",
            "if bar :baz\nend",
            "while bar :baz\nend",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn grouped_arguments_are_not_invocation_parentheses() {
        for argument in [
            "(a and b)",
            "(a or b)",
            "(not a)",
            "(a if b)",
            "(a while b)",
            "(a rescue b)",
            "(a in b)",
            "(a, b = 1, 2)",
            "(a; b)",
            "(a ? b : c)",
            "(1 + 2)",
            "(1 + 2) + 3",
            "(1 + 2), 3",
        ] {
            assert_positions(&format!("top.test {argument}"), &[(1, 1)]);
            assert_positions(&format!("def f\n  test {argument}\nend"), &[(2, 3)]);
        }
    }

    #[test]
    fn detects_nested_command_association_and_blocks_without_changing_them() {
        for (source, positions) in [
            ("object.outer inner 1", vec![(1, 1), (1, 14)]),
            ("object.outer inner(1)", vec![(1, 1)]),
            ("object.outer(inner 1)", vec![(1, 14)]),
            (
                "def f\n  outer inner 1 do; body; end\nend",
                vec![(2, 3), (2, 9)],
            ),
            ("def f\n  outer inner(1) { body }\nend", vec![(2, 3)]),
            (
                "def f\n  outer 1 do; inner 2; end\nend",
                vec![(2, 3), (2, 15)],
            ),
            ("object.outer 1 do; macro 2; end", vec![(1, 1)]),
            ("def f\n  outer 1 do; _1; end\nend", vec![(2, 3)]),
            ("def f\n  outer 1 do; it; end\nend", vec![(2, 3)]),
        ] {
            assert_positions(source, &positions);
        }
    }

    #[test]
    fn detects_block_splat_keyword_and_forwarded_arguments() {
        for argument in [
            "&block",
            "*args",
            "**kwargs",
            "key: value",
            "key:",
            "1, &block",
        ] {
            assert_positions(&format!("def f\n  test {argument}\nend"), &[(2, 3)]);
            assert_positions(&format!("object.test {argument}"), &[(1, 1)]);
            assert_positions(&format!("object.test({argument})"), &[]);
        }
        assert_positions("def f(...); object.test(...); end", &[]);
        assert_positions("def f(*, **, &); object.test(*, **, &); end", &[]);
        assert_positions("def f(&); object.test &; end", &[(1, 11)]);
    }

    #[test]
    fn exempts_setters_and_indexes_but_checks_their_arguments() {
        assert_positions("object.foo= value", &[]);
        assert_positions("object.foo=(value)", &[]);
        assert_positions("object.foo = value", &[]);
        assert_positions("object[index] = value", &[]);
        assert_positions("object[inner 1] = value", &[(1, 8)]);
        assert_positions("object.foo += inner 1", &[(1, 15)]);
    }

    #[test]
    fn heredoc_and_block_highlights_end_at_the_send_not_the_body() {
        for source in [
            "object.test <<~TEXT\n  value\nTEXT\n",
            "object.test <<~TEXT, 1\n  value\nTEXT\n",
            "object.test 1 do\n  body\nend\n",
        ] {
            assert_positions(source, &[(1, 1)]);
            assert_eq!(offenses(source)[0].end_line, 1);
        }
        assert_positions("object.test(<<~TEXT)\n  value\nTEXT\n", &[]);
        let source = "object.test(\n  value\n) do\n  body\nend\n";
        assert_positions(source, &[]);
        let source = "object.test value,\n  other do\n  body\nend\n";
        assert_positions(source, &[(1, 1)]);
        assert_eq!(offenses(source)[0].end_line, 2);
    }

    #[test]
    fn macro_scope_respects_wrappers_boundaries_and_constructor_receivers() {
        for source in [
            "def f; ::Class.new { bar :baz }; end",
            "def f; Module.new { bar :baz }; end",
            "def f; ::Struct.new(:x) { bar :baz }; end",
            "def f; Data.define(:x) { bar :baz }; end",
            "def f; Class.new(bar :baz); end",
            "class Foo; if x; bar :baz; elsif y; bar :baz; else; bar :baz; end; end",
            "class Foo; -> { bar :baz }; end",
        ] {
            assert_positions(source, &[]);
        }
        for source in [
            "def f; Other::Class.new { bar :baz }; end",
            "def f; Class&.new { bar :baz }; end",
            "def f; Data&.define(:x) { bar :baz }; end",
            "def f; klass = Class; klass.new { bar :baz }; end",
            "class Foo; while condition; bar :baz; end; end",
            "class Foo; begin; x; rescue; bar :baz; end; end",
            "class Foo; value = (bar :baz); end",
            "class Foo; foo { |x = (bar :baz)| }; end",
            "class Foo; foo(bar :baz); end",
            "class Foo; if bar :baz; x; end; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn rescue_and_ensure_are_not_transparent_macro_wrappers() {
        for source in [
            "begin; bar :baz; rescue; x; end",
            "begin; bar :baz; ensure; x; end",
            "class Foo; bar :baz; rescue; x; end",
            "class Foo; bar :baz; ensure; x; end",
            "begin; x; rescue; x; else; bar :baz; end",
            "begin; x; ensure; bar :baz; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn honors_both_directive_prefixes_and_multiline_highlights() {
        for prefix in ["rustocop", "rubocop"] {
            for source in [
                format!("# {prefix}:disable Style/MethodCallWithArgsParentheses\nobject.test 1\n"),
                format!("object.test 1 # {prefix}:disable Style/MethodCallWithArgsParentheses\n"),
                format!(
                    "# {prefix}:disable-next Style/MethodCallWithArgsParentheses\nobject.test 1,\n  2\n"
                ),
                format!(
                    "object.test 1,\n  2 # {prefix}:disable Style/MethodCallWithArgsParentheses\n"
                ),
            ] {
                assert_positions(&source, &[]);
            }
            assert_positions(
                &format!(
                    "object.test 1 do\n  body # {prefix}:disable Style/MethodCallWithArgsParentheses\nend\n"
                ),
                &[(1, 1)],
            );
        }
    }

    #[test]
    fn large_flat_macro_bodies_and_method_bodies_do_not_scan_siblings() {
        let source = format!(
            "class Foo\n{}\ndef f\n{}\nend\nend",
            "macro :value\n".repeat(2000),
            "test value\n".repeat(2000)
        );
        assert_eq!(offenses(&source).len(), 2000);
    }

    #[test]
    fn correction_span_work_is_lazy_and_bounded_by_local_arguments() {
        use crate::cops::testing::correction_offenses;
        let source = format!(
            "{}{}",
            "object.test value\n".repeat(4000),
            "# unrelated\n".repeat(4000)
        );
        CORRECTION_BYTES.with(|bytes| bytes.set(0));
        assert_eq!(offenses(&source).len(), 4000);
        assert_eq!(CORRECTION_BYTES.with(|bytes| bytes.get()), 0);
        let found = correction_offenses(&MethodCallWithArgsParentheses, &source);
        assert_eq!(found.len(), 4000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert_eq!(
            CORRECTION_BYTES.with(|bytes| bytes.get()),
            4000 * " value".len()
        );
        assert!(CORRECTION_BYTES.with(|bytes| bytes.get()) < source.len());
    }

    #[test]
    fn flags_yield_without_parentheses() {
        assert_positions("def foo\n  yield a\nend", &[(2, 3)]);
        assert_positions("def foo\n  yield a, b\nend", &[(2, 3)]);
        // Yield shares the pinned implicit-receiver macro exemption.
        assert_positions("def f; Class.new { yield a }; end", &[]);
        assert_positions("def f; Class.new(yield a); end", &[]);
        assert_positions("def f; Class&.new { yield a }; end", &[(1, 21)]);
    }
}
