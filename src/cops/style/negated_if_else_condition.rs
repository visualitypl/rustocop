//! `Style/NegatedIfElseCondition` — invert negative conditions with two branches.
use super::negated_if::{self, negation, unwrap};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node, StatementsNode};
pub struct NegatedIfElseCondition;
impl Cop for NegatedIfElseCondition {
    fn name(&self) -> &'static str {
        "Style/NegatedIfElseCondition"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/InverseMethods", "Style/Not"]
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(c) = candidate(node) else { return };
        let kind = if c.ternary { "ternary" } else { "if-else" };
        let correction = cx
            .corrections_requested()
            .then(|| correct(node, &c, cx, file))
            .flatten();
        negated_if::report(
            self,
            node,
            file,
            format!("Invert the negated condition and swap the {kind} branches."),
            correction,
            out,
        );
    }
}
struct Candidate<'pr> {
    condition: CallNode<'pr>,
    predicate: Node<'pr>,
    body: Option<StatementsNode<'pr>>,
    other: StatementsNode<'pr>,
    boundary: usize,
    else_start: usize,
    else_end: usize,
    end: usize,
    ternary: bool,
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<Candidate<'pr>> {
    let (predicate, body, other, boundary, else_loc, end, ternary) =
        if let Some(n) = node.as_if_node() {
            if n.if_keyword_loc().is_some_and(|k| k.as_slice() == b"elsif") {
                return None;
            }
            let ternary = n.if_keyword_loc().is_none();
            let e = n.subsequent()?.as_else_node()?;
            (
                n.predicate(),
                n.statements(),
                e.statements()?,
                n.then_keyword_loc()
                    .map(|l| l.end_offset())
                    .unwrap_or(n.predicate().location().end_offset()),
                e.else_keyword_loc(),
                n.end_keyword_loc()
                    .map(|l| l.start_offset())
                    .unwrap_or(e.location().end_offset()),
                ternary,
            )
        } else {
            let n = node.as_unless_node()?;
            let e = n.else_clause()?;
            (
                n.predicate(),
                n.statements(),
                e.statements()?,
                n.then_keyword_loc()
                    .map(|l| l.end_offset())
                    .unwrap_or(n.predicate().location().end_offset()),
                e.else_keyword_loc(),
                n.end_keyword_loc()?.start_offset(),
                false,
            )
        };
    if other.body().is_empty() {
        return None;
    }
    let condition = unwrap(predicate, true)?.as_call_node()?;
    if condition.is_safe_navigation()
        || condition.block().is_some()
        || !matches!(condition.name().as_slice(), b"!" | b"!=" | b"!~")
        || ast::call_arguments(&condition).len() >= 2
        || condition.name().as_slice() == b"!"
            && condition
                .receiver()
                .as_ref()
                .is_some_and(|r| negation(r).is_some())
    {
        return None;
    }
    let predicate = node
        .as_if_node()
        .map(|n| n.predicate())
        .or_else(|| node.as_unless_node().map(|n| n.predicate()))?;
    Some(Candidate {
        condition,
        predicate,
        body,
        other,
        boundary,
        else_start: else_loc.start_offset(),
        else_end: else_loc.end_offset(),
        end,
        ternary,
    })
}
fn correct(
    node: &Node<'_>,
    c: &Candidate<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !negated_if::trusted(&c.condition, cx, file) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let loc = node.location();
    if loc.end_offset() - loc.start_offset() > 16 * 1024
        || file.contains_heredoc(root, node)
        || file.heredoc_pending_at(root, loc.start_offset())
    {
        return None;
    }
    // Branch order is lexical order too: moving a first declaration can turn
    // a vcall into a local or a block-local into a capture. Use VariableForce's
    // declarations, not a second approximation of Ruby assignment syntax.
    let declarations = &file.negated_condition_safety(root).declarations;
    let at = declarations.partition_point(|&p| p < c.boundary);
    if declarations.get(at).is_some_and(|&p| p < c.end) {
        return None;
    }
    let source = &file.bytes[loc.start_offset()..loc.end_offset()];
    if source.windows(8).any(|w| w == b"__LINE__") {
        return None;
    }
    let comments = file.comments();
    let first = comments.partition_point(|r| r.end <= c.boundary);
    if comments[first..]
        .iter()
        .take_while(|r| r.start < c.end)
        .any(|r| !super::correction::comment_relocation_safe(&file.bytes[r.clone()]))
    {
        return None;
    }
    // Only a single transparently grouped expression is correction-eligible.
    // Detection follows upstream's first-child unwrapping of multi-statement
    // begin conditions; changing that first child need not invert their value.
    if !transparent(&c.predicate) {
        return None;
    }
    let receiver = c.condition.receiver()?;
    let replacement = if c.condition.name().as_slice() == b"!" {
        negated_if::text(&receiver)?.to_owned()
    } else {
        let arg = ast::call_arguments(&c.condition).into_iter().next()?;
        format!(
            "{} == {}",
            negated_if::text(&receiver)?,
            negated_if::text(&arg)?
        )
    };
    let mut edits = vec![Correction {
        range: c.condition.location().start_offset()..c.condition.location().end_offset(),
        replacement,
    }];
    if let Some(body) = &c.body {
        let (a, b) = if c.ternary {
            (
                body.location().start_offset()..body.location().end_offset(),
                c.other.location().start_offset()..c.other.location().end_offset(),
            )
        } else {
            let mut boundary = c.boundary;
            while boundary < c.else_start
                && matches!(
                    file.bytes[boundary],
                    b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r'
                )
            {
                boundary += 1;
            }
            if file.bytes.get(boundary) == Some(&b';') {
                boundary += 1;
            } else {
                boundary = c.boundary;
            }
            // A newline-only header relies on the branch's leading separator.
            // An inline else body cannot be pasted directly after its predicate
            // (`if false b`); do not rely on final parse rejection for safety.
            if boundary == c.predicate.location().end_offset() {
                let suffix = &file.bytes[c.else_end..c.end];
                let first = suffix
                    .iter()
                    .copied()
                    .find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c'))?;
                if !matches!(first, b'\n' | b';' | b'#') {
                    return None;
                }
            }
            (boundary..c.else_start, c.else_end..c.end)
        };
        edits.push(Correction {
            range: a.clone(),
            replacement: std::str::from_utf8(&file.bytes[b.clone()]).ok()?.into(),
        });
        edits.push(Correction {
            range: b,
            replacement: std::str::from_utf8(&file.bytes[a]).ok()?.into(),
        });
    } else {
        // Empty if branches become a positive one-branch guard. Keep comments
        // or same-line separators intact by withholding ambiguous removals.
        let start = file.line_start(file.line_of(c.else_start));
        let end = file.line_start(file.line_of(c.else_start) + 1);
        if !file.bytes[start..c.else_start].trim_ascii().is_empty()
            || !file.bytes[c.else_end..end].trim_ascii().is_empty()
            || super::correction::contains_comment(file, c.boundary..c.else_start)
        {
            return None;
        }
        edits.push(Correction {
            range: start..end,
            replacement: String::new(),
        });
    }
    let edit = super::correction::combine(file, edits)?;
    Some(edit)
}
fn transparent(n: &Node<'_>) -> bool {
    if let Some(p) = n.as_parentheses_node() {
        return p.body().as_ref().is_some_and(transparent);
    }
    if let Some(s) = n.as_statements_node() {
        return s.body().len() == 1 && s.body().first().as_ref().is_some_and(transparent);
    }
    if let Some(b) = n.as_begin_node() {
        return b.rescue_clause().is_none()
            && b.ensure_clause().is_none()
            && b.else_clause().is_none()
            && b.statements().is_some_and(|s| {
                s.body().len() == 1 && s.body().first().as_ref().is_some_and(transparent)
            });
    }
    n.as_call_node().is_some()
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop negated_if_else_condition_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    #[test]
    fn negative_if_else_and_ternary_locations_and_messages() {
        for (s, kind) in [
            ("  if !x\n a\nelse\n b\nend", "if-else"),
            ("  !x ? a : b", "ternary"),
            ("  if x != y; a; else b; end", "if-else"),
            ("  (x !~ y) ? a : b", "ternary"),
            ("  unless !x; a; else b; end", "if-else"),
        ] {
            let o: Vec<_> = crate::pipeline::lint_str(s)
                .into_iter()
                .filter(|o| o.cop == "Style/NegatedIfElseCondition")
                .collect();
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 3));
            assert_eq!(
                o[0].message,
                format!("Invert the negated condition and swap the {kind} branches.")
            );
        }
    }
    #[test]
    fn negated_comparison_receivers_are_not_double_negations() {
        for (source, kind) in [
            ("if !x != y; a; else; b; end", "if-else"),
            ("!x !~ y ? a : b", "ternary"),
            ("unless !x != y; a; else; b; end", "if-else"),
        ] {
            let found =
                crate::cops::testing::correction_offenses(&super::NegatedIfElseCondition, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                format!("Invert the negated condition and swap the {kind} branches.")
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn canonical_branch_swaps_comments_then_semicolons_ternaries_and_empty_if() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "if !false\n  a\nelse\n  b\nend\n",
                "if false\n  b\nelse\n  a\nend\n",
            ),
            (
                "if not true then a else b end\n",
                "if true then b else a end\n",
            ),
            ("if !nil; a; else b; end\n", "if nil; b; else a; end\n"),
            (
                "if !nil\x0c; a; else b; end\n",
                "if nil\x0c; b; else a; end\n",
            ),
            ("!false ? a : b\n", "false ? b : a\n"),
            ("(1 != 2) ? a : b\n", "(1 == 2) ? b : a\n"),
            (
                "if !true\n # A\n a\nelse\n # B\n b\nend\n",
                "if true\n # B\n b\nelse\n # A\n a\nend\n",
            ),
            ("if !true\nelse\n b\nend\n", "if true\n b\nend\n"),
            (
                "unless !false\n a\nelse\n b\nend\n",
                "unless false\n b\nelse\n a\nend\n",
            ),
        ] {
            assert_correction(&super::NegatedIfElseCondition, before, after);
        }
    }
    #[test]
    fn relocating_magic_comments_cannot_change_lexical_literal_policy() {
        let source = "if !false\n :a\nelse\n # shareable_constant_value: literal\n :b\nend\n";
        let found =
            crate::cops::testing::correction_offenses(&super::NegatedIfElseCondition, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn branch_declarations_and_multistatement_predicates_keep_offenses_without_edits() {
        for source in [
            "if !true\n a\nelse\n a = 1\nend\n",
            "if !true\n a\nelse b\nend\n",
            "if !true\n :a\nelse\n proc { x = 1 }\nend\n",
            "if begin\n !true\n false\nend\n :a\nelse\n :b\nend\n",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&super::NegatedIfElseCondition, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn rescue_ensure_conditions_are_not_transparent_negations() {
        for condition in ["begin; !x; rescue; y; end", "begin; !x; ensure; y; end"] {
            for s in [
                format!("if {condition}; a; else b; end"),
                format!("{condition} ? a : b"),
            ] {
                assert!(
                    crate::pipeline::lint_str(&s)
                        .iter()
                        .all(|o| o.cop != "Style/NegatedIfElseCondition"),
                    "{s}"
                );
            }
        }
    }
    #[test]
    fn ignores_elsif_empty_else_double_partial_and_multiple_args() {
        for s in [
            "if !x; a; elsif !y; b; else c; end",
            "if !x; a; else; end",
            "if !x; else; end",
            "if (); a; else b; end",
            "if !!x; a; else b; end",
            "if !x && y; a; else b; end",
            "if foo.!=(bar, baz); a; else b; end",
            "if !x; a; end",
        ] {
            assert!(
                crate::pipeline::lint_str(s)
                    .iter()
                    .all(|o| o.cop != "Style/NegatedIfElseCondition"),
                "{s}"
            );
        }
    }
}
