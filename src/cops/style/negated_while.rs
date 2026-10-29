//! `Style/NegatedWhile` — prefer inverse loop keywords for negative conditions.
use super::while_until_modifier::Loop;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NegatedWhile;
impl Cop for NegatedWhile {
    fn name(&self) -> &'static str {
        "Style/NegatedWhile"
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
        let Some(c) = Loop::of(node) else { return };
        if c.post_test {
            return;
        }
        let inverse = if c.word() == "while" {
            "until"
        } else {
            "while"
        };
        let before = out.len();
        // Removing ! bypasses dispatch on *every* test. The existing closed
        // immediate proof establishes only the first test, not the body effects
        // before subsequent tests. Only a direct argument-free break permits a
        // taken iteration; an initial false loop test never executes the body.
        let predicate = if let Some(n) = node.as_while_node() {
            n.predicate()
        } else {
            node.as_until_node().unwrap().predicate()
        };
        let never_enters = super::negated_if::unwrap(predicate, false)
            .and_then(|n| super::negated_if::negation(&n))
            .and_then(|n| n.receiver())
            .is_some_and(|n| {
                if c.word() == "while" {
                    n.as_true_node().is_some()
                } else {
                    n.as_false_node().is_some() || n.as_nil_node().is_some()
                }
            });
        let direct_break = c
            .single_body()
            .and_then(|n| n.as_break_node())
            .is_some_and(|n| n.arguments().is_none());
        let safe = never_enters || direct_break;
        super::negated_if::check_single(self, node, c.keyword, c.predicate, inverse, cx, file, out);
        if !safe && let Some(offense) = out.get_mut(before) {
            offense.correction = None;
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop negated_while_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn negative_prefix_and_modifier_positions() {
        for (word, inverse) in [("while", "until"), ("until", "while")] {
            for (source, line, col) in [
                (format!("{word} !cond\n work\nend\n"), 1, 1),
                (format!("  work {word} not cond\n"), 1, 3),
                (format!("{word} (x = foo; !bar)\nend\n"), 1, 1),
            ] {
                let o: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/NegatedWhile")
                    .collect();
                assert_eq!(o.len(), 1, "{source}");
                assert_eq!((o[0].line, o[0].col), (line, col));
                assert_eq!(
                    o[0].message,
                    format!("Favor `{inverse}` over `{word}` for negative conditions.")
                );
            }
        }
    }
    #[test]
    fn post_test_loops_are_not_reported() {
        for word in ["while", "until"] {
            for predicate in ["!cond", "!true", "!false", "not cond"] {
                let source = format!("begin\n work\nend {word} {predicate}\n");
                assert!(
                    lint_str(&source)
                        .iter()
                        .all(|o| o.cop != "Style/NegatedWhile"),
                    "{source}"
                );
                assert!(correction_offenses(&NegatedWhile, &source).is_empty());
            }
        }
    }
    #[test]
    fn partial_double_and_empty_conditions_are_not_negative() {
        for s in [
            "while !!cond; work; end",
            "work until !cond && other",
            "while not cond or other; work; end",
            "while (); end",
            "until (); end",
            "work while cond",
        ] {
            assert!(
                lint_str(s).iter().all(|o| o.cop != "Style/NegatedWhile"),
                "{s}"
            );
        }
    }
    #[test]
    fn closed_nonrepeating_negation_can_be_corrected() {
        assert_correction(
            &NegatedWhile,
            "while !true\n work\nend\n",
            "until true\n work\nend\n",
        );
        assert_correction(
            &NegatedWhile,
            "until !false\n work\nend\n",
            "while false\n work\nend\n",
        );
        assert_correction(
            &NegatedWhile,
            "while(!false)\n break\nend\n",
            "until(false)\n break\nend\n",
        );
        assert_correction(&NegatedWhile, ":work while !true\n", ":work until true\n");
    }
    #[test]
    fn unknown_dispatch_or_repeated_body_effects_keep_the_offense_without_an_edit() {
        for s in [
            "while !cond\n work\nend",
            "while !false\n work\nend",
            "while !false\n break effect\nend",
        ] {
            let o = correction_offenses(&NegatedWhile, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
}
