//! `Style/NilComparison` — fixed predicate spelling of comparisons with nil.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NilComparison;
impl Cop for NilComparison {
    fn name(&self) -> &'static str {
        "Style/NilComparison"
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !matches!(call.name().as_slice(), b"==" | b"===") || !compares_nil(&call) {
            return;
        }
        let Some(loc) = call.message_loc() else {
            return;
        };
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
            end_line: file.last_line_of(loc),
            message: "Prefer the use of the `nil?` predicate.".into(),
            correction,
        });
    }
}
fn correct(
    call: &ruby_prism::CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !trusted_call(call) || !core_safe(&call.as_node(), cx, file) {
        return None;
    }
    let receiver = call.receiver()?;
    let source = std::str::from_utf8(receiver.location().as_slice()).ok()?;
    let replacement = if source.starts_with(['-', '+']) {
        format!("({source}).nil?")
    } else {
        format!("{source}.nil?")
    };
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range: call.location().start_offset()..ast::source_end(&call.as_node()),
            replacement,
        },
    )
}

// This proof deliberately does not infer a receiver's type from its name or
// assume that user-defined ==, ===, != and nil? are interchangeable.
pub(super) fn trusted_call(call: &ruby_prism::CallNode<'_>) -> bool {
    matches!(call.name().as_slice(), b"==" | b"===" | b"!=")
        && compares_nil(call)
        && call.block().is_none()
        && call
            .receiver()
            .and_then(super::even_odd::single_group)
            .is_some_and(|n| super::case_equality::immediate(&n))
}
pub(super) fn core_safe(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let Some(root) = cx.ancestors().next_back() else {
        return false;
    };
    let facts = file.boolean_predicate_safety(root);
    let range = node.location().start_offset()..node.location().end_offset();
    !facts.core_changed
        && facts.before_effects(range.start)
        && !super::correction::contains_comment(file, range.clone())
        && !super::correction::touches_directive(file, range.clone())
        && !file.bytes[range.clone()].contains(&b'\n')
        && !file.heredoc_pending_at(root, range.start)
}

pub(super) fn compares_nil(call: &ruby_prism::CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call.receiver().is_some()
        && matches!(ast::call_arguments(call).as_slice(), [arg] if arg.as_nil_node().is_some())
}

// Prism retains explicit grouping, which already supplies the required binding.
pub(super) fn operator_expression(node: &Node<'_>, index_is_operator: bool) -> bool {
    node.as_call_node().is_some_and(|c| {
        ast::is_operator_method(c.name().as_slice())
            && (index_is_operator || !matches!(c.name().as_slice(), b"[]" | b"[]="))
    }) || matches!(
        node,
        Node::AndNode { .. }
            | Node::OrNode { .. }
            | Node::RangeNode { .. }
            | Node::FlipFlopNode { .. }
    ) || node
        .as_if_node()
        .is_some_and(|n| n.if_keyword_loc().is_none())
        || super::redundant_parentheses::is_assignment(node)
        || node.as_call_node().is_some_and(|c| c.equal_loc().is_some())
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/nil_comparison_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/NilComparison")
            .collect()
    }
    #[test]
    fn flags_predicate_style_at_selector() {
        for s in [
            "x == nil",
            "x === nil",
            "x.==(nil)",
            "x.===(nil)",
            "!x == nil",
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                (o[0].line, o[0].col),
                (1, if s.starts_with('!') { 4 } else { 3 })
            );
            assert_eq!(o[0].message, "Prefer the use of the `nil?` predicate.");
        }
    }
    #[test]
    fn corrects_closed_literals_but_not_unknown_dispatch() {
        use crate::cops::testing::correction_offenses;
        for (s, expected) in [
            ("nil == nil", "nil.nil?"),
            ("false === nil", "false.nil?"),
            ("1.==(nil)", "1.nil?"),
            ("-1 == nil", "(-1).nil?"),
            ("(1) == nil", "(1).nil?"),
        ] {
            let o = correction_offenses(&NilComparison, s);
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
            "x == nil",
            "!x == nil",
            "object.===(nil)",
            "class Integer; def nil?; true; end; end; 1 == nil",
            "load 'patch.rb'; 1 == nil",
            "1 == # comment\n nil",
        ] {
            assert!(
                correction_offenses(&NilComparison, s)[0]
                    .correction
                    .is_none(),
                "{s}"
            );
        }
    }
    #[test]
    fn ignores_other_arguments_and_safe_navigation() {
        for s in [
            "x.nil?",
            "nil == x",
            "x != nil",
            "x == false",
            "x&.==(nil)",
            "x.==(nil, nil)",
            "x.==(nil, &block)",
            "x == (nil)",
            "# x == nil\n'x == nil'",
            "def ==(x); end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
