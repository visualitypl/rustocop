//! `Style/EmptyStringInsideInterpolation` — fixed trailing-conditional policy.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EmptyStringInsideInterpolation;
impl Cop for EmptyStringInsideInterpolation {
    fn name(&self) -> &'static str {
        "Style/EmptyStringInsideInterpolation"
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
        // Only immediate interpolation expressions, not arbitrary nested ifs.
        let mut parents = cx.ancestors();
        if !parents
            .next()
            .is_some_and(|p| p.as_statements_node().is_some())
            || !parents
                .next()
                .is_some_and(|p| p.as_embedded_statements_node().is_some())
        {
            return;
        }
        let (condition, yes, other, inverted) = if let Some(n) = node.as_if_node() {
            if n.end_keyword_loc().is_none() && n.then_keyword_loc().is_none() {
                return;
            }
            (n.predicate(), n.statements(), n.subsequent(), false)
        } else if let Some(n) = node.as_unless_node() {
            if n.end_keyword_loc().is_none() {
                return;
            }
            (
                n.predicate(),
                n.statements(),
                n.else_clause().map(|e| e.as_node()),
                true,
            )
        } else {
            return;
        };
        let yes = yes.map(|s| branch(&s));
        let no = other.and_then(|n| {
            if let Some(e) = n.as_else_node() {
                e.statements().map(|s| branch(&s))
            } else {
                Some(n)
            }
        });
        let (Some(yes), Some(no)) = (yes, no) else {
            return;
        };
        for (empty, outcome, keyword) in [
            (&yes, &no, if inverted { "if" } else { "unless" }),
            (&no, &yes, if inverted { "unless" } else { "if" }),
        ] {
            if !is_empty(empty) {
                continue;
            }
            let (line, col) = file.line_col(node.location().start_offset());
            // RuboCop keeps the outcome and makes the rest a trailing
            // conditional, unless the condition holds such a conditional too.
            let correction = (cx.corrections_requested() && !nested_empty_conditional(&condition))
                .then(|| {
                    let o = outcome.location();
                    let n = node.location();
                    let predicate = std::str::from_utf8(condition.location().as_slice()).ok()?;
                    super::correction::combine(
                        file,
                        vec![
                            Correction {
                                range: n.start_offset()..o.start_offset(),
                                replacement: String::new(),
                            },
                            Correction {
                                range: o.end_offset()..n.end_offset(),
                                replacement: format!(" {keyword} {predicate}"),
                            },
                        ],
                    )
                })
                .flatten();
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Do not return empty strings in string interpolation.".into(),
                line,
                col,
                end_line: file.last_line_of(node.location()),
                correction,
            });
            // RuboCop deduplicates offenses at the same conditional when both
            // branches are empty; retain its first (empty-if) orientation.
            break;
        }
    }
}
fn branch<'pr>(s: &ruby_prism::StatementsNode<'pr>) -> Node<'pr> {
    if s.body().len() == 1 {
        s.body().iter().next().unwrap()
    } else {
        s.as_node()
    }
}
fn is_empty(n: &Node<'_>) -> bool {
    n.as_nil_node().is_some() || n.as_string_node().is_some_and(|s| s.unescaped().is_empty())
}
/// RuboCop's `empty_string_conditional_in_condition?`: an interpolated
/// conditional with an empty branch in the condition.
fn nested_empty_conditional(condition: &Node<'_>) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Finder(bool);
    impl<'pr> NodeVisitor<'pr> for Finder {
        fn enter(&mut self, node: Node<'pr>) {
            let Some(statements) = node
                .as_embedded_statements_node()
                .and_then(|e| e.statements())
            else {
                return;
            };
            let Some(inner) = statements.body().iter().next() else {
                return;
            };
            let branches = if let Some(n) = inner.as_if_node() {
                if n.if_keyword_loc().is_some() && n.end_keyword_loc().is_none() {
                    return;
                }
                let no = n.subsequent().and_then(|s| {
                    s.as_else_node()
                        .and_then(|e| e.statements())
                        .map(|s| branch(&s))
                });
                (n.statements().map(|s| branch(&s)), no)
            } else if let Some(n) = inner.as_unless_node() {
                if n.end_keyword_loc().is_none() {
                    return;
                }
                let no = n
                    .else_clause()
                    .and_then(|e| e.statements())
                    .map(|s| branch(&s));
                (no, n.statements().map(|s| branch(&s)))
            } else {
                return;
            };
            self.0 |= branches.0.is_some_and(|b| is_empty(&b))
                || branches.1.is_some_and(|b| is_empty(&b));
        }
    }
    let mut finder = Finder(false);
    walk(&mut finder, condition);
    finder.0
}
#[cfg(test)]
mod tests {
    //! RuboCop empty_string_inside_interpolation_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == EmptyStringInsideInterpolation.name())
            .collect()
    }
    #[test]
    fn pinned_conditional_branches_positions_and_message() {
        for empty in ["''", "\"\"", "nil"] {
            for expr in [
                format!("condition ? 'foo' : {empty}"),
                format!("condition ? 42 : {empty}"),
                format!("if condition; 'foo' else {empty} end"),
                format!("condition ? {empty} : 'foo'"),
                format!("if condition; {empty} else 'foo' end"),
            ] {
                let src = format!("\"#{{{expr}}}\"");
                let o = offenses(&src);
                assert_eq!(o.len(), 1, "{src}");
                assert_eq!((o[0].line, o[0].col), (1, 4));
                assert_eq!(
                    o[0].message,
                    "Do not return empty strings in string interpolation."
                );
            }
        }
    }
    #[test]
    fn pinned_lookalikes() {
        for s in [
            "condition ? send_node : 'foo'",
            "condition ? 'foo' : send_node",
            "condition ? send_node : another_send_node",
            "'foo' if condition",
            "'' if condition",
            "'' unless condition",
        ] {
            assert!(offenses(&format!("\"#{{{s}}}\"")).is_empty());
        }
        assert!(offenses("condition ? 'foo' : ''").is_empty());
    }
    #[test]
    fn nil_branch_canonical_pinned_corrections() {
        for (expr, after) in [
            ("condition ? 'foo' : nil", "'foo' if condition"),
            ("condition ? 42 : nil", "42 if condition"),
            ("if condition; 'foo' else nil end", "'foo' if condition"),
            ("condition ? nil : 'foo'", "'foo' unless condition"),
            ("if condition; nil else 'foo' end", "'foo' unless condition"),
        ] {
            assert_correction(
                &EmptyStringInsideInterpolation,
                &format!("\"#{{{expr}}}\""),
                &format!("\"#{{{after}}}\""),
            );
        }
    }
    #[test]
    fn interpolation_kinds_siblings_and_unless_keep_condition_semantics() {
        for (open, close) in [
            ("\"", "\""),
            (":\"", "\""),
            ("/", "/o"),
            ("`", "`"),
            ("%W[", "]"),
            ("%I[", "]"),
        ] {
            assert_correction(
                &EmptyStringInsideInterpolation,
                &format!("{open}#{{true ? 42 : nil}}{close}"),
                &format!("{open}#{{42 if true}}{close}"),
            );
        }
        assert_correction(
            &EmptyStringInsideInterpolation,
            r##""#{unless condition; nil else 'x' end}""##,
            r##""#{'x' if condition}""##,
        );
        assert_correction(
            &EmptyStringInsideInterpolation,
            r##""#{unless condition; 'x' else nil end}""##,
            r##""#{'x' unless condition}""##,
        );
        assert_eq!(
            offenses(r##""#{true ? '' : nil; false ? nil : ''}""##).len(),
            2
        );
        assert!(offenses(r##""#{consume(true ? '' : 'x')}""##).is_empty());
    }
}
