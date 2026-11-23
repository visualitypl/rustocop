//! `Style/ComparableClamp` — comparison and nested min/max clamp idioms.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ComparableClamp;
impl Cop for ComparableClamp {
    fn name(&self) -> &'static str {
        "Style/ComparableClamp"
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
        let (message, correction) = if array_clamp(node) {
            ("Use `Comparable#clamp` instead.".into(), None)
        } else if let Some((x, low, high)) = conditional_clamp(node, file) {
            let x_source = single_line(x.location().as_slice());
            let receiver = if needs_parentheses(&x) {
                format!("({x_source})")
            } else {
                x_source
            };
            let prefer = format!(
                "{receiver}.clamp({}, {})",
                single_line(low.location().as_slice()),
                single_line(high.location().as_slice())
            );
            let correction = cx
                .corrections_requested()
                .then(|| correct(node, &x, &low, &high, &prefer, cx, file))
                .flatten();
            (
                format!("Use `{prefer}` instead of `if/elsif/else`."),
                correction,
            )
        } else {
            return;
        };
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message,
            correction,
        });
    }
}

fn single(statements: Option<ruby_prism::StatementsNode<'_>>) -> Option<Node<'_>> {
    let s = statements?;
    (s.body().len() == 1)
        .then(|| s.body().iter().next())
        .flatten()
}
fn parts<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>, Node<'pr>)> {
    if let Some(n) = node.as_if_node() {
        let no = n.subsequent()?;
        let no = if let Some(e) = no.as_else_node() {
            single(e.statements())?
        } else {
            no
        };
        Some((n.predicate(), single(n.statements())?, no))
    } else {
        let n = node.as_unless_node()?;
        Some((
            n.predicate(),
            single(n.else_clause()?.statements())?,
            single(n.statements())?,
        ))
    }
}
fn comparison<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>, bool)> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation() || call.block().is_some() {
        return None;
    }
    let less = match call.name().as_slice() {
        b"<" => true,
        b">" => false,
        _ => return None,
    };
    let mut args = ast::call_arguments(&call).into_iter();
    let rhs = args.next()?;
    if args.next().is_some() {
        return None;
    }
    Some((call.receiver()?, rhs, less))
}
fn conditional_clamp<'pr>(
    node: &Node<'pr>,
    file: &SourceFile,
) -> Option<(Node<'pr>, Node<'pr>, Node<'pr>)> {
    let (c1, a, rest) = parts(node)?;
    let (c2, b, x) = parts(&rest)?;
    let (l1, r1, less1) = comparison(&c1)?;
    let (l2, r2, less2) = comparison(&c2)?;
    let eq = |a: &Node<'_>, b: &Node<'_>| {
        ast::fingerprint_with_source_lines(a, file) == ast::fingerprint_with_source_lines(b, file)
    };
    // `x < low` / `low > x`, followed by either upper-bound orientation;
    // or the same two comparisons in the opposite branch order.
    let first_low = (less1 && eq(&l1, &x) && eq(&r1, &a)) || (!less1 && eq(&r1, &x) && eq(&l1, &a));
    let first_high =
        (less1 && eq(&r1, &x) && eq(&l1, &a)) || (!less1 && eq(&l1, &x) && eq(&r1, &a));
    let second_low =
        (less2 && eq(&l2, &x) && eq(&r2, &b)) || (!less2 && eq(&r2, &x) && eq(&l2, &b));
    let second_high =
        (less2 && eq(&r2, &x) && eq(&l2, &b)) || (!less2 && eq(&l2, &x) && eq(&r2, &b));
    if first_low && second_high {
        Some((x, a, b))
    } else if first_high && second_low {
        Some((x, b, a))
    } else {
        None
    }
}
fn array_clamp(node: &Node<'_>) -> bool {
    fn pair<'pr>(node: &Node<'pr>, method: &[u8], outer: bool) -> Option<Vec<Node<'pr>>> {
        let call = node.as_call_node()?;
        if call.name().as_slice() != method
            || call.is_safe_navigation()
            || call
                .block()
                .is_some_and(|b| !outer || b.as_block_node().is_none())
            || !ast::call_arguments(&call).is_empty()
        {
            return None;
        }
        let a = call.receiver()?.as_array_node()?;
        (a.elements().len() == 2).then(|| a.elements().iter().collect())
    }
    for (outer, inner) in [(b"min".as_slice(), b"max".as_slice()), (b"max", b"min")] {
        // A literal block wraps the outer send in RuboCop, but wraps an
        // inner send *inside* the Array, where the pattern no longer matches.
        if pair(node, outer, true)
            .is_some_and(|elements| elements.iter().any(|n| pair(n, inner, false).is_some()))
        {
            return true;
        }
    }
    false
}
fn needs_parentheses(node: &Node<'_>) -> bool {
    (node.as_integer_node().is_some() && node.location().as_slice().starts_with(b"-"))
        || matches!(
            node,
            Node::AndNode { .. }
                | Node::OrNode { .. }
                | Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::RangeNode { .. }
        )
        || node
            .as_call_node()
            .is_some_and(|c| ast::is_operator_method(c.name().as_slice()))
        || matches!(
            node,
            Node::LocalVariableWriteNode { .. }
                | Node::InstanceVariableWriteNode { .. }
                | Node::ClassVariableWriteNode { .. }
                | Node::GlobalVariableWriteNode { .. }
                | Node::ConstantWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
                | Node::MultiWriteNode { .. }
        )
}
fn integer(node: &Node<'_>) -> Option<i32> {
    let value: i32 = node.as_integer_node()?.value().try_into().ok()?;
    ((-(1 << 30)..(1 << 30)).contains(&value)).then_some(value)
}
fn correct(
    node: &Node<'_>,
    x: &Node<'_>,
    low: &Node<'_>,
    high: &Node<'_>,
    prefer: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    integer(x)?;
    if integer(low)? > integer(high)? {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.comparable_clamp_safety(root);
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    if facts.changed
        || facts.first_effect.is_some_and(|end| end <= range.start)
        || super::correction::contains_comment(file, range.clone())
        || file.heredoc_pending_at(root, range.start)
    {
        return None;
    }
    let replacement = if node
        .as_if_node()
        .and_then(|n| n.if_keyword_loc())
        .is_some_and(|l| l.as_slice() == b"elsif")
    {
        let end = node.as_if_node()?.end_keyword_loc()?;
        let (_, col) = file.line_col(range.start);
        let nl = if file.bytes[range.clone()].windows(2).any(|w| w == b"\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        format!(
            "else{nl}{}{prefer}{nl}{}{}",
            " ".repeat(col as usize + 1),
            " ".repeat(col as usize - 1),
            std::str::from_utf8(end.as_slice()).ok()?
        )
    } else {
        prefer.into()
    };
    Some(Correction { range, replacement })
}

/// Correction-only closed eager core proof, cached once per source. Unknown
/// calls invalidate subsequent edits; declarations and deferred scopes invalidate
/// all edits. Trust only comparisons/clamp on immediate small Integers.
#[derive(Default)]
pub(crate) struct Safety {
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        if let Some(c) = node.as_call_node() {
            let trusted = c.receiver().is_some_and(|n| integer(&n).is_some())
                && !c.is_safe_navigation()
                && c.block().is_none()
                && matches!(c.name().as_slice(), b"<" | b">" | b"clamp")
                && ast::call_arguments(&c).iter().all(|n| integer(n).is_some());
            if !trusted {
                self.changed |= c.block().is_some();
                let end = c.location().end_offset();
                self.first_effect = Some(self.first_effect.map_or(end, |old| old.min(end)));
            }
        } else {
            self.changed |= !ast::is_basic_literal(&node)
                && !matches!(
                    node,
                    Node::ProgramNode { .. }
                        | Node::ArrayNode { .. }
                        | Node::StatementsNode { .. }
                        | Node::ArgumentsNode { .. }
                        | Node::IfNode { .. }
                        | Node::ElseNode { .. }
                        | Node::UnlessNode { .. }
                        | Node::ParenthesesNode { .. }
                        | Node::LocalVariableReadNode { .. }
                        | Node::LocalVariableWriteNode { .. }
                        | Node::SourceLineNode { .. }
                );
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/comparable_clamp_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ComparableClamp.name())
            .collect()
    }
    pub(super) fn conditional(first: &str, a: &str, second: &str, b: &str, x: &str) -> String {
        format!("if {first}\n  {a}\nelsif {second}\n  {b}\nelse\n  {x}\nend\n")
    }
    #[test]
    fn eight_comparison_orders_and_nested_elsif() {
        for (first, a, second, b) in [
            ("x < low", "low", "high < x", "high"),
            ("low > x", "low", "high < x", "high"),
            ("x < low", "low", "x > high", "high"),
            ("low > x", "low", "x > high", "high"),
            ("high < x", "high", "x < low", "low"),
            ("x > high", "high", "x < low", "low"),
            ("high < x", "high", "low > x", "low"),
            ("x > high", "high", "low > x", "low"),
        ] {
            let s = conditional(first, a, second, b, "x");
            for (s, line) in [(s.clone(), 1), (format!("if condition\n  work\nels{s}"), 3)] {
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col), (line, 1));
                assert_eq!(
                    o[0].message,
                    "Use `x.clamp(low, high)` instead of `if/elsif/else`."
                );
                assert!(
                    correction_offenses(&ComparableClamp, &s)[0]
                        .correction
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn eight_min_max_orders_are_detection_only() {
        for s in [
            "[[x, low].max, high].min",
            "[[low, x].max, high].min",
            "[high, [x, low].max].min",
            "[high, [low, x].max].min",
            "[[x, high].min, low].max",
            "[[high, x].min, low].max",
            "[low, [x, high].min].max",
            "[low, [high, x].min].max",
        ] {
            let o = offenses(&format!("\n  {s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, 3));
            assert_eq!(o[0].message, "Use `Comparable#clamp` instead.");
            assert!(
                correction_offenses(&ComparableClamp, s)[0]
                    .correction
                    .is_none()
            );
        }
    }
    #[test]
    fn operator_receiver_is_parenthesized() {
        let s = conditional("a + b < low", "low", "high < a + b", "high", "a + b");
        assert_eq!(
            offenses(&s)[0].message,
            "Use `(a + b).clamp(low, high)` instead of `if/elsif/else`."
        );
    }
    #[test]
    fn rejects_lookalikes() {
        for s in [
            "[low, high].min",
            "[[x, low].max, high].max",
            "[[x, low, y].max, high].min",
            "[[x, low].max, high, y].min",
            "[[x, low].max(2), high].min",
            "[[x, low].max { |a,b| a <=> b }, high].min",
            "[[x, low].max, high]&.min",
            "[[x, low]&.max, high].min",
            "if x < low; x; elsif high < x; x; else; x; end",
            "if x <= low; low; elsif high < x; high; else; x; end",
            "if x < low; low; log; elsif high < x; high; else; x; end",
            "if x < low; low; elsif high < x; high; else; y; end",
            "if x&.<(low); low; elsif high < x; high; else; x; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn outer_comparator_blocks_still_report_the_send_without_a_fix() {
        let s = "[[x, low].max, high].min do |a,b|\n  b <=> a\nend\n";
        let o = offenses(s);
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 1, 1));
        assert_eq!(o[0].message, "Use `Comparable#clamp` instead.");
        assert!(
            correction_offenses(&ComparableClamp, s)[0]
                .correction
                .is_none()
        );
    }
    #[test]
    fn grouped_conditions_do_not_match_the_pinned_send_pattern() {
        assert!(offenses("if (x < low); low; elsif (high < x); high; else; x; end").is_empty());
    }
    #[test]
    fn negative_integer_receiver_keeps_unary_binding() {
        assert_correction(
            &ComparableClamp,
            "if -2 < -3; -3; elsif -1 < -2; -1; else; -2; end\n",
            "(-2).clamp(-3, -1)\n",
        );
    }
    #[test]
    fn nested_if_ternary_and_unless_structural_forms_are_handled_without_upstream_crashes() {
        for s in [
            "2 < 1 ? 1 : 3 < 2 ? 3 : 2\n",
            "if 2 < 1; 1; else; if 3 < 2; 3; else; 2; end; end\n",
            "unless 2 < 1; if 3 < 2; 3; else; 2; end; else; 1; end\n",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
            assert_correction(&ComparableClamp, s, "2.clamp(1, 3)\n");
        }
    }
    #[test]
    fn source_line_equality_uses_the_evaluated_line() {
        assert_eq!(
            offenses("if __LINE__ < low; low; elsif high < __LINE__; high; else; __LINE__; end")
                .len(),
            1
        );
        assert!(
            offenses("if __LINE__ < low\n low\nelsif high < __LINE__\n high\nelse\n __LINE__\nend")
                .is_empty()
        );
    }
    #[test]
    fn canonical_integer_corrections() {
        assert_correction(
            &ComparableClamp,
            "if 2 < 1; 1; elsif 3 < 2; 3; else; 2; end\n",
            "2.clamp(1, 3)\n",
        );
        assert_correction(
            &ComparableClamp,
            "if 0 < 1\n  1\nelsif 3 < 0\n  3\nelse\n  0\nend\n",
            "0.clamp(1, 3)\n",
        );
    }
}
