//! `Style/ExponentialNotation` — pinned scientific mantissa spelling.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ExponentialNotation;
impl Cop for ExponentialNotation {
    fn name(&self) -> &'static str {
        "Style/ExponentialNotation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_float_node().is_none()
            || cx
                .parent()
                .is_some_and(|n| n.as_imaginary_node().is_some() || n.as_rational_node().is_some())
        {
            return;
        }
        // The pinned Prism adapter folds exactly one prefix-minus CallNode
        // with a direct numeric child, joining its source range to the float.
        // Do not absorb outer calls, grouping, or explicit `float.-@` sends.
        let signed = cx.parent().filter(|parent| {
            ast::negated_numeric_literal(parent)
                .is_some_and(|n| n.location().start_offset() == node.location().start_offset())
        });
        let location = signed.unwrap_or(node).location();
        let source = location.as_slice();
        // Deliberately match the pinned lowercase-e spelling policy, not a
        // floating-point approximation (uppercase E and imaginary are ignored).
        let Some(e) = source.iter().position(|&b| b == b'e') else {
            return;
        };
        // Ruby's /^...$/ anchors match physical lines, including when the
        // folded sign and literal occupy different lines.
        let valid = source[..e].split(|&b| b == b'\n').any(|mantissa| {
            let mantissa = mantissa.strip_prefix(b"-").unwrap_or(mantissa);
            matches!(mantissa.first(), Some(b'1'..=b'9'))
                && (mantissa.len() == 1
                    || (mantissa.get(1) == Some(&b'.')
                        && mantissa.len() > 2
                        && mantissa[2..].iter().all(u8::is_ascii_digit)))
        });
        if valid {
            return;
        }
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(location),
            message: "Use a mantissa >= 1 and < 10.".into(),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/exponential_notation_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/ExponentialNotation")
            .collect()
    }
    #[test]
    fn scientific_mantissa_offenses() {
        for s in [
            "10e6", "12.34e3", "0.314e1", "-10e6", "0e0", "1_0e3", "1.2_3e4",
        ] {
            let found = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].message, "Use a mantissa >= 1 and < 10.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn separated_minus_uses_the_upstream_float_range() {
        for (s, line, col, end_line) in [
            ("- 10e6", 1, 1, 1),
            ("- 1e6", 1, 1, 1),
            ("- -1e6", 1, 1, 1),
            ("- +1e6", 1, 1, 1),
            ("- - 10e6", 1, 3, 1),
            ("+ - 10e6", 1, 3, 1),
            ("-\n10e6", 1, 1, 2),
            ("# header\n  - 1e6", 2, 3, 2),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line)
            );
            assert_eq!(found[0].message, "Use a mantissa >= 1 and < 10.");
            assert!(found[0].correction.is_none());
        }
        for s in ["-\n1e6", "- + 1e6", "-(1e6)", "- 10e6i", "- 10E6"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn pinned_no_offense_and_token_lookalikes() {
        for s in [
            "120.03", "0.07390", "1e6", "3.1415e3", "-9.999e3", "5.02e-3", "1e+0", "1.0e0",
            "1e1_0", "10E6", "10e6i", "10r", "0xeeee", "\"10e6\"", "# 10e6\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
