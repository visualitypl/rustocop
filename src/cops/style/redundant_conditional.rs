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
            .then(|| correct(node, &condition, &expression, elsif, cx, file))
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

// Only closed small-Integer comparisons establish both an actual Boolean result
// and built-in ! dispatch. Arbitrary Ruby ==/!=/=== methods can return anything.
fn closed(call: &ruby_prism::CallNode<'_>) -> bool {
    let args = crate::cops::ast::call_arguments(call);
    let [rhs] = args.as_slice() else {
        return false;
    };
    call.receiver().is_some_and(|n| integer(&n))
        && integer(rhs)
        && !call.is_safe_navigation()
        && call.block().is_none()
        && matches!(
            call.name().as_slice(),
            b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<"
        )
}
fn integer(n: &Node<'_>) -> bool {
    n.as_integer_node()
        .and_then(|n| n.value().try_into().ok())
        .is_some_and(|i: i32| (-(1 << 30)..(1 << 30)).contains(&i))
}
fn correct(
    node: &Node<'_>,
    condition: &Node<'_>,
    expression: &str,
    elsif: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    if range.len() > 16 * 1024
        || !closed(&condition.as_call_node()?)
        || super::correction::contains_comment(file, range.clone())
        || super::correction::touches_directive(file, range.clone())
        || std::str::from_utf8(loc.as_slice()).is_err()
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let facts = file.redundant_conditional_safety(root);
    if facts.changed
        || facts.first_effect.is_some_and(|at| at <= range.start)
        || file.heredoc_pending_at(root, range.start)
    {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    let replacement = if elsif {
        // Prism includes the owning end in an elsif location. Keep it, with
        // the original newline convention, instead of deleting the outer end.
        let (_, col) = file.line_col(range.start);
        if col > 16 * 1024 {
            return None;
        }
        let nl = if loc.as_slice().windows(2).any(|s| s == b"\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        format!(
            "else{nl}{}{expression}{nl}{}end",
            " ".repeat(col as usize + 1),
            " ".repeat(col as usize - 1)
        )
    } else if cx
        .parent()
        .is_some_and(|p| p.as_statements_node().is_some())
    {
        expression.into()
    } else {
        // A comparison has lower precedence than many consumers of an if
        // expression, e.g. `true == if ... end`. Retain its grouping there.
        format!("({expression})")
    };
    super::literal_spelling::safe_edit(file, cx, Correction { range, replacement })
}

/// One correction-only main-parse proof. Deferred/repeated execution and implicit
/// Ruby dispatch are barriers, not inferred value flow or per-offense reparses.
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
    fn enter(&mut self, n: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        if let Some(call) = n.as_call_node() {
            if !closed(&call) {
                let end = n.location().end_offset();
                self.first_effect = Some(self.first_effect.map_or(end, |old| old.min(end)));
                self.changed |= call.block().is_some();
            }
        } else {
            self.changed |= !matches!(
                n,
                Node::ProgramNode { .. }
                    | Node::StatementsNode { .. }
                    | Node::ArgumentsNode { .. }
                    | Node::IntegerNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
                    | Node::NilNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::IfNode { .. }
                    | Node::UnlessNode { .. }
                    | Node::ElseNode { .. }
                    | Node::LocalVariableReadNode { .. }
                    | Node::LocalVariableWriteNode { .. }
            );
        }
    }
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
    fn unknown_dispatch_comments_and_source_sensitive_edits_are_withheld() {
        use crate::cops::testing::assert_correction;
        for source in [
            "x == y ? true : false\n",
            "x == y ? false : true\n",
            "effect; 1 == 2 ? true : false\n",
            "class Integer; def ==(x); :truthy; end; end; 1 == 2 ? true : false\n",
            "def f; 1 == 2 ? true : false; end\n",
            "while true; 1 == 2 ? true : false; end\n",
            "begin; 1 == 2 ? true : false; rescue; recover; ensure; clean; end\n",
            "if 1 == 2\n  true # intent\nelse\n  false\nend\n",
            "if 1 == 2\n  true\nelse\n  false\nend\np __LINE__\n",
            "p DATA.pos\n1 == 2 ? true : false\n__END__\npayload\n",
        ] {
            assert_correction(&RedundantConditional, source, source);
        }
    }
    #[test]
    fn all_pinned_calls_and_corrections_are_accounted_for() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_conditional_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 11);
        let mut corrections = 0;
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found = offenses(source);
            let positions: Vec<(u32, u32)> =
                serde_json::from_value(c["positions"].clone()).unwrap();
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
            if c["correction"].is_string() {
                corrections += 1;
                // Each pinned edit uses unknown x/y comparison dispatch (and
                // the elsif cases also use an effectful cond). Keep detection.
                crate::cops::testing::assert_correction(&RedundantConditional, source, source);
                let before = source
                    .replace("x == y", "1 == 2")
                    .replace("if cond", "if false");
                let after = c["correction"]
                    .as_str()
                    .unwrap()
                    .replace("x == y", "1 == 2")
                    .replace("if cond", "if false");
                crate::cops::testing::assert_correction(&RedundantConditional, &before, &after);
            }
        }
        assert_eq!(corrections, 8);
    }
    #[test]
    fn correction_source_work_is_lazy_and_capped_without_hiding_offenses() {
        REWRITE_BYTES.with(|n| n.set(0));
        let source = "if 1 < 2\n true\nelse\n false\nend\n".repeat(1000);
        assert_eq!(offenses(&source).len(), 1000);
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        let found = crate::cops::testing::correction_offenses(&RedundantConditional, &source);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert!(REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let large = format!("if 1 < 2\n{}true\nelse\n false\nend\n", " ".repeat(20_000));
        assert_eq!(offenses(&large).len(), 1);
        REWRITE_BYTES.with(|n| n.set(0));
        crate::cops::testing::assert_correction(&RedundantConditional, &large, &large);
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        // The elsif location excludes leading whitespace. Bound rendered
        // indentation too, not just bytes inside the AST candidate's range.
        let indented = format!(
            "if false\n true\n{}elsif 1 < 2\n true\nelse\n false\nend\n",
            " ".repeat(20_000)
        );
        crate::cops::testing::assert_correction(&RedundantConditional, &indented, &indented);
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
    fn block_pass_only_comparisons_are_reported_without_corrections() {
        for op in ["==", "===", "!=", "<=", ">=", ">", "<"] {
            for (branches, inverted) in [("true : false", false), ("false : true", true)] {
                let condition = format!("x.{op}(&blk)");
                let source = format!("  {condition} ? {branches}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 1));
                let expression = if inverted {
                    format!("!({condition})")
                } else {
                    condition
                };
                assert_eq!(
                    found[0].message,
                    format!("This conditional expression can just be replaced by `{expression}`.")
                );
                crate::cops::testing::assert_correction(&RedundantConditional, &source, &source);
            }
        }
    }
    #[test]
    fn nested_consumers_keep_precedence_and_else_association() {
        for (before, after) in [
            (
                "true == if 1 == 2 then true else false end\n",
                "true == (1 == 2)\n",
            ),
            ("result = 1 == 2 ? true : false\n", "result = (1 == 2)\n"),
            (
                "if true\n  if 1 == 2\n    true\n  else\n    false\n  end\nelse\n  false\nend\n",
                "if true\n  1 == 2\nelse\n  false\nend\n",
            ),
            (
                "if false\n  true\nelsif false\n  true\nelsif 1 < 2\n  false\nelse\n  true\nend\n",
                "if false\n  true\nelsif false\n  true\nelse\n  !(1 < 2)\nend\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&RedundantConditional, before, after);
        }
    }
    #[test]
    fn source_envelopes_and_line_values() {
        for (before, after) in [
            (
                "\u{feff}# encoding: UTF-8\r\nif 1 == 2\r\n  true\r\nelse\r\n  false\r\nend\r\n__END__\r\nraw\r\n",
                "\u{feff}# encoding: UTF-8\r\n1 == 2\r\n__END__\r\nraw\r\n",
            ),
            (
                "if false\r\n  false\r\nelsif 1 == 2\r\n  true\r\nelse\r\n  false\r\nend\r\n",
                "if false\r\n  false\r\nelse\r\n  1 == 2\r\nend\r\n",
            ),
            (
                "1 == 2 ? true : false # explanation\n",
                "1 == 2 # explanation\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&RedundantConditional, before, after);
        }
        for source in [
            "if 1 == 2\n true\nelse\n false\nend\np __LINE__\n",
            "if 1 == 2\n true\nelse\n false\nend # rubocop:disable Lint/Debugger\n",
            "p DATA.pos\n1 == 2 ? true : false\n__END__\nraw\n",
            "p File.read(__FILE__)\n1 == 2 ? true : false\n",
            "if 1 == 2\n # explanation\n true\nelse\n false\nend\n",
        ] {
            crate::cops::testing::assert_correction(&RedundantConditional, source, source);
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
