//! `Style/YodaExpression` — numeric/constant-first binary expressions (opt-in).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct YodaExpression;
impl Cop for YodaExpression {
    fn name(&self) -> &'static str {
        "Style/YodaExpression"
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
        let Some(rhs) = candidate(node) else { return };
        let mut child = node;
        for ancestor in cx.ancestors() {
            // A literal block's call is its sibling in RuboCop's AST, not
            // an ancestor of its body. Outer enclosing sends still count.
            if child.as_block_node().is_none() && candidate(ancestor).is_some() {
                return;
            }
            child = ancestor;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        // Swap the operands.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let lhs = node.as_call_node()?.receiver()?;
                let text =
                    |n: &Node<'_>| String::from_utf8_lossy(n.location().as_slice()).into_owned();
                let span = |n: &Node<'_>| n.location().start_offset()..n.location().end_offset();
                crate::offense::Correction::from_edits(
                    &file.bytes,
                    vec![(span(&lhs), text(&rhs)), (span(&rhs), text(&lhs))],
                )
            })
            .flatten();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!(
                "Non-literal operand (`{}`) should be first.",
                String::from_utf8_lossy(rhs.location().as_slice())
            ),
            correction,
        });
    }
}
fn constant_portion(node: &Node<'_>) -> bool {
    ast::is_const(node)
        || ast::negated_numeric_literal(node).is_some()
        || matches!(
            node,
            Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::RationalNode { .. }
                | Node::ImaginaryNode { .. }
                | Node::SourceLineNode { .. }
        )
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation()
        || !matches!(call.name().as_slice(), b"*" | b"+" | b"&" | b"|" | b"^")
    {
        return None;
    }
    let lhs = call.receiver()?;
    // Ancestors are rechecked for every nested candidate: read only the
    // first argument, never collect a potentially wide argument list. A lone
    // &block cannot be an operand, so no block-argument fallback is needed.
    let rhs = call.arguments()?.arguments().first()?;
    if !constant_portion(&lhs)
        || constant_portion(&rhs)
        || matches!(
            rhs,
            Node::SplatNode { .. }
                | Node::BlockArgumentNode { .. }
                | Node::ForwardingArgumentsNode { .. }
                | Node::KeywordHashNode { .. }
        )
    {
        return None;
    }
    Some(rhs)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/yoda_expression_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Fixed operators include &, |, ^.
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        let registry = crate::cops::Registry::new();
        let config = crate::config::ResolvedConfig::default();
        let active =
            crate::cops::select_active(&registry, &config, &["Style/YodaExpression".into()], &[]);
        let r = crate::pipeline::lint_one(
            std::path::Path::new("a.rb"),
            std::path::Path::new("a.rb"),
            Some(s.as_bytes()),
            &active,
            &config,
        );
        assert!(r.errors.is_empty() && r.internal_errors.is_empty(), "{r:?}");
        r.result.offenses
    }
    #[test]
    fn flags_all_supported_operators_and_numeric_kinds() {
        for lhs in [
            "1",
            "-1",
            "1.5",
            "1r",
            "2i",
            "CONST",
            "::Foo::BAR",
            "__LINE__",
            "__ENCODING__",
        ] {
            for op in ["*", "+", "&", "|", "^"] {
                let s = format!("# expression\n  {lhs} {op} x\n");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col), (2, 3));
                assert_eq!(o[0].message, "Non-literal operand (`x`) should be first.");
                assert!(o[0].correction.is_none());
            }
        }
    }
    #[test]
    fn reports_only_outermost_candidate_even_through_arguments() {
        for (s, rhs) in [
            ("2 + (1 + x)", "(1 + x)"),
            ("TWO + (ONE + x)", "(ONE + x)"),
            ("1 + CONST.+(ary)", "CONST.+(ary)"),
            ("1 + CONST.*", "CONST.*"),
            ("CONST.+({ key: value })", "{ key: value }"),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                format!("Non-literal operand (`{rhs}`) should be first.")
            );
        }
    }
    #[test]
    fn wide_argument_lists_have_bounded_repeated_ancestor_work() {
        for width in [256, 512, 1024] {
            // Every nested candidate rechecks the same wide enclosing send.
            // Count collected arguments instead of relying on wall-clock time.
            let source = format!("CONST.+(x{})", ", 1 + x".repeat(width));
            ast::CALL_ARGUMENT_VISITS.set(0);
            let o = offenses(&source);
            let visits = ast::CALL_ARGUMENT_VISITS.get();
            assert_eq!(o.len(), 1);
            assert_eq!((o[0].line, o[0].col), (1, 1));
            assert_eq!(o[0].message, "Non-literal operand (`x`) should be first.");
            assert!(o[0].correction.is_none());
            assert!(visits <= 8 * width, "{visits} visits for {width} arguments");
        }
    }

    #[test]
    fn block_sends_are_siblings_but_outer_operand_sends_are_ancestors() {
        let o = offenses("1.+(x) { 2 + y }");
        assert_eq!(o.iter().map(|o| o.col).collect::<Vec<_>>(), [1, 10]);
        let o = offenses("1 + f { 2 + y }");
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].col, 1);
        assert_eq!(offenses("f { 2 + y }")[0].col, 5);
        // An ordinary outer send which is not a candidate suppresses nothing.
        assert_eq!(offenses("consume(1 + x)")[0].col, 9);
    }

    #[test]
    fn accepts_non_numeric_literals_constants_on_both_sides_and_argument_only_nodes() {
        for s in [
            "x + 42",
            "x + FORTY_TWO",
            "x + y",
            "60 * 24",
            "CONST + OTHER",
            "\"str\" + x",
            "[1] + x",
            "1 - x",
            "1 / x",
            "1 ** x",
            "1&.+(x)",
            "CONST.+(*ary)",
            "CONST.+(&block)",
            "CONST.+(key: value)",
            "CONST.+(**options)",
            "CONST.*",
            "def f(...); CONST.+(...); end",
            "def f(*); CONST.+(*); end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
