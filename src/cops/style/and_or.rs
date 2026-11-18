//! `Style/AndOr` — keyword operators in conditions only; unsafe correction.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AndOr;
impl Cop for AndOr {
    fn name(&self) -> &'static str {
        "Style/AndOr"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let op = if let Some(n) = node.as_and_node() {
            n.operator_loc()
        } else if let Some(n) = node.as_or_node() {
            n.operator_loc()
        } else {
            return;
        };
        let (current, prefer) = match op.as_slice() {
            b"and" => ("and", "&&"),
            b"or" => ("or", "||"),
            _ => return,
        };
        let start = node.location().start_offset();
        let end = node.location().end_offset();
        let in_condition = cx
            .ancestors()
            .zip(cx.ancestors().skip(1))
            .any(|(ancestor, parent)| {
                // Prism models pattern guards as If/Unless, unlike RuboCop's
                // if_guard/unless_guard. Those guards are not on_if callbacks.
                if parent.as_in_node().is_some() {
                    return false;
                }
                let predicate = if let Some(n) = ancestor.as_if_node() {
                    Some(n.predicate())
                } else if let Some(n) = ancestor.as_unless_node() {
                    Some(n.predicate())
                } else if let Some(n) = ancestor.as_while_node() {
                    Some(n.predicate())
                } else {
                    ancestor.as_until_node().map(|n| n.predicate())
                };
                predicate.is_some_and(|n| {
                    n.location().start_offset() <= start && end <= n.location().end_offset()
                })
            });
        if !in_condition {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| correction(node, &op, prefer, cx.parent(), file))
            .flatten();
        let (line, col) = file.line_col(op.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(op.end_offset() - 1),
            message: format!("Use `{prefer}` instead of `{current}`."),
            correction,
        });
    }
}

type Range = std::ops::Range<usize>;

/// The edits of one correction: wraps, and other insertions/replacements.
#[derive(Default)]
struct Edits {
    wraps: Vec<Range>,
    other: Vec<(Range, String)>,
}

/// Upstream's `process_logical_operator` corrector block.
fn correction(
    node: &Node<'_>,
    operator: &ruby_prism::Location<'_>,
    prefer: &str,
    parent: Option<&Node<'_>>,
    file: &SourceFile,
) -> Option<Correction> {
    let (left, right, or) = match node.as_and_node() {
        Some(n) => (n.left(), n.right(), false),
        None => {
            let n = node.as_or_node()?;
            (n.left(), n.right(), true)
        }
    };
    let mut edits = Edits::default();
    for (expr, last) in [(&left, false), (&right, true)] {
        if let Some(call) = expr.as_call_node() {
            correct_send(expr, &call, last, &mut edits);
        } else if matches!(
            expr,
            Node::ReturnNode { .. }
                | Node::NextNode { .. }
                | Node::BreakNode { .. }
                | Node::YieldNode { .. }
        ) || is_assignment(expr)
        {
            correct_other(expr, last, &mut edits);
        }
    }
    edits.other.push((
        operator.start_offset()..operator.end_offset(),
        prefer.to_owned(),
    ));
    // keep_operator_precedence
    let span = |n: &Node<'_>| n.location().start_offset()..n.location().end_offset();
    if or && parent.is_some_and(|p| p.as_and_node().is_some()) {
        edits.wraps.push(span(node));
    } else if !or && right.as_or_node().is_some() {
        edits.wraps.push(span(&right));
    }
    // Nested wraps: the larger range's parentheses go outside.
    let mut opens = edits.wraps.clone();
    opens.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut closes = edits.wraps;
    closes.sort_by_key(|r| (r.end, std::cmp::Reverse(r.start)));
    let mut all: Vec<(Range, String)> = opens
        .into_iter()
        .map(|r| (r.start..r.start, "(".to_owned()))
        .collect();
    all.extend(edits.other);
    all.extend(closes.into_iter().map(|r| (r.end..r.end, ")".to_owned())));
    Correction::from_edits(&file.bytes, all)
}

/// `correct_send`.
fn correct_send(node: &Node<'_>, call: &ruby_prism::CallNode<'_>, last: bool, edits: &mut Edits) {
    let name = call.name().as_slice();
    if name == b"!" {
        // correct_not: `!x` recurses into a call receiver; `not x` wraps.
        if call.message_loc().is_some_and(|m| m.as_slice() == b"not") {
            return correct_other(node, last, edits);
        }
        if let Some(receiver) = call.receiver()
            && let Some(inner) = receiver.as_call_node()
        {
            correct_send(&receiver, &inner, false, edits);
        }
        return;
    }
    let arguments = ast::call_arguments(call);
    if call.is_attribute_write() {
        // correct_setter
        let (Some(receiver), Some(last_argument)) = (call.receiver(), arguments.last()) else {
            return;
        };
        let start = receiver.location().start_offset();
        let end = last_argument.location().end_offset();
        edits.other.push((start..start, "(".into()));
        edits.other.push((end..end, ")".into()));
        return;
    }
    if matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<") {
        return correct_other(node, last, edits);
    }
    // correctable_send?
    if call.opening_loc().is_some() || arguments.is_empty() || name == b"[]" {
        return;
    }
    let Some(selector) = call.message_loc() else {
        return;
    };
    let begin = selector.end_offset();
    let source = node.location().as_slice();
    let predicate_glued = source
        .windows(2)
        .any(|w| w[0] == b'?' && !matches!(w[1], b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\x0b'));
    let end = if predicate_glued { begin } else { begin + 1 };
    let close = arguments.last().unwrap().location().end_offset();
    edits.other.push((begin..begin, "(".into()));
    edits.other.push((begin..end, String::new()));
    edits.other.push((close..close, ")".into()));
}

/// `correct_other`: wrap, unless a parenthesized call or a bare keyword
/// that is the operator's last child.
fn correct_other(node: &Node<'_>, last: bool, edits: &mut Edits) {
    let parenthesized = match node {
        Node::CallNode { .. } => node
            .as_call_node()
            .unwrap()
            .opening_loc()
            .is_some_and(|o| o.as_slice() == b"("),
        Node::YieldNode { .. } => node.as_yield_node().unwrap().lparen_loc().is_some(),
        _ => false,
    };
    if parenthesized {
        return;
    }
    let childless = match node {
        Node::ReturnNode { .. } => node.as_return_node().unwrap().arguments().is_none(),
        Node::NextNode { .. } => node.as_next_node().unwrap().arguments().is_none(),
        Node::BreakNode { .. } => node.as_break_node().unwrap().arguments().is_none(),
        Node::YieldNode { .. } => node.as_yield_node().unwrap().arguments().is_none(),
        _ => false,
    };
    if childless && last {
        return;
    }
    let loc = node.location();
    edits.wraps.push(loc.start_offset()..loc.end_offset());
}

/// rubocop-ast's `assignment?`: variable, constant, multiple and shorthand
/// assignments (a setter call is a send).
fn is_assignment(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::MultiWriteNode { .. }
            | Node::LocalVariableOperatorWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::ClassVariableOperatorWriteNode { .. }
            | Node::GlobalVariableOperatorWriteNode { .. }
            | Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::CallOperatorWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::LocalVariableAndWriteNode { .. }
            | Node::InstanceVariableAndWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::ConstantAndWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
    )
}

#[cfg(test)]
mod tests {
    //! Ported/adapted RuboCop style/and_or_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn only_conditional_subtrees_with_exact_operator_locations() {
        for (s, line, col) in [
            ("if a and b; end", 1, 6),
            ("work unless a or b", 1, 15),
            ("begin; work; end while a and b", 1, 26),
            ("if foo { a and b }; end", 1, 12),
            ("(a and b) ? x : y", 1, 4),
        ] {
            let found = correction_offenses(&AndOr, s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{s}");
            assert!(found[0].correction.is_some());
        }
    }
    #[test]
    fn bodies_flow_control_and_pattern_guards_are_not_if_conditions() {
        for s in [
            "foo and return",
            "x = a or b",
            "if a; b and c; end",
            "case x; in y if (a or b); end",
            "case x; in y unless (a and b); end",
            "work if a && b",
        ] {
            assert!(correction_offenses(&AndOr, s).is_empty(), "{s}");
        }
    }
}
