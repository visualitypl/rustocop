//! `Style/CaseEquality` — explicit case equality, preserving regexp match state.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct CaseEquality;
// These are closed immediate values, not inferred types for arbitrary expressions.
pub(super) fn immediate(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::NilNode { .. } | Node::TrueNode { .. } | Node::FalseNode { .. }
    ) || small_integer(n)
        || n.as_array_node().is_some_and(|a| a.elements().is_empty())
}
fn small_integer(n: &Node<'_>) -> bool {
    super::even_odd::integer(n, None).is_some_and(|i| (-(1 << 30)..(1 << 30)).contains(&i))
        && n.as_integer_node().is_some()
}
pub(super) fn core_class(n: &Node<'_>) -> bool {
    ast::is_global_const(
        n,
        &["Array", "Integer", "NilClass", "TrueClass", "FalseClass"],
    )
}
fn integer_range(n: &Node<'_>) -> bool {
    let Some(p) = n.as_parentheses_node() else {
        return false;
    };
    let Some(inner) = super::even_odd::single_group(p.as_node()) else {
        return false;
    };
    inner.as_range_node().is_some_and(|r| {
        r.left().is_some_and(|n| small_integer(&n)) && r.right().is_some_and(|n| small_integer(&n))
    })
}
fn literal_candidate(call: &CallNode<'_>) -> bool {
    let args = ast::call_arguments(call);
    let [rhs] = args.as_slice() else { return false };
    call.name().as_slice() == b"==="
        && !call.is_safe_navigation()
        && call.block().is_none()
        && call.receiver().is_some_and(|lhs| {
            (core_class(&lhs) && immediate(rhs)) || (integer_range(&lhs) && small_integer(rhs))
        })
}
pub(super) fn trusted_call(call: &CallNode<'_>) -> bool {
    if literal_candidate(call) {
        return true;
    }
    let args = ast::call_arguments(call);
    let [arg] = args.as_slice() else { return false };
    !call.is_safe_navigation()
        && call.block().is_none()
        && call.receiver().is_some_and(|lhs| {
            let lhs = super::even_odd::single_group(lhs);
            lhs.is_some_and(|lhs| match call.name().as_slice() {
                b"is_a?" => immediate(&lhs) && core_class(arg),
                b"include?" => {
                    call.receiver().is_some_and(|lhs| integer_range(&lhs)) && small_integer(arg)
                }
                _ => false,
            })
        })
}
fn correct(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> Option<Correction> {
    let call = node.as_call_node()?;
    if !literal_candidate(&call) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.boolean_predicate_safety(root);
    let range = node.location().start_offset()..node.location().end_offset();
    if facts.core_changed
        || !facts.before_effects(range.start)
        || super::correction::contains_comment(file, range.clone())
        || super::correction::touches_directive(file, range.clone())
        || file.heredoc_pending_at(root, range.start)
    {
        return None;
    }
    let lhs = call.receiver()?;
    let args = ast::call_arguments(&call);
    let rhs = &args[0];
    let lhs = std::str::from_utf8(lhs.location().as_slice()).ok()?;
    let rhs = std::str::from_utf8(rhs.location().as_slice()).ok()?;
    let replacement = if core_class(&call.receiver()?) {
        let rhs = if rhs.starts_with(['-', '+']) {
            format!("({rhs})")
        } else {
            rhs.into()
        };
        format!("{rhs}.is_a?({lhs})")
    } else {
        format!("{lhs}.include?({rhs})")
    };
    super::literal_spelling::safe_edit(file, cx, Correction { range, replacement })
}
impl Cop for CaseEquality {
    fn name(&self) -> &'static str {
        "Style/CaseEquality"
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
        if call.name().as_slice() != b"==="
            || call.is_safe_navigation()
            || ast::call_arguments(&call).len() != 1
        {
            return;
        }
        let Some(lhs) = call.receiver() else { return };
        if matches!(
            lhs,
            Node::RegularExpressionNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
        ) {
            return;
        }
        let constant_name = lhs
            .as_constant_read_node()
            .map(|n| n.name())
            .or_else(|| lhs.as_constant_path_node().and_then(|n| n.name()));
        if constant_name.is_some_and(|name| !file.identifier_has_lowercase(name.as_slice())) {
            return;
        }
        let Some(loc) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset() - 1),
            message: "Avoid the use of the case equality operator `===`.".into(),
            correction: if cx.corrections_requested() {
                correct(node, cx, file)
            } else {
                None
            },
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported/adapted RuboCop style/case_equality_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn literal_core_corrections_preserve_precedence() {
        for (s, after) in [
            ("(1..10) === 7", "(1..10).include?(7)"),
            ("(1...10) === -7", "(1...10).include?(-7)"),
            ("Array === []", "[].is_a?(Array)"),
            ("Integer === -7", "(-7).is_a?(Integer)"),
            ("::Integer.===(7)", "7.is_a?(::Integer)"),
            ("TrueClass === true", "true.is_a?(TrueClass)"),
        ] {
            assert_correction(&CaseEquality, s, after);
        }
    }
    #[test]
    fn unknown_dispatch_and_evaluation_stay_reported() {
        for s in [
            "Array === var",
            "Array === a + b",
            "Array === !foo",
            "self.class === var",
            "foo === var",
            "(1..10) === 1.5",
            "(\"a\"..\"z\") === \"ab\"",
            "Other::Array === []",
            "Array=[]; Array === []",
            "def x; end; Integer === 7",
            "effect; Integer === 7",
            "Trigger; Integer === 7",
            "defined?(Trigger::Value); Integer === 7",
            "class Integer; def ===(x); false; end; end; Integer === 7",
        ] {
            let found = correction_offenses(&CaseEquality, s);
            assert!(!found.is_empty(), "{s}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{s}");
        }
    }
    #[test]
    fn literal_block_and_operator_dispatch_still_report_selector() {
        for s in ["Foo.===(x) { work }", "Foo.===(x) do; work; end"] {
            let found = correction_offenses(&CaseEquality, s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 5));
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn unicode_lowercase_uses_pinned_ruby_tables() {
        for (name, lower) in [
            ("A\u{a7cd}", false), // Unicode 16 addition, absent in the pin
            ("A\u{1c8a}", false), // Unicode 16 addition
            ("Aé", true),
            ("A\u{10428}", true), // supplementary Deseret lowercase
            ("A\u{10400}", false),
        ] {
            let s = format!("{name} === x");
            let found = correction_offenses(&CaseEquality, &s);
            assert_eq!(found.len(), usize::from(lower), "{s}");
            if lower {
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, name.chars().count() as u32 + 2)
                );
                assert_eq!(
                    found[0].message,
                    "Avoid the use of the case equality operator `===`."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn regexp_and_non_class_constants_remain_exempt() {
        for s in [
            "/a/ === x",
            "/#{x}/ === x",
            "REGEXP_CONSTANT === x",
            "Foo::BAR === x",
            "Foo&.===(x)",
            "case x; when Array; end",
        ] {
            assert!(correction_offenses(&CaseEquality, s).is_empty(), "{s}");
        }
    }
}
