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
    /// Upstream's Style/InverseMethods lists this cop as
    /// autocorrect-incompatible and runs first.
    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &["Style/InverseMethods"]
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
            .then(|| correct(&call, file))
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
/// RuboCop's corrector.
fn correct(call: &ruby_prism::CallNode<'_>, file: &SourceFile) -> Option<Correction> {
    let loc = call.message_loc()?;
    let receiver = call.receiver()?;
    // `range_with_surrounding_space(selector, side: :right)`
    let mut end = loc.end_offset();
    while matches!(file.bytes.get(end), Some(b' ' | b'\t')) {
        end += 1;
    }
    while file.bytes.get(end) == Some(&b'\n') {
        end += 1;
    }
    let range = loc.start_offset()..end;
    let edit = |range: std::ops::Range<usize>, replacement: &str| Correction {
        range,
        replacement: replacement.into(),
    };
    if let Some(child) = receiver.as_call_node().filter(|c| !c.is_safe_navigation())
        && let Some(inverse) = opposite(child.name().as_slice())
    {
        let selector = child.message_loc()?;
        return super::correction::combine(
            file,
            vec![
                edit(range, ""),
                edit(selector.start_offset()..selector.end_offset(), inverse),
            ],
        );
    }
    if requires_parens(&receiver) {
        let end = call.location().end_offset();
        return super::correction::combine(file, vec![edit(range, "!("), edit(end..end, ")")]);
    }
    Some(edit(range, "!"))
}
/// RuboCop's `requires_parens?`.
fn requires_parens(node: &Node<'_>) -> bool {
    if let Some(call) = node.as_call_node()
        && !call.is_safe_navigation()
        && ast::is_operator_method(call.name().as_slice())
    {
        let selector_first = call
            .message_loc()
            .is_some_and(|m| m.start_offset() == node.location().start_offset());
        // `binary_operation?`, or a unary `~`.
        return !selector_first || call.name().as_slice() == b"~";
    }
    matches!(
        node,
        Node::AndNode { .. }
            | Node::OrNode { .. }
            | Node::RangeNode { .. }
            | Node::FlipFlopNode { .. }
    ) || node
        .as_if_node()
        .is_some_and(|n| n.if_keyword_loc().is_none())
        || crate::cops::ast::is_assignment(node)
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
