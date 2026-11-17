//! `Style/TrailingUnderscoreVariable` — fixed named-underscore allowance.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::ops::Range;
pub struct TrailingUnderscoreVariable;
impl Cop for TrailingUnderscoreVariable {
    fn name(&self) -> &'static str {
        "Style/TrailingUnderscoreVariable"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_multi_write_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_multi_write_node().is_none() {
            return;
        }
        let mut findings = Vec::new();
        collect(node, &mut findings);
        for finding in findings {
            let preferred = finding.range.clone();
            let mut good = file.bytes[node.location().start_offset()..preferred.start].to_vec();
            good.extend_from_slice(finding.replacement.as_bytes());
            good.extend_from_slice(&file.bytes[preferred.end..node.location().end_offset()]);
            if finding.replacement == "[" {
                good.push(b']');
            }
            let (line, col) = file.line_col(finding.range.start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Do not use trailing `_`s in parallel assignment. Prefer `{}`.",
                    String::from_utf8_lossy(&good)
                ),
                line,
                col,
                end_line: file.line_of(finding.range.end.saturating_sub(1)),
                correction: cx
                    .corrections_requested()
                    .then(|| correction(node, &finding, file))
                    .flatten(),
            });
        }
    }
}

struct Finding {
    range: Range<usize>,
    replacement: String,
}

fn targets<'pr>(node: &Node<'pr>) -> (Vec<Node<'pr>>, Option<usize>) {
    let (left, rest, right, close) = if let Some(n) = node.as_multi_write_node() {
        (n.lefts(), n.rest(), n.rights(), n.rparen_loc())
    } else {
        let n = node.as_multi_target_node().unwrap();
        (n.lefts(), n.rest(), n.rights(), n.rparen_loc())
    };
    (
        left.iter()
            .chain(rest.filter(|n| n.as_implicit_rest_node().is_none()))
            .chain(right.iter())
            .collect(),
        close.map(|l| l.start_offset()),
    )
}
fn underscore(node: &Node<'_>) -> bool {
    node.as_local_variable_target_node()
        .is_some_and(|n| n.name().as_slice() == b"_")
        || node
            .as_splat_node()
            .and_then(|n| n.expression())
            .is_some_and(|n| underscore(&n))
}
fn collect(node: &Node<'_>, out: &mut Vec<Finding>) -> bool {
    let (vars, close) = targets(node);
    let mut locals_only = true;
    for child in &vars {
        locals_only &= if child.as_multi_target_node().is_some() {
            collect(child, out)
        } else {
            local_target(child)
        };
    }
    let first = vars
        .iter()
        .rposition(|n| !underscore(n))
        .map_or(0, |i| i + 1);
    if first == vars.len() {
        return locals_only;
    }
    // The pinned cop locates the first candidate by reverse structural equality,
    // not source identity. Repeated `_` targets can therefore put a splat before
    // the matching target even when it occurs inside the removable suffix.
    // Only bare `_` / `*_` candidates participate, so comparisons are bounded.
    let last_equal = vars
        .iter()
        .rposition(|n| underscore(n) && crate::cops::ast::same_node(n, &vars[first]))
        .unwrap();
    if vars[..last_equal]
        .iter()
        .any(|n| n.as_splat_node().is_some())
    {
        return locals_only;
    }
    let (range, replacement) = if first == 0 {
        if let Some(write) = node.as_multi_write_node() {
            let value = write.value();
            (
                node.location().start_offset()..value.location().start_offset(),
                if value
                    .as_array_node()
                    .is_some_and(|a| a.opening_loc().is_none())
                {
                    "["
                } else {
                    ""
                },
            )
        } else {
            (
                node.location().start_offset()..node.location().end_offset(),
                "_",
            )
        }
    } else if let Some(close) = close {
        (vars[first].location().start_offset() - 1..close, "")
    } else {
        (
            vars[first].location().start_offset()
                ..node
                    .as_multi_write_node()
                    .unwrap()
                    .operator_loc()
                    .start_offset(),
            "",
        )
    };
    out.push(Finding {
        range,
        replacement: replacement.into(),
    });
    locals_only
}
fn local_target(node: &Node<'_>) -> bool {
    node.as_local_variable_target_node().is_some()
        || node
            .as_splat_node()
            .is_some_and(|s| s.expression().is_none_or(|n| local_target(&n)))
}

/// RuboCop's corrector.
fn correction(node: &Node<'_>, finding: &Finding, file: &SourceFile) -> Option<Correction> {
    let mut edits = vec![Correction {
        range: finding.range.clone(),
        replacement: finding.replacement.clone(),
    }];
    if finding.replacement == "[" {
        let end = node.location().end_offset();
        edits.push(Correction {
            range: end..end,
            replacement: "]".into(),
        });
    }
    super::correction::combine(file, edits)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/trailing_underscore_variable_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TrailingUnderscoreVariable")
            .collect()
    }
    #[test]
    fn nested_offense_messages_apply_one_change_to_the_assignment() {
        let found = offenses("a, (_, (b, _), *_) = foo()");
        assert_eq!(found.len(), 2);
        for (o, col, good) in [
            (&found[0], 11, "a, (_, (b,), *_) = foo()"),
            (&found[1], 15, "a, (_, (b, _),) = foo()"),
        ] {
            assert_eq!((o.line, o.col, o.end_line), (1, col, 1));
            assert_eq!(
                o.message,
                format!("Do not use trailing `_`s in parallel assignment. Prefer `{good}`.")
            );
        }
    }
    #[test]
    fn canonical_all_underscore_array_values() {
        for (before, after) in [
            ("_, _ = 1, 2, 3", "[1, 2, 3]"),
            ("_, _ = *foo", "[*foo]"),
            ("_, _ = [1, 2]", "[1, 2]"),
            ("(_, _, _,) = [1, 2]", "[1, 2]"),
            ("(_,) = [1, 2]", "[1, 2]"),
            ("f((_, _ = [1, 2]))", "f(([1, 2]))"),
        ] {
            crate::cops::testing::assert_correction(
                &super::TrailingUnderscoreVariable,
                before,
                after,
            );
        }
    }
    #[test]
    fn repeated_underscore_targets_before_and_after_splats_match_pin() {
        for source in [
            "_, *_, _ = [1, 2, 3]",
            "(_, *_, _) = [1, 2, 3]",
            "a, _, *_, _ = [1, 2, 3, 4]",
            "a, (_, *_, _) = [1, [2, 3, 4]]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // A splat after the last equal bare target does not suppress the offense.
        assert_eq!(offenses("_, *_ = [1, 2]").len(), 1);
        assert_eq!(offenses("a, _, *_ = [1, 2, 3]").len(), 1);
    }
    #[test]
    fn pinned_no_offenses_and_syntax_lookalikes() {
        for source in [
            "a, b, c = foo()",
            "_, a, b = foo()",
            "*a, b, _ = foo()",
            "a, *b, _ = foo()",
            "_, *b, _ = *foo",
            "a, *b, _, _ = foo()",
            "a, *b, _c, _d = foo()",
            "a, *b, _c = foo()",
            "a, (_, b) = foo()",
            "a, (_, (b,), c, (d, e),) = foo()",
            "a, b, _c = foo()",
            "a, *_b = foo()",
            "a, *_b, _ = foo()",
            "a, *_b, _, _ = foo()",
            "_ = foo()",
            "f { |a, _| a }",
            "def f(a, _); end",
            "f(a, _)",
            "[a, _]; {a: _}",
            "for a, _ in xs; end",
            "x => [a, _]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
