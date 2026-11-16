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
        let Some(c) = candidate(node, file) else {
            return;
        };
        let kind = if c.ternary { "ternary" } else { "if-else" };
        let correction = cx
            .corrections_requested()
            .then(|| correct(&c, file))
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
    body: Option<StatementsNode<'pr>>,
    other: StatementsNode<'pr>,
    boundary: usize,
    else_start: usize,
    else_end: usize,
    end: usize,
    ternary: bool,
}
fn candidate<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Candidate<'pr>> {
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
    // RuboCop's AST takes a `;` after the condition as the `then`.
    let mut boundary = boundary;
    let mut after = boundary;
    while matches!(file.bytes.get(after), Some(b' ' | b'\t')) {
        after += 1;
    }
    if file.bytes.get(after) == Some(&b';') {
        boundary = after + 1;
    }
    Some(Candidate {
        condition,
        body,
        other,
        boundary,
        else_start: else_loc.start_offset(),
        else_end: else_loc.end_offset(),
        end,
        ternary,
    })
}
/// RuboCop's corrector: the condition without its negation (`!=`
/// becomes `==`), and the two branches swapped, whitespace and comments
/// included; an empty `if` branch instead loses the `else` line.
fn correct(c: &Candidate<'_>, file: &SourceFile) -> Option<Correction> {
    let text =
        |r: std::ops::Range<usize>| std::str::from_utf8(&file.bytes[r]).ok().map(str::to_owned);
    let span = |n: &Node<'_>| n.location().start_offset()..n.location().end_offset();
    let condition = c.condition.as_node();
    let receiver = c.condition.receiver()?;
    let replacement = if c.condition.name().as_slice() == b"!" {
        text(span(&receiver))?
    } else {
        let argument = ast::call_arguments(&c.condition).into_iter().next()?;
        let method = if c.condition.name().as_slice() == b"!=" {
            "=="
        } else {
            "=~"
        };
        format!(
            "{} {method} {}",
            text(span(&receiver))?,
            text(span(&argument))?
        )
    };
    let mut edits = vec![Correction {
        range: span(&condition),
        replacement,
    }];
    let empty_if = c.body.as_ref().is_none_or(|b| b.body().is_empty());
    if empty_if {
        let line = file.line_of(c.else_start);
        edits.push(Correction {
            range: file.line_start(line)..file.line_start(line + 1),
            replacement: String::new(),
        });
    } else {
        let (a, b) = if c.ternary {
            (span(&c.body.as_ref()?.as_node()), span(&c.other.as_node()))
        } else {
            (c.boundary..c.else_start, c.else_end..c.end)
        };
        edits.push(Correction {
            range: a.clone(),
            replacement: text(b.clone())?,
        });
        edits.push(Correction {
            range: b,
            replacement: text(a)?,
        });
    }
    super::correction::combine(file, edits)
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
