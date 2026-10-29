//! `Style/EachForSimpleLoop` — fixed-count, parameterless range iteration.
use super::correction::{contains_comment, preserves_data_positions, preserves_line_values};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EachForSimpleLoop;
impl Cop for EachForSimpleLoop {
    fn name(&self) -> &'static str {
        "Style/EachForSimpleLoop"
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
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"each" || !ast::call_arguments(&call).is_empty() {
            return;
        }
        let Some(block) = call.block().and_then(|n| n.as_block_node()) else {
            return;
        };
        if block.parameters().is_some_and(|p| {
            p.as_block_parameters_node()
                .is_none_or(|p| !p.locals().is_empty() || p.parameters().is_some())
        }) {
            return;
        }
        let Some(paren) = call.receiver().and_then(|n| n.as_parentheses_node()) else {
            return;
        };
        let Some(body) = paren.body().and_then(|n| n.as_statements_node()) else {
            return;
        };
        if body.body().len() != 1 {
            return;
        }
        let Some(range) = body.body().iter().next().and_then(|n| n.as_range_node()) else {
            return;
        };
        let Some(min) = range.left().and_then(|n| n.as_integer_node()) else {
            return;
        };
        let Some(max) = range.right().and_then(|n| n.as_integer_node()) else {
            return;
        };
        let start = call.location().start_offset();
        let end = ast::source_end(node);
        let correction = cx
            .corrections_requested()
            .then(|| {
                // each returns the Range, times the Integer. Only a top-level
                // discarded statement is currently proven not to expose this.
                let mut parents = cx.ancestors();
                if !parents
                    .next()
                    .is_some_and(|p| p.as_statements_node().is_some())
                    || !parents
                        .next()
                        .is_some_and(|p| p.as_program_node().is_some())
                {
                    return None;
                }
                let root = cx.ancestors().next_back()?;
                if file.each_with_object_facts(root).custom_dispatch
                    || file.class_method_bindings(root).observes_sources
                    || contains_comment(file, start..end)
                {
                    return None;
                }
                let min = integer_value(min.value())?;
                let max = integer_value(max.value())?;
                let count = max
                    .checked_sub(min)?
                    .checked_add(i128::from(!range.is_exclude_end()))?;
                let replacement = if count < 0 {
                    format!("({count}).times")
                } else {
                    format!("{count}.times")
                };
                let edit = Correction {
                    range: start..end,
                    replacement,
                };
                (preserves_line_values(file, &edit) && preserves_data_positions(file, cx, &edit))
                    .then_some(edit)
            })
            .flatten();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message:
                "Use `Integer#times` for a simple loop which iterates a fixed number of times."
                    .into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction,
        });
    }
}
// Prism's decoded integer limbs avoid reparsing radix/underscore spellings.
// Larger-than-i128 endpoints/counts remain diagnosed but unedited.
fn integer_value(value: ruby_prism::Integer<'_>) -> Option<i128> {
    let (negative, digits) = value.to_u32_digits();
    if digits.len() > 4 {
        return None;
    }
    let magnitude = digits
        .iter()
        .rev()
        .fold(0u128, |n, digit| (n << 32) | u128::from(*digit));
    if negative && magnitude == (1u128 << 127) {
        return Some(i128::MIN);
    }
    let magnitude = i128::try_from(magnitude).ok()?;
    Some(if negative { -magnitude } else { magnitude })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop each_for_simple_loop_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == EachForSimpleLoop.name())
            .collect()
    }
    #[test]
    fn pinned_diagnostics_and_all_eleven_correction_pairs() {
        for (range, count, dot, block) in [
            ("0...10", 10, ".", " { do_something }"),
            ("0...10", 10, ".", " do\n  do_something\nend"),
            ("0...10", 10, "&.", " { do_something }"),
            ("0...10", 10, "&.", " do\n  do_something\nend"),
            ("0..10", 11, ".", " {}"),
            ("0..10", 11, ".", " do\nend"),
            ("3..7", 5, ".", " do\nend"),
            ("3..7", 5, "&.", " do\nend"),
            ("0...10", 10, ".", " {}"),
            ("0...10", 10, ".", " do\nend"),
            ("3...7", 4, ".", " do\nend"),
        ] {
            let s = format!("({range}){dot}each{block}\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 1, 1));
            assert_eq!(
                o[0].message,
                "Use `Integer#times` for a simple loop which iterates a fixed number of times."
            );
            assert_correction(&EachForSimpleLoop, &s, &format!("{count}.times{block}\n"));
        }
    }
    #[test]
    fn pinned_negatives_and_parameter_arity() {
        for s in [
            "(a..10).each {}",
            "(0..b).each {}",
            "('a'..'b').each {}",
            "(0...10).each { |n| p n }",
            "(3..7).each do |n|\nend",
            "(3...7).each do |n|\nend",
            "(0...10)&.each { |n| p n }",
            "(0..3).each { p _1 }",
            "(0..3).each { p it }",
            "(0..3).each { |;local| }",
            "(0..3).each(1) {}",
            "(0..3).each(&:work)",
            "(0..3).each",
            "(0..).each {}",
            "(..3).each {}",
            "(0.0..3).each {}",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn return_value_and_source_sensitive_edits_are_withheld_not_hidden() {
        for s in [
            "x = (0..3).each {}",
            "p((0..3).each {})",
            "def m; (0..3).each {}; end",
            "(0..3).each {}.inspect",
            "(0..\n3).each {}; p __LINE__",
            "(0..3).each {}; p DATA.pos\n__END__\nraw",
            "class Integer; def times; end; end\n(0..3).each {}",
        ] {
            let o = correction_offenses(&EachForSimpleLoop, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn large_integers_do_not_wrap_or_hide_offenses() {
        assert_correction(
            &EachForSimpleLoop,
            "(999999999999999999999999..1000000000000000000000001).each {}",
            "3.times {}",
        );
        let s = "(0..999999999999999999999999999999999999999999999999).each {}";
        let o = correction_offenses(&EachForSimpleLoop, s);
        assert_eq!(o.len(), 1);
        assert!(o[0].correction.is_none());
    }
    #[test]
    fn signed_radix_and_descending_counts() {
        for (range, n) in [("-3..2", "6"), ("0x2...0b101", "3"), ("7..3", "(-3)")] {
            assert_correction(
                &EachForSimpleLoop,
                &format!("({range}).each {{}}"),
                &format!("{n}.times {{}}"),
            );
        }
    }
}
