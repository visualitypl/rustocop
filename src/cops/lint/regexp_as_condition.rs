//! `Lint/RegexpAsCondition` — implicit matching against `$_`.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RegexpAsCondition;
impl Cop for RegexpAsCondition {
    fn name(&self) -> &'static str {
        "Lint/RegexpAsCondition"
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
        if !regexp(node) {
            return;
        }
        let ancestors: Vec<_> = cx.ancestors().collect();
        if !ancestors.iter().any(|n| conditional(n)) || !converted(node, &ancestors) {
            return;
        }
        // The outer match callback runs first and ignores only its source range.
        // Heredoc interpolation descendants can lie outside that range.
        if ancestors.iter().enumerate().any(|(i, n)| {
            regexp(n)
                && n.location().start_offset() <= node.location().start_offset()
                && node.location().end_offset() <= n.location().end_offset()
                && ancestors[i + 1..].iter().any(|n| conditional(n))
                && converted(n, &ancestors[i + 1..])
        }) {
            return;
        }
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: cx.corrections_requested()
                .then(|| correction(node, cx)).flatten(),
            message: "Do not use regexp literal as a condition. The regexp literal matches `$_` implicitly.".into(),
            line,
            col,
            end_line: file.line_of(node.location().end_offset().saturating_sub(1)),
        });
    }
}
/// RuboCop's corrector: an explicit match against `$_`.
fn correction(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> Option<crate::offense::Correction> {
    let source = std::str::from_utf8(node.location().as_slice()).ok()?;
    if let Some(bang) = cx.parent().and_then(|p| p.as_call_node()).filter(|c| {
        c.name().as_slice() == b"!"
            && c.receiver()
                .is_some_and(|r| r.location().start_offset() == node.location().start_offset())
    }) {
        let loc = bang.location();
        return Some(crate::offense::Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: format!("!({source} =~ $_)"),
        });
    }
    let loc = node.location();
    Some(crate::offense::Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!("{source} =~ $_"),
    })
}
fn regexp(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::RegularExpressionNode { .. }
            | Node::InterpolatedRegularExpressionNode { .. }
            | Node::MatchLastLineNode { .. }
            | Node::InterpolatedMatchLastLineNode { .. }
    )
}
fn conditional(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::CaseNode { .. }
            | Node::CaseMatchNode { .. }
    ) || node.as_while_node().is_some_and(|n| !n.is_begin_modifier())
        || node.as_until_node().is_some_and(|n| !n.is_begin_modifier())
}
// Port of parser's check_condition, not Prism's broader implicit-match flags.
// Negation invokes check_condition independently, even outside an if predicate.
fn converted(node: &Node<'_>, ancestors: &[&Node<'_>]) -> bool {
    let mut child = node;
    for (i, parent) in ancestors.iter().enumerate() {
        let predicate = if let Some(n) = parent.as_if_node() {
            Some(n.predicate())
        } else if let Some(n) = parent.as_unless_node() {
            Some(n.predicate())
        } else if let Some(n) = parent.as_while_node() {
            Some(n.predicate())
        } else {
            parent.as_until_node().map(|n| n.predicate())
        };
        if let Some(predicate) = predicate {
            if ancestors
                .get(i + 1)
                .is_some_and(|n| n.as_in_node().is_some())
            {
                return false;
            }
            return predicate.location().start_offset() == child.location().start_offset()
                && predicate.location().end_offset() == child.location().end_offset();
        }
        if let Some(call) = parent.as_call_node() {
            return call.name().as_slice() == b"!"
                && call.call_operator_loc().is_none()
                && call.arguments().is_none()
                && call.block().is_none();
        }
        let transparent = match parent {
            Node::ParenthesesNode { .. }
            | Node::AndNode { .. }
            | Node::OrNode { .. }
            | Node::RangeNode { .. }
            | Node::FlipFlopNode { .. } => true,
            Node::StatementsNode { .. } => parent.as_statements_node().unwrap().body().len() == 1,
            _ => false,
        };
        if !transparent {
            return false;
        }
        child = parent;
    }
    false
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/regexp_as_condition_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/RegexpAsCondition")
            .collect()
    }
    #[test]
    fn pinned_conditions_and_bang() {
        for (s, col) in [
            ("if /foo/\nend\n", 4),
            ("if !/foo/\nend\n", 5),
            ("while /foo/\n bar\nend\n", 7),
            ("until /foo/\n bar\nend\n", 7),
            ("/foo/ ? a : b\n", 1),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(
                found[0].message,
                "Do not use regexp literal as a condition. The regexp literal matches `$_` implicitly."
            );
        }
    }
    #[test]
    fn canonical_bang_in_closed_inactive_branch() {
        crate::cops::testing::assert_correction(
            &super::RegexpAsCondition,
            "if false\n  if !/foo/\n  end\nend\n",
            "if false\n  if !(/foo/ =~ $_)\n  end\nend\n",
        );
    }
    #[test]
    fn nested_ignored_match_requires_the_outer_callback_to_actually_report() {
        for (source, expected) in [
            ("if /#{if /inner/; nil; end}/; end", vec![(1, 4)]),
            ("!/#{if /inner/; nil; end}/", vec![(1, 8)]),
            (
                "begin; nil; end while /#{if /inner/; nil; end}/",
                vec![(1, 29)],
            ),
            ("if x; /#{if /inner/; nil; end}/; end", vec![(1, 13)]),
        ] {
            let found: Vec<_> = offenses(source).iter().map(|o| (o.line, o.col)).collect();
            assert_eq!(found, expected, "{source}");
        }
    }
    #[test]
    fn heredoc_interpolation_uses_ignored_source_range_not_ancestry() {
        for (source, expected) in [
            (
                "if /#{<<TEXT}/\n#{if /inner/; nil; end}\nTEXT\nend\n",
                vec![(1, 4), (2, 6)],
            ),
            (
                "if /#{<<TEXT}\n#{if /inner/; nil; end}\nTEXT\n/\nend\n",
                vec![(1, 4)],
            ),
        ] {
            let found: Vec<_> = offenses(source).iter().map(|o| (o.line, o.col)).collect();
            assert_eq!(found, expected, "{source}");
        }
    }
    #[test]
    fn pinned_lookalikes() {
        for s in ["/foo/", "!/foo/", "if /foo/ =~ str\nend\n"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
