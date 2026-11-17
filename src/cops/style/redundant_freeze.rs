//! `Style/RedundantFreeze` — immutable literals and pinned immutable-result idioms.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantFreeze;
impl Cop for RedundantFreeze {
    fn name(&self) -> &'static str {
        "Style/RedundantFreeze"
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
        if call.is_safe_navigation() || call.name().as_slice() != b"freeze" {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let inner = strip(&receiver);
        if !immutable(inner.as_ref().unwrap_or(&receiver), file) && !operation(&receiver) {
            return;
        }
        let loc = call.location();
        let (line, col) = file.line_col(loc.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                let dot = call.call_operator_loc()?;
                Some(Correction {
                    range: dot.start_offset()..call.message_loc()?.end_offset(),
                    replacement: String::new(),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: "Do not freeze immutable objects, as freezing them has no effect.".into(),
            correction,
        });
    }
}
fn strip<'pr>(n: &Node<'pr>) -> Option<Node<'pr>> {
    n.as_parentheses_node()
        .and_then(|p| p.body())
        .and_then(|b| b.as_statements_node().and_then(|s| s.body().iter().next()))
}
fn immutable(n: &Node<'_>, file: &SourceFile) -> bool {
    matches!(
        n,
        Node::IntegerNode { .. }
            | Node::SourceLineNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::SymbolNode { .. }
            | Node::InterpolatedSymbolNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::RegularExpressionNode { .. }
            | Node::InterpolatedRegularExpressionNode { .. }
            | Node::RangeNode { .. }
    ) || (file.frozen_string_setting() == Some(true)
        && (n.as_source_file_node().is_some()
            || n.as_string_node().is_some()
            || n.as_interpolated_string_node()
                .is_some_and(|s| s.parts().iter().all(|p| p.as_string_node().is_some()))))
}
fn numeric(n: &Node<'_>) -> bool {
    if matches!(n, Node::IntegerNode { .. } | Node::FloatNode { .. }) {
        return true;
    }
    if let Some(inner) = strip(n) {
        return numeric(&inner);
    }
    let Some(c) = n.as_call_node() else {
        return false;
    };
    let args = ast::call_arguments(&c);
    matches!(
        c.name().as_slice(),
        b"+" | b"-" | b"*" | b"**" | b"/" | b"%"
    ) && c.receiver().is_some_and(|r| numeric(&r))
        && matches!(args.as_slice(), [a] if numeric(a))
}
fn operation(n: &Node<'_>) -> bool {
    if let Some(c) = n.as_call_node() {
        return !c.is_safe_navigation()
            && matches!(c.name().as_slice(), b"count" | b"length" | b"size");
    }
    if n.as_parentheses_node().is_none() {
        return false;
    }
    let Some(n) = strip(n) else { return false };
    let Some(c) = n.as_call_node() else {
        return false;
    };
    let args = ast::call_arguments(&c);
    let [rhs] = args.as_slice() else { return false };
    if matches!(
        c.name().as_slice(),
        b"==" | b"===" | b"!=" | b"<=" | b">=" | b"<" | b">"
    ) {
        return true;
    }
    let Some(lhs) = c.receiver() else {
        return false;
    };
    (matches!(lhs, Node::FloatNode { .. } | Node::IntegerNode { .. })
        && matches!(
            c.name().as_slice(),
            b"+" | b"-" | b"*" | b"**" | b"/" | b"%" | b"<<"
        ))
        || (numeric(&lhs)
            && matches!(rhs, Node::FloatNode { .. } | Node::IntegerNode { .. })
            && matches!(
                c.name().as_slice(),
                b"+" | b"-" | b"*" | b"**" | b"/" | b"%"
            ))
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_freeze_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    #[test]
    fn exact_positions_literals_mutability_and_safe_navigation() {
        for literal in [
            "nil", "false", "true", "1", "1.5", "1r", "1i", ":sym", "/x/", "(1..3)", "__LINE__",
        ] {
            let source = format!("# header\n  {literal}.freeze\n");
            let found = crate::pipeline::lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == RedundantFreeze.name())
                .collect::<Vec<_>>();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                "Do not freeze immutable objects, as freezing them has no effect."
            );
            crate::cops::testing::assert_correction(
                &RedundantFreeze,
                &source,
                &format!("# header\n  {literal}\n"),
            );
        }
        for source in [
            "nil&.freeze",
            "false&.freeze",
            "1&.freeze",
            "[1].freeze",
            "{}.freeze",
            "'a'.freeze",
            "object.freeze",
            "Other.freeze",
            "freeze",
            "\"#{1}\".freeze",
            "# frozen_string_literal: true\n\"#{1}\".freeze",
            "(a + b).freeze",
            "('a' * 2).freeze",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&RedundantFreeze, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn all_pinned_diagnostics_and_messages() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_freeze_pinned.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let found = crate::cops::testing::correction_offenses(
                &RedundantFreeze,
                case["source"].as_str().unwrap(),
            );
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{case}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(messages), case["messages"], "{case}");
        }
    }
}
