//! `Style/CaseEquality` — explicit case equality, preserving regexp match state.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct CaseEquality;
/// RuboCop's `replacement`.
fn correct(node: &Node<'_>) -> Option<Correction> {
    let call = node.as_call_node()?;
    let lhs = call.receiver()?;
    let args = ast::call_arguments(&call);
    let rhs = args.first()?;
    let text = |n: &Node<'_>| {
        std::str::from_utf8(n.location().as_slice())
            .ok()
            .map(str::to_owned)
    };
    let parenthesized = |n: &Node<'_>| -> Option<String> {
        let wrap = matches!(
            n,
            Node::AndNode { .. }
                | Node::OrNode { .. }
                | Node::IfNode { .. }
                | Node::RangeNode { .. }
        ) || super::redundant_parentheses::is_assignment(n)
            || n.as_call_node().is_some_and(|c| {
                !c.is_safe_navigation()
                    && !c.block().is_some_and(|b| b.as_block_node().is_some())
                    && ast::is_operator_method(c.name().as_slice())
            });
        let source = text(n)?;
        Some(if wrap { format!("({source})") } else { source })
    };
    let replacement = match &lhs {
        Node::ParenthesesNode { .. } => {
            let first = lhs
                .as_parentheses_node()?
                .body()?
                .as_statements_node()?
                .body()
                .first()?;
            first.as_range_node()?;
            format!("{}.include?({})", text(&lhs)?, text(rhs)?)
        }
        Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. } => {
            format!("{}.is_a?({})", parenthesized(rhs)?, text(&lhs)?)
        }
        Node::CallNode { .. } => {
            let c = lhs.as_call_node()?;
            if c.name().as_slice() != b"class"
                || !c.receiver().is_some_and(|r| r.as_self_node().is_some())
                || c.arguments().is_some()
            {
                return None;
            }
            format!("{}.is_a?({})", parenthesized(rhs)?, text(&lhs)?)
        }
        _ => return None,
    };
    let loc = node.location();
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement,
    })
}
impl Cop for CaseEquality {
    fn name(&self) -> &'static str {
        "Style/CaseEquality"
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
            correction: cx.corrections_requested().then(|| correct(node)).flatten(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported/adapted RuboCop style/case_equality_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
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
