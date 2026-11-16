//! `Style/NestedTernaryOperator` — nested ternaries at any descendant depth.
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NestedTernaryOperator;
impl Cop for NestedTernaryOperator {
    fn name(&self) -> &'static str {
        "Style/NestedTernaryOperator"
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
        if !ternary(node) || cx.ancestors().any(ternary) {
            return;
        }
        // Only disjoint outermost ternaries walk their subtrees. Every node is
        // inspected at most once, including typed children and block bodies.
        let mut nested = Nested {
            corrections_requested: cx.corrections_requested(),
            root_parens: cx.corrections_requested()
                && modifier_position(node, cx.ancestors().find(|n| !transparent(n))),
            ..Nested::default()
        };
        walk(&mut nested, node);
        if nested.offenses.is_empty() {
            return;
        }
        let mut correction = cx
            .corrections_requested()
            .then(|| rewrite(node, &nested.targets, cx, file))
            .flatten();
        for n in &nested.offenses {
            let (line, col) = file.line_col(n.start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(n.end_offset().saturating_sub(1).max(n.start_offset())),
                message: "Ternary operators must not be nested. Prefer `if` or `else` constructs instead.".into(),
                correction: correction.take(),
            });
        }
    }
}
fn ternary(n: &Node<'_>) -> bool {
    n.as_if_node().is_some_and(|c| c.if_keyword_loc().is_none())
}
#[derive(Default)]
struct Nested<'pr> {
    stack: Vec<(Node<'pr>, bool, bool)>,
    offenses: Vec<ruby_prism::Location<'pr>>,
    targets: Vec<(Node<'pr>, bool)>,
    root_parens: bool,
    corrections_requested: bool,
}
impl<'pr> NodeVisitor<'pr> for Nested<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        VISITS.with(|v| v.set(v.get() + 1));
        if ternary(&node) && !self.stack.is_empty() {
            self.offenses.push(node.location());
            if self.corrections_requested {
                for (n, nested, _) in &mut self.stack {
                    if ternary(n) {
                        *nested = true;
                    }
                }
            }
        }
        let parens = self.corrections_requested
            && ternary(&node)
            && if self.stack.is_empty() {
                self.root_parens
            } else {
                modifier_position(
                    &node,
                    self.stack
                        .iter()
                        .rev()
                        .map(|(n, _, _)| n)
                        .find(|n| !transparent(n)),
                )
            };
        self.stack.push((node, false, parens));
    }
    fn leave(&mut self) {
        let (node, nested, parens) = self.stack.pop().unwrap();
        if nested {
            self.targets.push((node, parens));
        }
    }
}
fn transparent(n: &Node<'_>) -> bool {
    n.as_arguments_node().is_some() || n.as_statements_node().is_some_and(|s| s.body().len() == 1)
}
fn first_arg(n: &Node<'_>, arg: Option<ruby_prism::ArgumentsNode<'_>>) -> bool {
    arg.and_then(|a| a.arguments().first())
        .is_some_and(|a| a.location().start_offset() == n.location().start_offset())
}
fn modifier_position(n: &Node<'_>, parent: Option<&Node<'_>>) -> bool {
    let Some(p) = parent else { return false };
    // An implicit hash can hide a bare command argument behind an AssocNode.
    // Always delimit rewritten keys, including targets inside the subtree walk.
    if let Some(a) = p.as_assoc_node() {
        return a.key().location().start_offset() == n.location().start_offset();
    }
    if matches!(
        p,
        Node::ReturnNode { .. } | Node::BreakNode { .. } | Node::NextNode { .. }
    ) {
        return true;
    }
    if let Some(c) = p.as_call_node() {
        return c.opening_loc().is_none()
            && (c.name().as_slice() == b"!"
                && c.message_loc().is_some_and(|l| l.as_slice() == b"not")
                || first_arg(n, c.arguments()));
    }
    if let Some(c) = p.as_defined_node() {
        return c.lparen_loc().is_none();
    }
    if let Some(c) = p.as_yield_node() {
        return c.lparen_loc().is_none() && first_arg(n, c.arguments());
    }
    if let Some(c) = p.as_super_node() {
        return c.lparen_loc().is_none() && first_arg(n, c.arguments());
    }
    false
}
fn separator(
    file: &SourceFile,
    loc: ruby_prism::Location<'_>,
    within: std::ops::Range<usize>,
    replacement: String,
) -> Correction {
    let mut range = loc.start_offset()..loc.end_offset();
    while range.start > within.start && file.bytes[range.start - 1].is_ascii_whitespace() {
        range.start -= 1;
    }
    while range.end < within.end && file.bytes[range.end].is_ascii_whitespace() {
        range.end += 1;
    }
    Correction { range, replacement }
}
fn rewrite(
    node: &Node<'_>,
    targets: &[(Node<'_>, bool)],
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    if range.len() > 16 * 1024
        || super::correction::contains_comment(file, range.clone())
        || super::correction::touches_directive(file, range.clone())
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node)
        || file.heredoc_pending_at(root, range.start)
        || file.bytes[range.clone()]
            .windows(8)
            .any(|s| s == b"__LINE__")
    {
        return None;
    }
    let end = file.line_start(file.line_of(range.start) + 1);
    let nl = if file.bytes.get(end.saturating_sub(2)..end) == Some(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    #[cfg(test)]
    REWRITE_BYTES.with(|v| v.set(v.get() + range.len()));
    let mut edits = Vec::new();
    for (target, parens) in targets {
        let c = target.as_if_node()?;
        if super::multiline_if_then::ends_with_open_range(&c.predicate()) {
            return None;
        }
        let span = target.location().start_offset()..target.location().end_offset();
        let yes = c.statements()?.body().first()?;
        // Only branch-wide parentheses are removed, never delimiters of a
        // receiver or binary operand. Multiple statements remain branch-local.
        if let Some(p) = yes.as_parentheses_node() {
            let body = p.body()?;
            if body
                .as_statements_node()
                .is_some_and(|s| s.body().is_empty())
            {
                return None;
            }
            for l in [p.opening_loc(), p.closing_loc()] {
                edits.push(Correction {
                    range: l.start_offset()..l.end_offset(),
                    replacement: String::new(),
                });
            }
        }
        edits.push(separator(
            file,
            c.then_keyword_loc()?,
            span.clone(),
            nl.into(),
        ));
        let colon = c.subsequent()?.as_else_node()?.else_keyword_loc();
        edits.push(separator(
            file,
            colon,
            span.clone(),
            format!("{nl}else{nl}"),
        ));
        edits.push(Correction {
            range: span.start..span.start,
            replacement: if *parens { "(if " } else { "if " }.into(),
        });
        edits.push(Correction {
            range: span.end..span.end,
            replacement: format!("{nl}end{}", if *parens { ")" } else { "" }),
        });
    }
    let edit = super::correction::combine(file, edits)?;
    Some(edit)
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Ported from RuboCop nested_ternary_operator_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/nested_ternary_operator_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 15);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/NestedTernaryOperator")
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                case["positions"],
                "{case}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                case["messages"],
                "{case}"
            );
            if let Some(after) = case["correction"].as_str() {
                crate::cops::testing::assert_correction(&NestedTernaryOperator, source, after);
            } else {
                assert!(
                    crate::cops::testing::correction_offenses(&NestedTernaryOperator, source)
                        .iter()
                        .all(|o| o.correction.is_none()),
                    "{case}"
                );
            }
        }
    }
    #[test]
    fn transfer_arguments_and_nested_receivers_keep_grouping() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "def f; super a ? (b ? 1 : 2) : 3; end\n",
                "def f; super (if a\nb ? 1 : 2\nelse\n3\nend); end\n",
            ),
            (
                "def f; yield a ? (b ? 1 : 2) : 3; end\n",
                "def f; yield (if a\nb ? 1 : 2\nelse\n3\nend); end\n",
            ),
            (
                "loop { break a ? (b ? 1 : 2) : 3 }\n",
                "loop { break (if a\nb ? 1 : 2\nelse\n3\nend) }\n",
            ),
            (
                "loop { next a ? (b ? 1 : 2) : 3 }\n",
                "loop { next (if a\nb ? 1 : 2\nelse\n3\nend) }\n",
            ),
            (
                "(a ? (b ? 1 : 2) : 3).inspect\n",
                "(if a\nb ? 1 : 2\nelse\n3\nend).inspect\n",
            ),
            (
                "a ? (b ? (c ? 1 : 2) : 3) : 4\n",
                "if a\nif b\nc ? 1 : 2\nelse\n3\nend\nelse\n4\nend\n",
            ),
        ] {
            assert_correction(&NestedTernaryOperator, before, after);
        }
    }
    #[test]
    fn hash_keys_retain_expression_delimiters() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "p true ? (false ? 1 : 2) : 3 => 4\n",
                "p (if true\nfalse ? 1 : 2\nelse\n3\nend) => 4\n",
            ),
            (
                "p :first, true ? (false ? 1 : 2) : 3 => 4\n",
                "p :first, (if true\nfalse ? 1 : 2\nelse\n3\nend) => 4\n",
            ),
            (
                "a ? (p b ? (c ? 1 : 2) : 3 => 4) : 5\n",
                "if a\np (if b\nc ? 1 : 2\nelse\n3\nend) => 4\nelse\n5\nend\n",
            ),
        ] {
            assert_correction(&NestedTernaryOperator, before, after);
        }
    }
    #[test]
    fn source_envelope_and_literal_interiors_are_verbatim() {
        crate::cops::testing::assert_correction(
            &NestedTernaryOperator,
            "\u{feff}# coding: utf-8\r\na ? (b ? \"é\r\n  b\" : %q(c\r\n d)) : /x/ # tail\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\nif a\r\nb ? \"é\r\n  b\" : %q(c\r\n d)\r\nelse\r\n/x/\r\nend # tail\r\n__END__\r\npayload\x00\r\n",
        );
    }
    #[test]
    fn subtree_scans_are_disjoint_and_corrections_lazy_bounded() {
        use crate::cops::testing::correction_offenses;
        let source = "a ? (b ? 1 : 2) : 3\n".repeat(6000);
        REWRITE_BYTES.with(|n| n.set(0));
        VISITS.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&NestedTernaryOperator], &source).len(),
            6000
        );
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        assert!(VISITS.with(|n| n.get()) < source.len());
        assert_eq!(
            correction_offenses(&NestedTernaryOperator, &source)
                .iter()
                .filter(|o| o.correction.is_some())
                .count(),
            6000
        );
        assert!(REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let huge = format!("a ? [{}] : (b ? 1 : 2)\n", "1,".repeat(10000));
        assert!(
            correction_offenses(&NestedTernaryOperator, &huge)[0]
                .correction
                .is_none()
        );
    }
    #[test]
    fn non_nested_lookalikes() {
        for source in [
            "a ? b : c\n",
            "if a\n b ? c : d\nelse\n e\nend\n",
            "a ? b : c; d ? e : f\n",
            "'a ? b ? c : d : e'\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&NestedTernaryOperator], source).is_empty(),
                "{source}"
            );
        }
    }
}
