//! `Style/RedundantConditional` — comparison conditionals with Boolean branches.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct RedundantConditional;
impl Cop for RedundantConditional {
    fn name(&self) -> &'static str {
        "Style/RedundantConditional"
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
        let (condition, yes, no, unless, elsif) = if let Some(n) = node.as_if_node() {
            if n.if_keyword_loc().is_some() && n.end_keyword_loc().is_none() {
                return;
            }
            (
                n.predicate(),
                boolean(n.statements()),
                n.subsequent()
                    .and_then(|n| n.as_else_node())
                    .and_then(|n| boolean(n.statements())),
                false,
                n.if_keyword_loc().is_some_and(|l| l.as_slice() == b"elsif"),
            )
        } else if let Some(n) = node.as_unless_node() {
            if n.end_keyword_loc().is_none() {
                return;
            }
            (
                n.predicate(),
                boolean(n.statements()),
                n.else_clause().and_then(|n| boolean(n.statements())),
                true,
                false,
            )
        } else {
            return;
        };
        let (Some(yes), Some(no)) = (yes, no) else {
            return;
        };
        let Some(call) = condition.as_call_node() else {
            return;
        };
        if yes == no
            || call.is_safe_navigation()
            || !matches!(
                call.name().as_slice(),
                b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<"
            )
            || crate::cops::ast::call_arguments(&call).len() != 1
            || call
                .block()
                .is_some_and(|block| block.as_block_argument_node().is_none())
        {
            return;
        }
        let source = String::from_utf8_lossy(condition.location().as_slice());
        let expression = if yes == unless {
            format!("!({source})")
        } else {
            source.into_owned()
        };
        let correction = cx
            .corrections_requested()
            .then(|| correct(node, &expression, elsif, file))
            .flatten();
        let (line, col) = file.line_col(node.location().start_offset());
        let replacement = if elsif {
            format!("else; {expression}")
        } else {
            expression
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "This conditional expression can just be replaced by `{}`.",
                crate::offense::single_line(replacement.as_bytes())
            ),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction,
        });
    }
}
fn boolean(statements: Option<ruby_prism::StatementsNode<'_>>) -> Option<bool> {
    let s = statements?;
    if s.body().len() != 1 {
        return None;
    }
    match s.body().first()? {
        Node::TrueNode { .. } => Some(true),
        Node::FalseNode { .. } => Some(false),
        _ => None,
    }
}

/// RuboCop replaces the conditional with the expression; an `elsif` (in
/// RuboCop's AST, up to its last branch) becomes an indented `else`.
fn correct(
    node: &Node<'_>,
    expression: &str,
    elsif: bool,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    if !elsif {
        return Some(Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: expression.into(),
        });
    }
    let end = node
        .as_if_node()?
        .subsequent()?
        .as_else_node()?
        .statements()?
        .location()
        .end_offset();
    let (_, col) = file.line_col(loc.start_offset());
    Some(Correction {
        range: loc.start_offset()..end,
        replacement: format!("else\n{}{expression}", " ".repeat(col as usize + 1)),
    })
}

#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop style/redundant_conditional_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantConditional.name())
            .collect()
    }
    #[test]
    fn pinned_ternaries_if_unless_and_elsif() {
        for (s, line, col, replacement) in [
            ("x == y ? true : false\n", 1, 1, "x == y"),
            ("x == y ? false : true\n", 1, 1, "!(x == y)"),
            ("if x == y\n  true\nelse\n  false\nend\n", 1, 1, "x == y"),
            ("if x == y\n  false\nelse\n  true\nend\n", 1, 1, "!(x == y)"),
            (
                "unless x == y\n  true\nelse\n  false\nend\n",
                1,
                1,
                "!(x == y)",
            ),
            (
                "unless x == y\n  false\nelse\n  true\nend\n",
                1,
                1,
                "x == y",
            ),
            (
                "if cond\n  false\nelsif x == y\n  true\nelse\n  false\nend\n",
                3,
                1,
                "else; x == y",
            ),
            (
                "if cond\n  false\nelsif x == y\n  false\nelse\n  true\nend\n",
                3,
                1,
                "else; !(x == y)",
            ),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(
                found[0].message,
                format!("This conditional expression can just be replaced by `{replacement}`.")
            );
        }
    }
    #[test]
    fn canonical_closed_comparison_corrections() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("1 == 2 ? true : false\n", "1 == 2\n"),
            ("1 == 2 ? false : true\n", "!(1 == 2)\n"),
            ("if 1 == 2\n  true\nelse\n  false\nend\n", "1 == 2\n"),
            ("if 1 == 2\n  false\nelse\n  true\nend\n", "!(1 == 2)\n"),
            ("unless 1 == 2\n  true\nelse\n  false\nend\n", "!(1 == 2)\n"),
            ("unless 1 == 2\n  false\nelse\n  true\nend\n", "1 == 2\n"),
            (
                "if false\n  false\nelsif 1 == 2\n  true\nelse\n  false\nend\n",
                "if false\n  false\nelse\n  1 == 2\nend\n",
            ),
            (
                "if false\n  false\nelsif 1 == 2\n  false\nelse\n  true\nend\n",
                "if false\n  false\nelse\n  !(1 == 2)\nend\n",
            ),
        ] {
            assert_correction(&RedundantConditional, before, after);
        }
    }
    #[test]
    fn all_seven_operators_and_both_boolean_orders() {
        for op in ["==", "===", "!=", "<=", ">=", ">", "<"] {
            for (branches, inverted) in [("true : false", false), ("false : true", true)] {
                let source = format!("  1 {op} 2 ? {branches}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1);
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 1));
                let expression = if inverted {
                    format!("!(1 {op} 2)")
                } else {
                    format!("1 {op} 2")
                };
                assert_eq!(
                    found[0].message,
                    format!("This conditional expression can just be replaced by `{expression}`.")
                );
                crate::cops::testing::assert_correction(
                    &RedundantConditional,
                    &source,
                    &format!("  {expression}\n"),
                );
            }
        }
    }
    #[test]
    fn nonboolean_branches_and_lookalikes() {
        for s in [
            "x == y ? 1 : 10",
            "if x == y; 1; else; 2; end",
            "if cond; 1; elsif x == y; 2; else; 3; end",
            "true if x == y",
            "false unless x == y",
            "if x == y; true; end",
            "x == y ? true : true",
            "x == y ? false : false",
            "x == y ? nil : false",
            "if x == y; work; true; else; false; end",
            "if x == y; true; else; work; false; end",
            "if x == y; return true; else; return false; end",
            "if x == y; (true); else; false; end",
            "(x == y) ? true : false",
            "x&.==(y) ? true : false",
            "x.==(y) { work } ? true : false",
            "x.== { work } ? true : false",
            "x.==(y, &blk) ? true : false",
            "x&.==(&blk) ? true : false",
            "x.==(y,z) ? true : false",
            "x.== ? true : false",
            "x <=> y ? true : false",
            "x =~ y ? true : false",
            "x.eql?(y) ? true : false",
            "x ? true : false",
            "if x == y; begin; true; ensure; work; end; else; false; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
