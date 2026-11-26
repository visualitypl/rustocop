//! `Lint/FlipFlop` — conditional inclusive and exclusive flip-flops.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The detection-only `Lint/FlipFlop` cop.
pub struct FlipFlop;

impl Cop for FlipFlop {
    fn name(&self) -> &'static str {
        "Lint/FlipFlop"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !matches!(node, Node::FlipFlopNode { .. } | Node::RangeNode { .. })
            || !conditional_range(node, cx)
        {
            return;
        }
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            message: "Avoid the use of flip-flop operators.".into(),
            line,
            col,
            end_line: file.line_of(node.location().end_offset().saturating_sub(1)),
        });
    }
}

// The pinned Prism-to-parser builder first emits ranges, then converts only
// condition paths through a single-child begin (parentheses), and/or, ranges,
// and syntactic !/not. Explicit begin/end (kwbegin), assignments and calls are
// barriers. Prism itself propagates conditionality through kwbegin, so merely
// matching FlipFlopNode would produce false positives there. Consult ancestors,
// never reparse or rescan the root, and still allow inner independent conditions.
pub(super) fn conditional_range(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    let mut ancestors = cx.ancestors().peekable();
    while let Some(parent) = ancestors.next() {
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
            // Prism represents pattern guards as If/Unless, but the pinned
            // translation uses if_guard/unless_guard, not condition(). Those
            // builders do not convert ranges to flip-flops.
            if ancestors.peek().is_some_and(|n| n.as_in_node().is_some()) {
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
    //! Examples ported from RuboCop's lint/flip_flop_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/FlipFlop")
            .collect()
    }

    #[test]
    fn pinned_inclusive_and_exclusive_examples() {
        for operator in ["..", "..."] {
            let source = format!(
                "DATA.each_line do |line|\nprint line if (line =~ /begin/){operator}(line =~ /end/)\nend\n"
            );
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 15, 2));
            assert_eq!(found[0].message, "Avoid the use of flip-flop operators.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn explicit_begin_is_a_conversion_barrier_but_inner_conditions_are_independent() {
        for operator in ["..", "..."] {
            for expression in [
                format!("begin; a{operator}b; end"),
                format!("begin; a{operator}b; rescue; c{operator}d; end"),
                format!("(x; a{operator}b)"),
                format!("(a{operator}b).!"),
                format!("x = a{operator}b"),
            ] {
                assert!(offenses(&format!("if {expression}; end")).is_empty());
            }
            let found = offenses(&format!("if begin; if a{operator}b; end; end; end"));
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 14));
            assert_eq!(offenses(&format!("!(a{operator}b)")).len(), 1);
        }
    }

    #[test]
    fn pattern_guards_do_not_convert_ranges_but_syntactic_not_still_does() {
        for keyword in ["if", "unless"] {
            assert!(offenses(&format!("case x; in y {keyword} a..b; end")).is_empty());
            let found = offenses(&format!("case x; in y {keyword} !(a..b); end"));
            assert_eq!(found.len(), 1);
        }
    }

    #[test]
    fn ranges_and_text_are_not_flip_flops() {
        for source in [
            "x = 1..2",
            "consume(1...2)",
            "case x; when 1..2; end",
            "[1..2]",
            "'if a..b'",
            "# if a..b\n",
            "if (x >= 5) && (x <= 10); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
