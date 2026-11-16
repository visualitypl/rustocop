//! `Style/NestedModifier` — nested postfix conditionals.
use crate::{
    cops::{
        Cop, NodeContext, variables,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct NestedModifier;
impl Cop for NestedModifier {
    fn name(&self) -> &'static str {
        "Style/NestedModifier"
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
        let Some(inner) = Modifier::of(node) else {
            return;
        };
        let mut ancestors = cx.ancestors().filter(|n| !transparent(n));
        let Some(parent) = ancestors.next() else {
            return;
        };
        let Some(outer) = Modifier::of(parent) else {
            return;
        };
        // RuboCop ignores the entire subtree of the first (outermost) pair.
        let mut previous_modifier = true;
        for ancestor in ancestors {
            let modifier = Modifier::of(ancestor).is_some();
            if modifier && previous_modifier {
                return;
            }
            previous_modifier = modifier;
        }
        let (line, col) = file.line_col(inner.keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid using nested modifiers.".into(),
            correction: cx
                .corrections_requested()
                .then(|| rewrite(node, &inner, &outer, cx, file))
                .flatten(),
        });
    }
}
fn transparent(n: &Node<'_>) -> bool {
    n.as_statements_node().is_some_and(|s| s.body().len() == 1)
}
struct Modifier<'pr> {
    keyword: Location<'pr>,
    condition: Node<'pr>,
    conditional: bool,
}
impl<'pr> Modifier<'pr> {
    fn of(n: &Node<'pr>) -> Option<Self> {
        if let Some(c) = n.as_if_node() {
            if c.end_keyword_loc().is_some() {
                return None;
            }
            return Some(Self {
                keyword: c.if_keyword_loc()?,
                condition: c.predicate(),
                conditional: true,
            });
        }
        if let Some(c) = n.as_unless_node() {
            if c.end_keyword_loc().is_some() {
                return None;
            }
            return Some(Self {
                keyword: c.keyword_loc(),
                condition: c.predicate(),
                conditional: true,
            });
        }
        if let Some(c) = n.as_while_node() {
            if c.closing_loc().is_some() || c.is_begin_modifier() {
                return None;
            }
            return Some(Self {
                keyword: c.keyword_loc(),
                condition: c.predicate(),
                conditional: false,
            });
        }
        let c = n.as_until_node()?;
        if c.closing_loc().is_some() || c.is_begin_modifier() {
            return None;
        }
        Some(Self {
            keyword: c.keyword_loc(),
            condition: c.predicate(),
            conditional: false,
        })
    }
}
#[derive(Default)]
struct Hazards {
    binding: bool,
}
impl<'pr> NodeVisitor<'pr> for Hazards {
    fn enter(&mut self, n: Node<'pr>) {
        self.binding |= variables::is_local_assignment(&n)
            || matches!(
                n,
                Node::MatchWriteNode { .. }
                    | Node::MatchPredicateNode { .. }
                    | Node::MatchRequiredNode { .. }
                    | Node::BlockNode { .. }
                    | Node::LambdaNode { .. }
                    | Node::DefNode { .. }
                    | Node::ClassNode { .. }
                    | Node::ModuleNode { .. }
                    | Node::BeginNode { .. }
                    | Node::RescueModifierNode { .. }
                    | Node::RangeNode { .. }
                    | Node::FlipFlopNode { .. }
                    | Node::ReturnNode { .. }
                    | Node::BreakNode { .. }
                    | Node::NextNode { .. }
                    | Node::ForNode { .. }
            );
    }
}
fn rewrite(
    node: &Node<'_>,
    inner: &Modifier<'_>,
    outer: &Modifier<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !inner.conditional || !outer.conditional {
        return None;
    }
    let range = inner.keyword.start_offset()..outer.condition.location().end_offset();
    if range.len() > 16 * 1024
        || super::correction::contains_comment(file, range.clone())
        // Reordering multiline predicates can move __LINE__ even when the
        // aggregate newline count (and every following line) stays unchanged.
        || file.bytes[range.clone()].windows(8).any(|s| s == b"__LINE__")
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node)
        || file.contains_heredoc(root, &outer.condition)
        || file.heredoc_pending_at(root, range.start)
    {
        return None;
    }
    let mut hazards = Hazards::default();
    walk(&mut hazards, &inner.condition);
    walk(&mut hazards, &outer.condition);
    if hazards.binding {
        return None;
    }
    let negate = inner.keyword.as_slice() != outer.keyword.as_slice();
    if negate {
        // Inserting ! is dispatch. Only closed immediate sources before effects
        // can establish that this new call has the built-in truthiness meaning.
        if !super::case_equality::immediate(&inner.condition) {
            return None;
        }
        let proof = file.negated_condition_safety(root);
        if !proof.before_effects(range.end)
            || cx.ancestors().any(|n| {
                matches!(
                    n,
                    Node::WhileNode { .. }
                        | Node::UntilNode { .. }
                        | Node::ForNode { .. }
                        | Node::BlockNode { .. }
                        | Node::LambdaNode { .. }
                ) || n
                    .as_begin_node()
                    .is_some_and(|b| b.rescue_clause().is_some() || b.ensure_clause().is_some())
            })
        {
            return None;
        }
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    let is_if = outer.keyword.as_slice() == b"if";
    let left = operand(&outer.condition, is_if, false)?;
    let right = operand(&inner.condition, true, negate)?;
    let edit = Correction {
        range,
        replacement: format!(
            "{} {left} {} {right}",
            if is_if { "if" } else { "unless" },
            if is_if { "&&" } else { "||" }
        ),
    };
    Some(edit)
}
fn operand(n: &Node<'_>, wrap_or: bool, negate: bool) -> Option<String> {
    let text = std::str::from_utf8(n.location().as_slice()).ok()?;
    let mut source = text.to_owned();
    let mut wrap = n.as_if_node().is_some()
        || n.as_unless_node().is_some()
        || super::redundant_parentheses::is_assignment(n)
        || n.as_rescue_modifier_node().is_some()
        || n.as_defined_node()
            .is_some_and(|d| d.lparen_loc().is_none());
    if let Some(c) = n.as_call_node() {
        let name = c.name().as_slice();
        let prefix_not = name == b"!" && c.message_loc().is_some_and(|l| l.as_slice() == b"not");
        // Setter syntax is a CallNode too, but its complete assignment must
        // stay grouped; it cannot be converted to a parenthesized command call.
        let setter = c.equal_loc().is_some();
        wrap |= prefix_not || setter;
        if !setter
            && c.arguments().is_some_and(|a| !a.arguments().is_empty())
            && c.opening_loc().is_none()
            && (c.receiver().is_none() || c.call_operator_loc().is_some())
        {
            if c.is_safe_navigation() || c.block().is_some() {
                wrap = true;
            } else {
                let at = c.message_loc()?.end_offset() - n.location().start_offset();
                source = format!("{}({})", &text[..at], text[at..].trim_start());
            }
        }
    }
    if let Some(a) = n.as_and_node() {
        wrap |= a.operator_loc().as_slice() == b"and" || negate;
    }
    if let Some(o) = n.as_or_node() {
        wrap |= o.operator_loc().as_slice() == b"or" || wrap_or || negate;
    }
    if let Some(y) = n.as_yield_node() {
        wrap |= y.arguments().is_some() && y.lparen_loc().is_none();
    }
    if let Some(s) = n.as_super_node() {
        wrap |= s.arguments().is_some() && s.lparen_loc().is_none();
    }
    if wrap {
        source = format!("({source})");
    }
    Some(if negate { format!("!{source}") } else { source })
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Ported from RuboCop nested_modifier_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/nested_modifier_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 23);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/NestedModifier")
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
                // Opposite keywords introduce overridable ! dispatch: retain these findings.
                let withheld = [47, 58, 69, 80, 102, 113, 124, 135, 157, 239]
                    .contains(&case["spec_line"].as_u64().unwrap());
                crate::cops::testing::assert_correction(
                    &NestedModifier,
                    source,
                    if withheld { source } else { after },
                );
            } else {
                assert!(
                    crate::cops::testing::correction_offenses(&NestedModifier, source)
                        .iter()
                        .all(|o| o.correction.is_none()),
                    "{case}"
                );
            }
        }
    }
    #[test]
    fn post_test_loops_are_not_basic_modifiers() {
        for source in [
            "begin; work; end while a if b\n",
            "work if (begin; work; end until a)\n",
            "begin; work if a; end until b\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&NestedModifier], source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn binding_control_flow_ranges_and_dispatch_withhold_individual_edits() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "work if a if a = true\n",
            "work if (a = true) if a\n",
            "work if /(?<a>x)/ =~ value if a\n",
            "work if (value in a) if b\n",
            "work if (1..) if true\n",
            "work if true if 1..2\n",
            "work if (begin; a; rescue; b; ensure; c; end) if d\n",
            "work unless arbitrary if outer\n",
        ] {
            let found = correction_offenses(&NestedModifier, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn closed_immediate_mixed_keywords_and_defined_probes() {
        use crate::cops::testing::assert_correction;
        assert_correction(
            &NestedModifier,
            "1 unless false if true\n",
            "1 if true && !false\n",
        );
        assert_correction(
            &NestedModifier,
            "1 if false unless true\n",
            "1 unless true || !false\n",
        );
        assert_correction(
            &NestedModifier,
            "work if b if defined? a\n",
            "work if (defined? a) && b\n",
        );
    }
    #[test]
    fn command_names_include_unicode_and_explicit_operator_dispatch() {
        use crate::cops::testing::assert_correction;
        for (source, expected) in [
            ("work if b if café 1\n", "work if café(1) && b\n"),
            ("work if b if obj.+ 1\n", "work if obj.+(1) && b\n"),
            ("work if b if obj.[] 1\n", "work if obj.[](1) && b\n"),
            ("work if b if obj&.café 1\n", "work if (obj&.café 1) && b\n"),
        ] {
            assert_correction(&NestedModifier, source, expected);
        }
    }
    #[test]
    fn setter_conditions_keep_complete_assignment_operands() {
        use crate::cops::testing::assert_correction;
        for setter in ["obj[1] = 2", "obj.foo = 1", "obj&.foo = 1"] {
            for (keyword, op) in [("if", "&&"), ("unless", "||")] {
                assert_correction(
                    &NestedModifier,
                    &format!("work {keyword} a {keyword} {setter}\n"),
                    &format!("work {keyword} ({setter}) {op} a\n"),
                );
                assert_correction(
                    &NestedModifier,
                    &format!("work {keyword} {setter} {keyword} a\n"),
                    &format!("work {keyword} a {op} ({setter})\n"),
                );
            }
        }
    }
    #[test]
    fn correction_work_is_lazy_and_bounded() {
        use crate::cops::testing::correction_offenses;
        let source = "work if a if b\n".repeat(6000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&NestedModifier], &source).len(),
            6000
        );
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        assert_eq!(
            correction_offenses(&NestedModifier, &source)
                .iter()
                .filter(|o| o.correction.is_some())
                .count(),
            6000
        );
        assert!(REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let huge = format!("work if [{}] if b\n", "1,".repeat(10000));
        assert!(
            correction_offenses(&NestedModifier, &huge)[0]
                .correction
                .is_none()
        );
    }
    #[test]
    fn non_nested_lookalikes() {
        for source in [
            "work if a && b\n",
            "if a\n work if b\nend\n",
            "work if (b if a)\n",
            "work rescue fallback\n",
            "work while a\n",
            "begin\n work\nend while a\n",
            "a ? b : c\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&NestedModifier], source).is_empty(),
                "{source}"
            );
        }
    }
}
