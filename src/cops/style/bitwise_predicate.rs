//! `Style/BitwisePredicate` — prefer Integer bit predicates (detection only).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct BitwisePredicate;
impl Cop for BitwisePredicate {
    fn name(&self) -> &'static str {
        "Style/BitwisePredicate"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation() {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let grouped = receiver.as_parentheses_node().is_some();
        let Some(bit) = super::even_odd::single_group(receiver).and_then(|n| n.as_call_node())
        else {
            return;
        };
        if bit.name().as_slice() != b"&" || bit.is_safe_navigation() || bit.block().is_some() {
            return;
        }
        let Some(lhs) = bit.receiver() else { return };
        let args = ast::call_arguments(&bit);
        let [rhs] = args.as_slice() else { return };
        let args = ast::call_arguments(&call);
        let number = args
            .first()
            .and_then(|n| super::even_odd::integer(n, Some(file)));
        let method = match (call.name().as_slice(), args.len(), number) {
            (b"positive?", 0, _)
            | (b">", 1, Some(0))
            | (b">=", 1, Some(1))
            | (b"!=", 1, Some(0)) => "anybits?",
            (b"==", 1, _)
                if grouped
                    && (same_flags(&lhs, &args[0], file) || same_flags(rhs, &args[0], file)) =>
            {
                "allbits?"
            }
            (b"zero?", 0, _) | (b"==", 1, Some(0)) => "nobits?",
            _ => return,
        };
        // Match upstream's message orientation without emitting its unsafe edit.
        let (base, flags) =
            if method == "allbits?" && lhs.location().as_slice() == args[0].location().as_slice() {
                (rhs, &lhs)
            } else {
                (&lhs, rhs)
            };
        let preferred = format!(
            "{}.{}({})",
            crate::offense::single_line(base.location().as_slice()),
            method,
            crate::offense::single_line(flags.location().as_slice())
        );
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: format!("Replace with `{preferred}` for comparison with bit flags."),
            correction: None,
        });
    }
}
fn same_flags(a: &Node<'_>, b: &Node<'_>, file: &SourceFile) -> bool {
    ast::fingerprint_with_integer_source_lines(a, file)
        == ast::fingerprint_with_integer_source_lines(b, file)
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/bitwise_predicate_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/BitwisePredicate")
            .collect()
    }
    #[test]
    fn pinned_predicates_and_positions() {
        for (s, preferred) in [
            ("(variable & flags).positive?", "variable.anybits?(flags)"),
            ("(variable & flags) > 0", "variable.anybits?(flags)"),
            ("(variable & flags) >= 1", "variable.anybits?(flags)"),
            ("(variable & flags) != 0", "variable.anybits?(flags)"),
            ("(variable & flags) == flags", "variable.allbits?(flags)"),
            ("(flags & variable) == flags", "variable.allbits?(flags)"),
            ("(variable & flags).zero?", "variable.nobits?(flags)"),
            ("(variable & flags) == 0", "variable.nobits?(flags)"),
            ("variable & flags == 0", "variable.nobits?(flags)"),
        ] {
            let found = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                format!("Replace with `{preferred}` for comparison with bit flags.")
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_no_offense_and_lookalikes() {
        for s in [
            "variable.anybits?(flags)",
            "variable.allbits?(flags)",
            "variable.nobits?(flags)",
            "(variable & flags) > 1",
            "foo == bar",
            "(flags & variable) == flagments",
            "zero?",
            "(x | y).zero?",
            "(x & y) == 0.0",
            "(x & y) >= 0",
            "((x & y)).zero?",
            "(x & y)&.zero?",
            "(x & y).positive?(1)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn line_macros_compare_their_values_not_source_spelling_or_offset() {
        for s in [
            "(x & y) >= __LINE__",
            "(x & __LINE__) == __LINE__",
            "(x & __LINE__) == 1",
            "(x & flags(__LINE__)) == flags(1)",
            "(x & flags(__LINE__)) == flags(__LINE__)",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        assert!(offenses("(x & __LINE__) ==\n__LINE__").is_empty());
    }
    #[test]
    fn equivalent_numeric_flags_are_detection_only() {
        for (left, right) in [
            ("1r", "1.0r"),
            ("0.01r", "0.010r"),
            ("9007199254740993r", "9007199254740993.0r"),
            ("0r", "-0.0r"),
            ("0.0", "-0.0"),
            ("1ri", "1.0ri"),
            ("0.0i", "-0.0i"),
        ] {
            let s = format!("(value & flags({left})) == flags({right})");
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "Replace with `value.allbits?(flags({left}))` for comparison with bit flags."
                )
            );
            assert!(found[0].correction.is_none());
        }
        for s in [
            "(value & flags(1r)) == flags(1.0)",
            "(value & flags(1r)) == flags(1)",
            "(value & flags(0.01r)) == flags(0.02r)",
            "(value & flags(9007199254740993r)) == flags(9007199254740992r)",
            "(value & flags(1ri)) == flags(2ri)",
            "(value & flags(1.0)) == flags(-1.0)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn spaced_integer_signs_match_only_upstream_literal_folding() {
        for s in ["(x & y) > - 0", "(x & y) >= - -1", "(x & y) != -\n0"] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                "Replace with `x.anybits?(y)` for comparison with bit flags."
            );
        }
        for s in ["(x & y) > + 0", "(x & y) > - - 0", "(x & y) > 0.-@"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn exact_integer_values_and_structural_flags() {
        for s in [
            "(x & y) > 0x0",
            "(x & y) >= 0b1",
            "(x & y) != -0",
            "(x & 0x10) == 16",
            "(x & -1) == -1",
            "(x & obj.flags) == obj.flags",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "(x & y) > 0r",
            "(x & y) != 0i",
            "(x & y) >= 1.0",
            "x & y == y",
            "(x & y) == -1",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
