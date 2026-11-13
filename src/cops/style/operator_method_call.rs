//! Explicit operator sends, with precedence-preserving atomic safe edits.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct OperatorMethodCall;
const OPERATORS: &[&[u8]] = &[
    b"|", b"^", b"&", b"<=>", b"==", b"===", b"=~", b">", b">=", b"<", b"<=", b"<<", b">>", b"+",
    b"-", b"*", b"/", b"%", b"**", b"~", b"!", b"!=", b"!~",
];
impl Cop for OperatorMethodCall {
    fn name(&self) -> &'static str {
        "Style/OperatorMethodCall"
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
        let (Some(dot), Some(selector), Some(receiver)) = (
            call.call_operator_loc(),
            call.message_loc(),
            call.receiver(),
        ) else {
            return;
        };
        if dot.as_slice() == b"&."
            || !OPERATORS.contains(&call.name().as_slice())
            || selector.as_slice() != call.name().as_slice()
            || ast::is_const(&receiver)
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let [rhs] = args.as_slice() else { return };
        if matches!(
            rhs,
            Node::SplatNode { .. }
                | Node::ForwardingArgumentsNode { .. }
                | Node::BlockArgumentNode { .. }
        ) {
            return;
        }
        let hash_first = rhs
            .as_keyword_hash_node()
            .and_then(|n| n.elements().first())
            .or_else(|| rhs.as_hash_node().and_then(|n| n.elements().first()));
        if hash_first.is_some_and(|n| n.as_assoc_splat_node().is_some()) {
            return;
        }
        let chained = !call.block().is_some_and(|n| n.as_block_node().is_some())
            && cx
                .ancestors()
                // Prism adds an argument-list wrapper absent from RuboCop's
                // send tree. It must not hide an enclosing call's exemption.
                .find(|n| n.as_arguments_node().is_none())
                .and_then(|n| n.as_call_node())
                .is_some();
        if chained && call.opening_loc().is_some() && child_bearing(rhs) {
            return;
        }
        let (line, col) = file.line_col(dot.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Redundant dot detected.".into(),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| correction(node, &receiver, rhs, cx, file))
                .flatten(),
        });
    }
}

// RuboCop's parenthesized/chained exception tests the first AST child.
fn child_bearing(node: &Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        return call.receiver().is_some();
    }
    if let Some(array) = node.as_array_node() {
        return !array.elements().is_empty();
    }
    if let Some(hash) = node.as_hash_node() {
        return !hash.elements().is_empty();
    }
    matches!(
        node,
        Node::ConstantPathNode { .. }
            | Node::RangeNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::LocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::GlobalVariableReadNode { .. }
            | Node::ClassVariableReadNode { .. }
            | Node::ConstantReadNode { .. }
            | Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
    )
}

// Only primary expressions and no-argument receiver chains. Unknown command
// arguments, blocks, assignments and compound expressions stay reported, not
// rewritten with a merely parseable but differently associated operator.
fn primary(node: &Node<'_>) -> bool {
    if let Some(group) = node.as_parentheses_node() {
        return group
            .body()
            .and_then(|n| n.as_statements_node())
            .is_some_and(|s| s.body().len() == 1 && primary(&s.body().first().unwrap()));
    }
    if let Some(call) = node.as_call_node() {
        return call.block().is_none()
            && ast::call_arguments(&call).is_empty()
            && !ast::is_operator_method(call.name().as_slice())
            && call.message_loc().is_some()
            && call.receiver().is_none_or(|n| primary(&n));
    }
    matches!(
        node,
        Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::SelfNode { .. }
            | Node::LocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::ClassVariableReadNode { .. }
            | Node::GlobalVariableReadNode { .. }
            | Node::ConstantReadNode { .. }
            | Node::ConstantPathNode { .. }
    ) || node.as_hash_node().is_some_and(|n| n.elements().is_empty())
}
fn correction(
    node: &Node<'_>,
    receiver: &Node<'_>,
    rhs: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let call = node.as_call_node()?;
    // ! and ~ have no infix syntax; an explicit one-argument override cannot
    // be expressed as the unary operator. Literal blocks must not migrate.
    if matches!(call.name().as_slice(), b"!" | b"~")
        || call.block().is_some()
        || !primary(receiver)
        || !primary(rhs)
    {
        return None;
    }
    let root = cx.ancestors().last().unwrap_or(node);
    if file.contains_heredoc(root, node) {
        return None;
    }
    let start = node.location().start_offset();
    let end = ast::source_end(node);
    let bytes = &file.bytes[start..end];
    if bytes.contains(&b'\n') || bytes.contains(&b'#') {
        return None;
    }
    let dot = call.call_operator_loc()?;
    let selector = call.message_loc()?;
    // An explicit send is a primary expression. Its infix replacement must
    // remain one operand whether it is the enclosing call's receiver OR an
    // argument (including the RHS of a binary operator). Prism's ArgumentsNode
    // is only a list wrapper, not a precedence boundary; parentheses are one.
    let wrap = cx
        .ancestors()
        .find(|n| n.as_arguments_node().is_none())
        .is_some_and(|n| n.as_call_node().is_some());
    let text = |a, b| std::str::from_utf8(&file.bytes[a..b]).ok();
    let mut replacement = String::new();
    if wrap {
        replacement.push('(');
    }
    let signed_power =
        call.name().as_slice() == b"**" && matches!(file.bytes.get(start), Some(b'-' | b'+'));
    if signed_power {
        replacement.push('(');
    }
    replacement.push_str(text(start, dot.start_offset())?);
    if signed_power {
        replacement.push(')');
    }
    replacement.push(' ');
    replacement.push_str(text(selector.start_offset(), selector.end_offset())?);
    if wrap {
        replacement.push(' ');
        replacement.push_str(text(
            rhs.location().start_offset(),
            rhs.location().end_offset(),
        )?);
        replacement.push(')');
    } else {
        let rest = text(selector.end_offset(), end)?;
        // `foo +(bar)` is a command invocation of foo with unary +bar,
        // NOT an infix addition. Retain the argument group but separate it
        // from every unary/splat/literal-ambiguous operator.
        if selector.end_offset() == rhs.location().start_offset()
            || (matches!(
                call.name().as_slice(),
                b"+" | b"-" | b"*" | b"**" | b"&" | b"/" | b"%"
            ) && rest.starts_with('('))
        {
            replacement.push(' ');
        }
        replacement.push_str(rest);
    }
    Some(Correction {
        range: start..end,
        replacement,
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's corresponding Style specs (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/OperatorMethodCall")
            .collect()
    }
    #[test]
    fn pinned_offense_and_canonical_correction() {
        let found = offenses("foo.+ bar\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 4, 1));
        assert_eq!(found[0].message, "Redundant dot detected.");
        assert_correction(&OperatorMethodCall, "foo.+ bar\n", "foo + bar\n");
    }
    #[test]
    fn signed_receiver_exponentiation_keeps_unary_precedence() {
        assert_correction(&OperatorMethodCall, "-1.**(2)\n", "(-1) ** (2)\n");
    }

    #[test]
    fn pinned_binary_operator_shapes_and_chains_correct_atomically() {
        for operator in OPERATORS.iter().filter(|op| !matches!(**op, b"!" | b"~")) {
            let op = std::str::from_utf8(operator).unwrap();
            for (before, after, col) in [
                (format!("foo.{op} bar"), format!("foo {op} bar"), 4),
                (format!("foo.{op}bar"), format!("foo {op} bar"), 4),
                (format!("foo.{op} 42"), format!("foo {op} 42"), 4),
                (
                    format!("foo bar.{op} baz"),
                    format!("foo (bar {op} baz)"),
                    8,
                ),
                (
                    format!("foo.bar.{op}(baz).quux(2)"),
                    format!("(foo.bar {op} baz).quux(2)"),
                    8,
                ),
            ] {
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col), (1, col), "{before}");
                assert_eq!(found[0].message, "Redundant dot detected.");
                assert_correction(&OperatorMethodCall, &before, &after);
            }
            let gap = if matches!(op, "+" | "-" | "*" | "**" | "&" | "/" | "%") {
                " "
            } else {
                ""
            };
            assert_correction(
                &OperatorMethodCall,
                &format!("foo.{op}(bar)"),
                &format!("foo {op}{gap}(bar)"),
            );
        }
        for (before, after) in [
            ("foo.==({})", "foo ==({})"),
            ("foo.+ @bar.to_s", "foo + @bar.to_s"),
            ("foo./ (bar)", "foo / (bar)"),
            ("foo.+(bar) * baz", "(foo + bar) * baz"),
            ("foo.-(bar).abs", "(foo - bar).abs"),
        ] {
            assert_correction(&OperatorMethodCall, before, after);
        }
    }

    #[test]
    fn pinned_unary_argument_shapes_remain_offenses_without_invalid_infix_edits() {
        use crate::cops::testing::correction_offenses;
        for op in ["!", "~"] {
            for before in [
                format!("foo.{op} bar"),
                format!("foo.{op}bar"),
                format!("foo.{op} 42"),
                format!("foo bar.{op} baz"),
                format!("foo.{op}(bar)"),
                format!("foo.bar.{op}(baz).quux(2)"),
            ] {
                let found = correction_offenses(&OperatorMethodCall, &before);
                assert_eq!(found.len(), 1, "{before}");
                assert!(found[0].correction.is_none(), "{before}");
            }
        }
    }

    #[test]
    fn complex_operands_blocks_comments_and_heredocs_withhold_individually() {
        use crate::cops::testing::correction_offenses;
        for before in [
            "foo.+(bar * baz)",
            "(foo + bar).*(baz)",
            "foo.+(bar) { side_effect }",
            "foo.+(\nbar\n)",
            "foo.+(bar # comment\n)",
            "foo.==(<<~DOC)\ntext\nDOC\n",
            "foo.+(command arg)",
            "foo.+(@bar) { body }.to_s",
        ] {
            let found = correction_offenses(&OperatorMethodCall, before);
            assert_eq!(found.len(), 1, "{before}");
            assert!(found[0].correction.is_none(), "{before}");
        }
    }

    #[test]
    fn splats_forwarding_constants_and_unary_spellings_are_not_offenses() {
        for before in [
            "def f(...); bar.==(...); end",
            "def f(&blk); bar.==(&blk); end",
            "def f(&); bar.==(&); end",
            "def f(*); bar.==(*); end",
            "def f(**kw); bar.==(**kw); end",
            "def f(**); bar.==(**); end",
            "foo.-@ bar",
            "foo.!@ bar",
            "foo.~@ bar",
            "foo.` bar",
            "::Foo.+(bar)",
            "A::Foo.+(bar)",
            "foo.==({**kw})",
        ] {
            assert!(offenses(before).is_empty(), "{before}");
        }
    }

    #[test]
    fn parenthesized_operator_arguments_use_the_enclosing_call_policy() {
        for source in [
            "puts(foo.+(42))",
            "puts(foo.+(@bar))",
            "puts foo.+(42)",
            "puts(first, foo.+(@bar))",
            "outer(inner(foo.+(42)))",
            "puts(foo.+([1]))",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_correction(&OperatorMethodCall, "puts(foo.+(bar))", "puts((foo + bar))");
        assert_correction(&OperatorMethodCall, "puts(foo.+ 42)", "puts((foo + 42))");
    }

    // Compare dispatch trees from the main Prism parse, not merely parse success.
    // Ignore grouping/layout but preserve call receiver, selector and argument roles.
    fn dispatch_tree(node: &Node<'_>, source: &[u8]) -> String {
        if let Some(program) = node.as_program_node() {
            return dispatch_tree(&program.statements().as_node(), source);
        }
        if let Some(statements) = node.as_statements_node() {
            return format!(
                "statements{:?}",
                statements
                    .body()
                    .iter()
                    .map(|n| dispatch_tree(&n, source))
                    .collect::<Vec<_>>()
            );
        }
        if let Some(group) = node.as_parentheses_node() {
            let statements = group.body().unwrap().as_statements_node().unwrap();
            assert_eq!(statements.body().len(), 1);
            return dispatch_tree(&statements.body().first().unwrap(), source);
        }
        if let Some(call) = node.as_call_node() {
            assert!(call.block().is_none());
            return format!(
                "call({:?},safe={},receiver={:?},args={:?})",
                call.name().as_slice(),
                call.call_operator_loc()
                    .is_some_and(|l| l.as_slice() == b"&."),
                call.receiver().map(|n| dispatch_tree(&n, source)),
                ast::call_arguments(&call)
                    .iter()
                    .map(|n| dispatch_tree(n, source))
                    .collect::<Vec<_>>()
            );
        }
        format!("{:?}", ast::fingerprint_ignoring_grouping(node, source))
    }

    fn assert_dispatch_preserved(before: &str, after: &str) {
        let original = ruby_prism::parse(before.as_bytes());
        let fixed = ruby_prism::parse(after.as_bytes());
        assert_eq!(original.errors().count(), 0, "{before}");
        assert_eq!(fixed.errors().count(), 0, "{after}");
        assert_eq!(
            dispatch_tree(&original.node(), before.as_bytes()),
            dispatch_tree(&fixed.node(), after.as_bytes()),
            "{before} -> {after}"
        );
    }

    #[test]
    fn argument_and_receiver_roles_preserve_exact_bytes_and_dispatch_tree() {
        for operand in ["nil", "true", "false", "{}", "rhs", "rhs()"] {
            for operator in OPERATORS.iter().filter(|op| !matches!(**op, b"!" | b"~")) {
                let op = std::str::from_utf8(operator).unwrap();
                let before = format!("x.{op}({operand})");
                let after = format!("(x {op} {operand})");
                for (prefix, suffix) in [
                    ("3 * ", ""), // right binary operand (ArgumentsNode)
                    ("3 - ", ""), // association, not just precedence
                    ("3 ** ", ""),
                    ("", " * 3"), // left binary operand (receiver)
                    ("", " - 3"),
                    ("", " ** 3"),
                    ("", ".abs"), // chained receiver
                    ("", "&.abs"),
                    ("p(", ")"), // parenthesized argument
                    ("p(1, ", ", 2)"),
                    ("p ", ""), // command argument
                    ("p 3 * ", ""),
                    ("p(3 * ", ")"), // reproducer
                    ("obj.value = 3 * ", ""),
                    ("obj[3 * ", "]"),
                ] {
                    let before = format!("{prefix}{before}{suffix}\n");
                    let after = format!("{prefix}{after}{suffix}\n");
                    assert_eq!(offenses(&before).len(), 1, "{before}");
                    let fixes =
                        crate::cops::testing::correction_offenses(&OperatorMethodCall, &before);
                    let applied = crate::corrections::apply(before.as_bytes(), &fixes).unwrap();
                    let actual = std::str::from_utf8(&applied.bytes).unwrap();
                    assert_dispatch_preserved(&before, actual);
                    assert_correction(&OperatorMethodCall, &before, &after);
                }
            }
        }
    }

    #[test]
    fn explicit_grouping_and_child_bearing_exemptions_remain_distinct() {
        for operand in ["nil", "true", "false", "{}", "rhs", "rhs()"] {
            for (prefix, suffix) in [("3 * (", ")"), ("p((", "))"), ("(", ").abs")] {
                let before = format!("{prefix}x.+({operand}){suffix}\n");
                let after = format!("{prefix}x + ({operand}){suffix}\n");
                assert_correction(&OperatorMethodCall, &before, &after);
                assert_dispatch_preserved(&before, &after);
            }
        }
        for operand in ["42", "@bar", "[1]", "{a: 1}", "obj.rhs"] {
            for (prefix, suffix) in [("3 * ", ""), ("p(", ")"), ("p ", ""), ("", ".abs")] {
                assert!(offenses(&format!("{prefix}x.+({operand}){suffix}")).is_empty());
            }
        }
        // The same child-bearing operand without explicit argument parentheses
        // remains an offense; grouping is a correction concern, not an exemption.
        for operand in ["nil", "true", "false", "rhs", "rhs()", "42", "@bar"] {
            let before = format!("p(x.+ {operand})\n");
            let after = format!("p((x + {operand}))\n");
            assert_correction(&OperatorMethodCall, &before, &after);
            assert_dispatch_preserved(&before, &after);
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "foo + bar",
            "Foo.+(bar)",
            "foo.+(a,b)",
            "obj.!",
            "obj.+@ bar",
            "obj&.+(bar)",
            "foo.+(@bar).to_s",
            "def f(*a); obj.==(*a); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
