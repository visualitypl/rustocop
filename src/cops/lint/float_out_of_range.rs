//! `Lint/FloatOutOfRange` — overflowing and underflowed float literals.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/FloatOutOfRange` cop (detection only).
pub struct FloatOutOfRange;
impl Cop for FloatOutOfRange {
    fn name(&self) -> &'static str {
        "Lint/FloatOutOfRange"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_float_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(number) = node.as_float_node() else {
            return;
        };
        // The parser adapter folds one prefix minus on a literal. Report that
        // complete spelling, but never treat rational/imaginary literals as floats.
        if cx.parent().is_some_and(|p| p.as_imaginary_node().is_some()) {
            return;
        }
        let location = cx
            .parent()
            .filter(|p| ast::negated_numeric_literal(p).is_some())
            .map_or_else(|| number.location(), |p| p.location());
        if !number.value().is_infinite()
            && !(number.value() == 0.0
                && location.as_slice().iter().any(|b| matches!(b, b'1'..=b'9')))
        {
            return;
        }
        // Keep literal ownership/raw offsets; only the displayed prefix uses
        // the pinned buffer's composition, encoding-header and BOM semantics.
        let (line, col) = file.reference_line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            message: "Float out of range.".into(),
            line,
            col,
            end_line: file.last_line_of(location),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/lint/float_out_of_range_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/FloatOutOfRange")
            .collect()
    }
    #[test]
    fn reports_overflow_and_underflow_with_exact_diagnostics() {
        for number in ["9.9999e999", "1.0e-400"] {
            let found = offenses(&format!("value =\n  {number}\n"));
            assert_eq!(found.len(), 1, "{number}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(found[0].message, "Float out of range.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn exact_source_digit_test_includes_zero_exponents_but_not_complex_components() {
        for source in ["0e1", "0e-1", "-1e400", "--1e400", "1e-400"] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
        for source in ["1e400i", "-1e400i", "0e0", "0.0r", "'1e400'", "# 1e400\n"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn projected_columns_preserve_signed_literal_ownership() {
        for (prefix, line, col) in [
            ("# coding: UTF8-MAC\n'e\u{301}'; ", 2, 6),
            ("# coding: UTF8-MAC\r\n'e\u{301}'; ", 2, 6),
            ("\u{feff}'é'; ", 1, 7),
            ("# coding: UTF-8\n'e\u{301}'; ", 2, 7),
            (" # coding: UTF8-MAC\n'e\u{301}'; ", 2, 7),
        ] {
            for literal in ["1e400", "-1e400", "--1e400", "+1e-400", "-+1e-400"] {
                let source = format!("{prefix}{literal}\r\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source:?}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (line, col, line),
                    "{source:?}"
                );
                assert_eq!(found[0].message, "Float out of range.");
                assert!(found[0].correction.is_none());
            }
            for literal in ["-1e400i", "--1e400i", "-0.0r", "0e0", "5e-324"] {
                assert!(offenses(&format!("{prefix}{literal}\n")).is_empty());
            }
        }
    }

    #[test]
    fn accepts_zero_and_representable_floats() {
        for number in [
            "0.0",
            "1.1e-100",
            "55.7e89",
            "5e-324",
            "1.7976931348623157e308",
        ] {
            assert!(offenses(number).is_empty(), "{number}");
        }
    }
}
