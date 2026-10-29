//! `Style/RedundantFilterChain` — prefer a direct predicate (unsafe correction withheld).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantFilterChain;
impl Cop for RedundantFilterChain {
    fn name(&self) -> &'static str {
        "Style/RedundantFilterChain"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(predicate) = node.as_call_node() else {
            return;
        };
        let prefer = match predicate.name().as_slice() {
            b"any?" => "any?",
            b"empty?" | b"none?" => "none?",
            b"one?" => "one?",
            _ => return,
        };
        if predicate
            .arguments()
            .is_some_and(|a| !a.arguments().is_empty())
            || predicate.block().is_some()
        {
            return;
        }
        let Some(select) = predicate.receiver().and_then(|n| n.as_call_node()) else {
            return;
        };
        if !matches!(
            select.name().as_slice(),
            b"select" | b"filter" | b"find_all"
        ) {
            return;
        }
        let Some(block) = select.block() else { return };
        // Prism stores a passed block separately from ordinary arguments.
        // Both matcher branches require no ordinary filter arguments.
        if select
            .arguments()
            .is_some_and(|a| !a.arguments().is_empty())
            || (block.as_block_node().is_none() && block.as_block_argument_node().is_none())
        {
            return;
        }
        let Some(start) = select.message_loc() else {
            return;
        };
        let Some(end) = predicate.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(start.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(end),
            correction: None,
            message: format!(
                "Use `{prefer}` instead of `{}.{}`.",
                String::from_utf8_lossy(select.name().as_slice()),
                String::from_utf8_lossy(predicate.name().as_slice())
            ),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop redundant_filter_chain_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantFilterChain")
            .collect()
    }
    #[test]
    fn all_filter_predicate_pairs_and_block_forms() {
        for method in ["select", "filter", "find_all"] {
            for (predicate, prefer) in [
                ("any?", "any?"),
                ("empty?", "none?"),
                ("none?", "none?"),
                ("one?", "one?"),
            ] {
                for block in [
                    " { |x| x > 1 }",
                    " { _1 > 1 }",
                    " { it > 1 }",
                    "(&:odd?)",
                    "(&callback)",
                ] {
                    for (dot, col) in [(".", 5), ("&.", 6)] {
                        let s = format!("arr{dot}{method}{block}{dot}{predicate}");
                        let found = offenses(&s);
                        assert_eq!(found.len(), 1, "{s}");
                        assert_eq!((found[0].line, found[0].col), (1, col));
                        assert_eq!(
                            found[0].message,
                            format!("Use `{prefer}` instead of `{method}.{predicate}`.")
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn fixed_no_active_support_and_predicate_arguments_or_blocks() {
        for s in [
            "arr.select { |x| x > 1 }.many?",
            "arr.select { it }.present?",
            "arr.any? { it }",
            "relation.select(:name).any?",
            "foo.select.any?",
            "arr.select(&:odd?).any?(Integer)",
            "arr.select(&:odd?).any? { |x| x > 10 }",
            "arr.select(&:odd?).any?(&:even?)",
            "arr.select(:name, &callback).none?",
            "arr.map { it }.none?",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn filter_arguments_are_excluded_even_with_a_literal_block() {
        for method in ["select", "filter", "find_all"] {
            for block in [" { it }", " { _1 }", " { |x| x }", "(&callback)"] {
                let s = format!("xs.{method}(1){block}.any?");
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
    }
    #[test]
    fn captures_arity_control_flow_and_lazy_receivers_stay_detection_only() {
        for block in [
            " { |x,y; seen| seen = x; x > y }",
            " { |x| next x }",
            " { |x| break x }",
            " { return true }",
            " { |*xs| xs }",
            " {}",
        ] {
            let s = format!("side_effect().lazy.select{block}.any?");
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none());
        }
        let found = offenses("arr\n  .select { it }\n  .none?");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 4, 3));
    }
}
