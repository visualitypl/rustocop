//! `Style/FloatDivision` — coerce one operand only (detection-only).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct FloatDivision;
impl Cop for FloatDivision {
    fn name(&self) -> &'static str {
        "Style/FloatDivision"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"/" || call.is_safe_navigation() {
            return;
        }
        let Some(left) = call.receiver() else {
            return;
        };
        let args = crate::cops::ast::call_arguments(&call);
        let [right] = args.as_slice() else {
            return;
        };
        let (Some(left), Some(right)) = (coercion_receiver(&left), coercion_receiver(right)) else {
            return;
        };
        if regexp_capture(&left) || regexp_capture(&right) {
            return;
        }
        // Upstream deliberately ignores a divisor whose removed coercion would
        // cross a comment, even though this implementation never removes it.
        let right_call = args[0].as_call_node().unwrap();
        if super::correction::contains_comment(
            file,
            right.location().end_offset()..right_call.message_loc().unwrap().start_offset(),
        ) {
            return;
        }
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            message: "Prefer using `.to_f` on one side only.".into(),
            correction: None,
        });
    }
}
fn coercion_receiver<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let call = node.as_call_node()?;
    (call.name().as_slice() == b"to_f"
        && !call.is_safe_navigation()
        && call.arguments().is_none()
        && call.block().is_none())
    .then(|| call.receiver())
    .flatten()
}
fn regexp_capture(node: &Node<'_>) -> bool {
    if node.as_numbered_reference_read_node().is_some() {
        return true;
    }
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.name().as_slice() != b"last_match"
        || call.is_safe_navigation()
        || call.block().is_some()
        || !call
            .receiver()
            .is_some_and(|r| crate::cops::ast::is_global_const(&r, &["Regexp"]))
    {
        return false;
    }
    matches!(crate::cops::ast::call_arguments(&call).as_slice(), [arg] if arg.as_integer_node().is_some() || crate::cops::ast::negated_numeric_literal(arg).is_some_and(|n| n.as_integer_node().is_some()))
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/float_division_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/FloatDivision")
            .collect()
    }
    #[test]
    fn both_explicit_coercions_are_reported_without_unsafe_edits() {
        for s in [
            "a.to_f / b.to_f",
            "(a - b).to_f / (3 * d / 2).to_f",
            "0.to_f / -1.to_f",
            "custom.to_f / other.to_f",
        ] {
            let found = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].message, "Prefer using `.to_f` on one side only.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn only_exact_global_regexp_capture_shapes_are_exempt() {
        for s in [
            "Other::Regexp.last_match(1).to_f / a.to_f",
            "Regexp.last_match(:key).to_f / a.to_f",
            "$&.to_f / a.to_f",
            "Regexp.last_match(1, 2).to_f / a.to_f",
            "a.to_f / b\n .to_f",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn single_implicit_safe_navigation_and_regexp_coercions_are_allowed() {
        for s in [
            "a.to_f / b",
            "a / b.to_f",
            "a / to_f",
            "to_f / b",
            "to_f / to_f",
            "a.fdiv(b)",
            "1.0 / 0.0",
            "1r / 2i",
            "a&.to_f / b.to_f",
            "a.to_f / b&.to_f",
            "a.to_f(1) / b.to_f",
            "a.to_f / b.to_f { 1 }",
            "$1.to_f / $2.to_f",
            "Regexp.last_match(1).to_f / a.to_f",
            "a.to_f / ::Regexp.last_match(1).to_f",
            "a.to_f / b # keep\n .to_f",
            "a.to_f / b. # keep\n to_f",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
