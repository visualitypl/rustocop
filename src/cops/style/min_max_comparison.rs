//! `Style/MinMaxComparison` — unsafe comparison hints; detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MinMaxComparison;
impl Cop for MinMaxComparison {
    fn name(&self) -> &'static str {
        "Style/MinMaxComparison"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (mut condition, yes, no) = if let Some(n) = node.as_if_node() {
            (
                n.predicate(),
                single(n.statements()),
                n.subsequent()
                    .and_then(|e| e.as_else_node())
                    .and_then(|e| single(e.statements())),
            )
        } else if let Some(n) = node.as_unless_node() {
            (
                n.predicate(),
                n.else_clause().and_then(|e| single(e.statements())),
                single(n.statements()),
            )
        } else {
            return;
        };
        let (Some(yes), Some(no)) = (yes, no) else {
            return;
        };
        if let Some(p) = condition.as_parentheses_node() {
            let Some(body) = p
                .body()
                .and_then(|n| n.as_statements_node())
                .and_then(|n| single(Some(n)))
            else {
                return;
            };
            condition = body;
        }
        let Some(compare) = condition.as_call_node() else {
            return;
        };
        if compare.is_safe_navigation() || compare.block().is_some() {
            return;
        }
        let greater = match compare.name().as_slice() {
            b">" | b">=" => true,
            b"<" | b"<=" => false,
            _ => return,
        };
        let Some(lhs) = compare.receiver() else {
            return;
        };
        let args = ast::call_arguments(&compare);
        let [rhs] = args.as_slice() else { return };
        let lhs_fingerprint = ast::fingerprint_with_source_lines(&lhs, file);
        let rhs_fingerprint = ast::fingerprint_with_source_lines(rhs, file);
        let yes = ast::fingerprint_with_source_lines(&yes, file);
        let no = ast::fingerprint_with_source_lines(&no, file);
        let normal = if lhs_fingerprint == yes && rhs_fingerprint == no {
            true
        } else if lhs_fingerprint == no && rhs_fingerprint == yes {
            false
        } else {
            return;
        };
        let method = if greater == normal { "max" } else { "min" };
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: format!(
                "Use `[{}, {}].{method}` instead.",
                single_line(lhs.location().as_slice()),
                single_line(rhs.location().as_slice())
            ),
            correction: None,
        });
    }
}
fn single(statements: Option<ruby_prism::StatementsNode<'_>>) -> Option<Node<'_>> {
    let s = statements?;
    (s.body().len() == 1)
        .then(|| s.body().iter().next())
        .flatten()
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/min_max_comparison_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == MinMaxComparison.name())
            .collect()
    }
    #[test]
    fn all_operator_and_branch_orders() {
        for (op, normal) in [(">", "max"), (">=", "max"), ("<", "min"), ("<=", "min")] {
            for (branches, method) in [
                ("a : b", normal),
                ("b : a", if normal == "max" { "min" } else { "max" }),
            ] {
                for condition in [format!("a {op} b"), format!("(a {op} b)")] {
                    let s = format!("{condition} ? {branches}");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!((o[0].line, o[0].col), (1, 1));
                    assert_eq!(o[0].message, format!("Use `[a, b].{method}` instead."));
                    assert!(
                        crate::cops::testing::correction_offenses(&MinMaxComparison, &s)[0]
                            .correction
                            .is_none()
                    );
                }
            }
        }
    }
    #[test]
    fn if_unless_and_elsif() {
        for (s, line, col, method) in [
            ("if a > b\n a\nelse\n b\nend", 1, 1, "max"),
            ("if a < b\n a\nelse\n b\nend", 1, 1, "min"),
            ("unless a > b\n a\nelse\n b\nend", 1, 1, "min"),
            ("unless a < b\n a\nelse\n b\nend", 1, 1, "max"),
            ("if x\nelsif a < b\n a\nelse\n b\nend", 2, 1, "min"),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (line, col));
            assert_eq!(o[0].message, format!("Use `[a, b].{method}` instead."));
        }
    }
    #[test]
    fn source_line_equality_uses_evaluated_lines() {
        for (s, message) in [
            (
                "__LINE__ > 0 ? __LINE__ : 0",
                "Use `[__LINE__, 0].max` instead.",
            ),
            (
                "__LINE__ > 0 ? 0 : __LINE__",
                "Use `[__LINE__, 0].min` instead.",
            ),
            (
                "0 < __LINE__ ? __LINE__ : 0",
                "Use `[0, __LINE__].max` instead.",
            ),
            (
                "[__LINE__] > xs ? [__LINE__] : xs",
                "Use `[[__LINE__], xs].max` instead.",
            ),
        ] {
            let s = format!("\n{s}");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, 1));
            assert_eq!(o[0].message, message);
            assert!(
                crate::cops::testing::correction_offenses(&MinMaxComparison, &s)[0]
                    .correction
                    .is_none()
            );
        }
        for s in [
            "__LINE__ > 0 ?\n __LINE__ : 0",
            "__LINE__ > 0 ? 0 :\n __LINE__",
            "0 < __LINE__ ?\n __LINE__ : 0",
            "[__LINE__] > xs ?\n [__LINE__] : xs",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn branch_condition_and_arity_lookalikes() {
        for s in [
            "a > b ? c : d",
            "condition ? a : b",
            "if a\nelsif foo(b)\n b\nend",
            "[a,b].max",
            "[a,b].min",
            "a > b ? a : c",
            "a > b ? (a; a) : b",
            "a > b ? a : nil",
            "a&.>(b) ? a : b",
            "if a > b; a; end",
            "a > b ? (a) : b",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
