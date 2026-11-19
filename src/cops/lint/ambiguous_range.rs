//! `Lint/AmbiguousRange` — flags range boundaries that are complex
//! expressions without parentheses (`x || 1..2`, `x - 1..2`, `1..2.to_a`):
//! ranges bind loosely, so the boundary is rarely what it looks like.
//!
//! Matches RuboCop's semantics with its default
//! `RequireParenthesesForMethodChains: false`. A boundary is fine when it is
//! parenthesized, a literal, a rational literal (`1/3r`), a variable, a
//! constant, `self`, or a call (without a block) that is a unary operation
//! (`-a`), or else neither an operator call (`[]` aside) nor a call on a
//! basic literal (`2.to_a`). Flip-flops are not ranges and are left alone.

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{is_basic_literal, is_const, is_literal, is_operator_method, is_variable},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/AmbiguousRange` cop.
pub struct AmbiguousRange;

impl Cop for AmbiguousRange {
    fn name(&self) -> &'static str {
        "Lint/AmbiguousRange"
    }

    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_range_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(range) = node.as_range_node() else {
            return;
        };
        for boundary in [range.left(), range.right()].into_iter().flatten() {
            if is_acceptable(&boundary) {
                continue;
            }
            let (line, col) = file.line_col(boundary.location().start_offset());
            let loc = boundary.location();
            out.push(Offense {
                // Wrap the boundary in parentheses.
                correction: cx
                    .corrections_requested()
                    .then(|| {
                        crate::offense::Correction::from_edits(
                            &file.bytes,
                            vec![
                                (loc.start_offset()..loc.start_offset(), "(".into()),
                                (loc.end_offset()..loc.end_offset(), ")".into()),
                            ],
                        )
                    })
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: "Wrap complex range boundaries with parentheses to avoid ambiguity."
                    .to_string(),
                line,
                end_line: file.last_line_of(boundary.location()),
                col,
            });
        }
    }
}

/// RuboCop's `acceptable?` for a range boundary.
fn is_acceptable(node: &Node<'_>) -> bool {
    match node {
        Node::ParenthesesNode { .. } | Node::SelfNode { .. } => true,
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            is_rational_literal(&call) || is_acceptable_call(&call)
        }
        _ => is_literal(node) || is_variable(node) || is_const(node),
    }
}

/// RuboCop's `acceptable_call?`, with method chains allowed.
fn is_acceptable_call(call: &ruby_prism::CallNode<'_>) -> bool {
    // A call with a literal block is a `block` node in RuboCop's AST, not a
    // call.
    if call
        .block()
        .is_some_and(|block| block.as_block_node().is_some())
    {
        return false;
    }
    let name = call.name().as_slice();
    let unary = is_operator_method(name)
        && call.message_loc().is_some_and(|message| {
            message.start_offset() == call.as_node().location().start_offset()
        });
    if unary {
        return true;
    }
    // `1..2.to_a` calls `to_a` on 2, not on the range.
    if call
        .receiver()
        .is_some_and(|receiver| is_basic_literal(&receiver))
    {
        return false;
    }
    !is_operator_method(name) || name == b"[]"
}

/// RuboCop's `rational_literal?`: `1/3r`, an integer divided by a rational.
fn is_rational_literal(call: &ruby_prism::CallNode<'_>) -> bool {
    call.name().as_slice() == b"/"
        && call
            .receiver()
            .is_some_and(|receiver| receiver.as_integer_node().is_some())
        && call.block().is_none()
        && call.arguments().is_some_and(|arguments| {
            let mut arguments = arguments.arguments().iter();
            arguments
                .next()
                .is_some_and(|argument| argument.as_rational_node().is_some())
                && arguments.next().is_none()
        })
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/ambiguous_range_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! `RequireParenthesesForMethodChains: false`.

    use super::*;
    use crate::pipeline::lint_str;

    const MESSAGE: &str = "Wrap complex range boundaries with parentheses to avoid ambiguity.";

    fn offenses(source: &str) -> Vec<Offense> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/AmbiguousRange")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    /// Run `check` for both range operators, substituting `{op}`.
    fn each_operator(check: impl Fn(&str)) {
        for op in ["..", "..."] {
            check(op);
        }
    }

    fn src(template: &str, op: &str) -> String {
        template.replace("{op}", op)
    }

    #[test]
    fn flags_unparenthesized_logical_boundary_with_message() {
        each_operator(|op| {
            let found = offenses(&src("x || 1{op}2\n", op));
            assert_eq!(found.len(), 1, "{op}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].message, MESSAGE);
        });
    }

    #[test]
    fn flags_operator_boundaries() {
        each_operator(|op| {
            assert_eq!(positions(&src("x - 1{op}2\n", op)), vec![(1, 1)]);
            assert_eq!(positions(&src("(x || 1{op}2)\n", op)), vec![(1, 2)]);
        });
    }

    #[test]
    fn flags_both_sides() {
        each_operator(|op| {
            let col = 7 + op.len() as u32;
            assert_eq!(
                positions(&src("x || 1{op}y || 2\n", op)),
                vec![(1, 1), (1, col)]
            );
            let col = 9 + op.len() as u32;
            assert_eq!(positions(&src("(x || 1){op}y || 2\n", op)), vec![(1, col)]);
        });
    }

    #[test]
    fn flags_endless_and_beginless_ranges() {
        each_operator(|op| {
            assert_eq!(positions(&src("x || 1{op}\n", op)), vec![(1, 1)]);
            let col = 1 + op.len() as u32;
            assert_eq!(positions(&src("{op}y || 1\n", op)), vec![(1, col)]);
        });
    }

    #[test]
    fn flags_method_call_on_basic_literal() {
        each_operator(|op| {
            let col = 2 + op.len() as u32;
            assert_eq!(positions(&src("1{op}2.to_a\n", op)), vec![(1, col)]);
        });
    }

    #[test]
    fn accepts_parenthesized_and_simple_boundaries() {
        each_operator(|op| {
            for template in [
                "x || (1{op}2)\n(x || 1){op}2\n",
                "x[1]{op}2\n",
                "1{op}2\n'a'{op}'z'\n\"#{foo}-#{bar}\"{op}'123-4567'\n`date`{op}'foobar'\n:\"#{foo}-#{bar}\"{op}:baz\n/a/{op}/b/\n42{op}nil\n",
                "@a{op}@b\n",
                "Foo::MIN{op}Foo::MAX\n",
                "self{op}42\n42{op}self\n",
                "a{op}b\n",
                "+a{op}10\n",
                "-a{op}10\n",
                "1/10r{op}1/3r\n",
                "foo.bar{op}10\n",
                "(foo.bar){op}10\n",
            ] {
                let source = src(template, op);
                assert!(offenses(&source).is_empty(), "should not flag {source:?}");
            }
        });
    }

    // The examples below were checked against RuboCop 1.92.

    #[test]
    fn flags_other_complex_boundaries() {
        assert_eq!(positions("foo { 1 }..2\n"), vec![(1, 1)]);
        assert_eq!(positions("a..b.map { 1 }\n"), vec![(1, 4)]);
        assert_eq!(positions("defined?(a)..2\n"), vec![(1, 1)]);
        assert_eq!(positions("x = 1\nx + 1..x * 2\n"), vec![(2, 1), (2, 8)]);
        assert_eq!(positions("a..'b'.upcase\n"), vec![(1, 4)]);
        assert_eq!(positions("a..b&.c(1) == d\n"), vec![(1, 4)]);
        // Regexp references are not variables in RuboCop's AST.
        assert_eq!(positions("$1..$2\n"), vec![(1, 1), (1, 5)]);
        // An operator method called with a dot is not a unary operation.
        assert_eq!(positions("a.!..b\n"), vec![(1, 1)]);
    }

    #[test]
    fn accepts_more_simple_boundaries() {
        for source in [
            "x = 1\nx..x\n",
            "1.0..2.5\n",
            "a..b.c(1)\n",
            "a..b&.c\n",
            "!a..b\n",
            "a.(1)..b\n",
            "foo(&blk)..1\n",
            "[1]..[2]\n",
            "__FILE__..__LINE__\n",
            "::Foo..::Bar\n",
            "$a..$b\n",
            "@@a..@@b\n",
            "if a..b; end\n",
            "not(a)..b\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }
}
