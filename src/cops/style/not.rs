//! `Style/Not` — keyword negation, with binding-preserving safe fixes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Not;
impl Cop for Not {
    fn name(&self) -> &'static str {
        "Style/Not"
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
        let Some(loc) = call.message_loc().filter(|l| l.as_slice() == b"not") else {
            return;
        };
        if call.name().as_slice() != b"!" || call.call_operator_loc().is_some() {
            return;
        }
        let (line, col) = file.line_col(loc.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| correct(&call, cx, file))
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Use `!` instead of `not`.".into(),
            correction,
        });
    }
}
fn opposite(name: &[u8]) -> Option<&'static str> {
    Some(match name {
        b"==" => "!=",
        b"!=" => "==",
        b"<=" => ">",
        b">" => "<=",
        b"<" => ">=",
        b">=" => "<",
        _ => return None,
    })
}
fn small_integer(node: &Node<'_>) -> bool {
    node.as_integer_node().is_some()
        && super::even_odd::integer(node, None)
            .is_some_and(|n| (-(1 << 30)..(1 << 30)).contains(&n))
}
fn correct(
    call: &ruby_prism::CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = call.message_loc()?;
    let receiver = call.receiver()?;
    let root = cx.ancestors().next_back()?;
    let end = call
        .opening_loc()
        .map_or(receiver.location().start_offset(), |l| l.start_offset());
    let gap = loc.end_offset()..end;
    if file.bytes[gap.clone()].contains(&b'\n')
        || super::correction::contains_comment(file, gap)
        || file.contains_heredoc(root, &call.as_node())
        || file.heredoc_pending_at(root, loc.start_offset())
    {
        return None;
    }
    let mut edits = Vec::new();
    let grouped = call.opening_loc().is_some();
    if !grouped
        && let Some(child) = receiver.as_call_node()
        && let Some(inverse) = opposite(child.name().as_slice())
    {
        // Replacing !(x < y) by x >= y is not valid for arbitrary dispatch,
        // nonboolean operator results, NaN, or partial-order values.
        let args = ast::call_arguments(&child);
        if child.is_safe_navigation()
            || child.block().is_some()
            || !child.receiver().is_some_and(|n| small_integer(&n))
            || !matches!(args.as_slice(), [rhs] if small_integer(rhs))
            || !super::nil_comparison::core_safe(&call.as_node(), cx, file)
        {
            return None;
        }
        let selector = child.message_loc()?;
        // An enclosing `not` can simultaneously become `!`. Retain the
        // comparison's grouping so that tighter prefix negation cannot capture
        // just its left operand after this `not` disappears.
        let nested = cx.parent().and_then(|n| n.as_call_node()).is_some_and(|p| {
            p.name().as_slice() == b"!"
                && p.call_operator_loc().is_none()
                && p.opening_loc().is_none()
        });
        edits.push(Correction {
            range: loc.start_offset()..end,
            replacement: if nested { "(" } else { "" }.into(),
        });
        edits.push(Correction {
            range: selector.start_offset()..selector.end_offset(),
            replacement: inverse.into(),
        });
        if nested {
            let end = call.location().end_offset();
            edits.push(Correction {
                range: end..end,
                replacement: ")".into(),
            });
        }
    } else {
        let parens = !grouped && requires_parens(&receiver);
        edits.push(Correction {
            range: loc.start_offset()..end,
            replacement: if parens { "!(" } else { "!" }.into(),
        });
        if parens {
            let end = call.location().end_offset();
            edits.push(Correction {
                range: end..end,
                replacement: ")".into(),
            });
        }
    }
    let edit = super::correction::combine(file, edits)?;
    super::literal_spelling::safe_edit(file, cx, edit)
}
fn requires_parens(node: &Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        return call.equal_loc().is_some()
            || (ast::is_operator_method(call.name().as_slice())
                && !matches!(
                    call.name().as_slice(),
                    b"[]" | b"[]=" | b"!" | b"+@" | b"-@"
                ));
    }
    matches!(node, Node::MatchPredicateNode { .. })
        || super::nil_comparison::operator_expression(node, false)
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/not_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/Not")
            .collect()
    }
    #[test]
    fn flags_keyword_at_selector() {
        for (s, col) in [
            ("not test", 1),
            ("not(test)", 1),
            ("x = 10 if not y", 11),
            ("x = (not ~a)", 6),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1);
            assert_eq!((o[0].line, o[0].col), (1, col));
            assert_eq!(o[0].message, "Use `!` instead of `not`.");
        }
    }
    #[test]
    fn corrections_preserve_operand_binding_and_physical_lines() {
        use crate::cops::testing::correction_offenses;
        for (s, expected) in [
            ("not test", "!test"),
            ("not(test)", "!(test)"),
            ("not(x < y)", "!(x < y)"),
            ("not a >> b", "!(a >> b)"),
            ("not a && b", "!(a && b)"),
            ("not a || b", "!(a || b)"),
            ("not a ? b : c", "!(a ? b : c)"),
            ("x if not 1..5", "x if !(1..5)"),
            ("x if not a = 5", "x if !(a = 5)"),
            ("x = (not ~a)", "x = (!(~a))"),
            ("not (~a).zero?", "!(~a).zero?"),
            ("not foo 1", "!foo 1"),
            ("not 1 < 2", "1 >= 2"),
        ] {
            let o = correction_offenses(&Not, s);
            let c = o[0].correction.as_ref().expect(s);
            assert_eq!(
                format!(
                    "{}{}{}",
                    &s[..c.range.start],
                    c.replacement,
                    &s[c.range.end..]
                ),
                expected
            );
        }
        for s in [
            "not x < y",
            "not x == y",
            "not # comment\n x",
            "not\n x",
            "not <<~TEXT\n hi\nTEXT\n",
        ] {
            assert!(correction_offenses(&Not, s)[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn ignores_other_negations_and_names() {
        for s in [
            "!test",
            "x.!",
            "x&.!",
            "notable",
            "object.not",
            ":not",
            "# not x\n'not x'",
            "def not; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
