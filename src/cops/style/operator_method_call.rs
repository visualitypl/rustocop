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
                .then(|| correction(node, rhs, cx, file))
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

/// RuboCop's corrector: the dot becomes a space, a space follows the
/// operator when the argument touches it (or `(` follows `/`), and a call
/// that is the receiver of another loses its parentheses and is wrapped
/// in them whole (`ParenthesesCorrector`).
fn correction(
    node: &Node<'_>,
    rhs: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let call = node.as_call_node()?;
    let dot = call.call_operator_loc()?;
    let selector = call.message_loc()?;
    let bytes = &file.bytes;
    let insert = |at: usize, text: &str| Correction {
        range: at..at,
        replacement: text.into(),
    };
    let mut edits = vec![Correction {
        range: dot.start_offset()..dot.end_offset(),
        replacement: " ".into(),
    }];
    // Prism's argument list is no node of RuboCop's AST.
    let parent = cx.ancestors().find(|n| n.as_arguments_node().is_none());
    let parent_call = parent.as_ref().and_then(|n| n.as_call_node());
    let first_argument = parent_call.as_ref().is_some_and(|p| {
        ast::call_arguments(p)
            .first()
            .is_some_and(|a| a.location().start_offset() == node.location().start_offset())
    });
    let start = node.location().start_offset();
    let end = ast::source_end(node);
    if parent_call.is_some() && !first_argument {
        if let (Some(open), Some(close)) = (call.opening_loc(), call.closing_loc()) {
            let mut right = open.end_offset();
            while matches!(bytes.get(right), Some(b' ' | b'\t')) {
                right += 1;
            }
            while bytes.get(right).is_some_and(u8::is_ascii_whitespace) {
                right += 1;
            }
            edits.push(Correction {
                range: open.start_offset()..right,
                replacement: String::new(),
            });
            let body_end = rhs.location().end_offset();
            let between = &bytes[body_end.min(close.start_offset())..close.start_offset()];
            let after = {
                let line_end = bytes[close.end_offset()..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(bytes.len(), |n| close.end_offset() + n);
                bytes[close.end_offset()..line_end].trim_ascii_start()
            };
            let comment_above = body_end < close.start_offset()
                && between.windows(1).any(|w| w == b"#")
                && between[between.iter().position(|&b| b == b'#').unwrap()..].contains(&b'\n')
                && !after.is_empty()
                && !after.starts_with(b"#");
            let mut left = close.start_offset();
            while left > 0 && matches!(bytes[left - 1], b' ' | b'\t') {
                left -= 1;
            }
            if !comment_above {
                while left > 0 && bytes[left - 1] == b'\n' {
                    left -= 1;
                }
            }
            edits.push(Correction {
                range: left..close.end_offset(),
                replacement: String::new(),
            });
        }
        edits.push(insert(selector.end_offset(), " "));
        edits.push(insert(start, "("));
        edits.push(insert(end, ")"));
    }
    let rhs_start = rhs.location().start_offset();
    let space_after = selector.end_offset() == rhs_start
        || parent_call.is_none()
            && call.name().as_slice() == b"/"
            && &bytes[selector.end_offset()..rhs_start] == b"(";
    if space_after {
        edits.push(insert(selector.end_offset(), " "));
    }
    super::correction::combine(file, edits)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's corresponding Style specs (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, offense::Offense, pipeline::lint_str};
    #[test]
    fn corrects_as_rubocop() {
        for (before, after) in [
            ("foo.+(bar)\n", "foo +(bar)\n"),
            ("foo bar.| baz\n", "foo bar | baz\n"),
            ("foo.+(bar).baz\n", "(foo + bar).baz\n"),
            ("foo.+bar\n", "foo + bar\n"),
        ] {
            assert_correction(&OperatorMethodCall, before, after);
        }
    }
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
