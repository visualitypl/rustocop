//! `Style/NumericLiteralPrefix` — lowercase bases, explicit octal, no decimal prefix.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NumericLiteralPrefix;
impl Cop for NumericLiteralPrefix {
    fn name(&self) -> &'static str {
        "Style/NumericLiteralPrefix"
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
        let Some(loc) = super::numeric_literal::location(node, cx, false) else {
            return;
        };
        let int = super::numeric_literal::integer_part(loc.as_slice(), file);
        // Ruby's ^/$ match physical lines, including a separated prefix sign.
        static PATTERNS: std::sync::LazyLock<[(regex::bytes::Regex, &str); 4]> =
            std::sync::LazyLock::new(|| {
                [
                    (
                        regex::bytes::Regex::new(r"(?m-u:^0O?[0-7]+$)").unwrap(),
                        "Use 0o for octal literals.",
                    ),
                    (
                        regex::bytes::Regex::new(r"(?m-u:^0X[0-9A-F]+$)").unwrap(),
                        "Use 0x for hexadecimal literals.",
                    ),
                    (
                        regex::bytes::Regex::new(r"(?m-u:^0B[01]+$)").unwrap(),
                        "Use 0b for binary literals.",
                    ),
                    (
                        regex::bytes::Regex::new(r"(?m-u:^0[dD][0-9]+$)").unwrap(),
                        "Do not use prefixes for decimal literals.",
                    ),
                ]
            });
        let Some((kind, (_, message))) = PATTERNS
            .iter()
            .enumerate()
            .find(|(_, (re, _))| re.is_match(int))
        else {
            return;
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: (*message).into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file, kind))
                .flatten(),
        });
    }
}
fn correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    kind: usize,
) -> Option<crate::offense::Correction> {
    // Edit only the unsigned numeric token, not an intervening unary send,
    // comment, sign or newline. No value approximation or integer-size limit.
    let token = node.location();
    let sign = usize::from(matches!(token.as_slice().first(), Some(b'+' | b'-')));
    let raw = &token.as_slice()[sign..];
    let old_prefix = match kind {
        0 if raw.starts_with(b"0O") => 2,
        0 => 1,
        _ => 2,
    };
    let prefix = ["0o", "0x", "0b", ""][kind];
    let mut digits = &raw[old_prefix..];
    if kind == 3 {
        // Simply dropping 0d would reinterpret leading zeroes as octal.
        while digits.len() > 1 && digits[0] == b'0' {
            digits = &digits[1..];
        }
    }
    let edit = crate::offense::Correction {
        range: token.start_offset() + sign..token.end_offset(),
        replacement: format!("{prefix}{}", String::from_utf8_lossy(digits)),
    };
    super::literal_spelling::safe_edit(file, cx, edit)
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/numeric_literal_prefix_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/NumericLiteralPrefix")
            .collect()
    }
    #[test]
    fn reports_all_bases_and_preserves_signed_corrections() {
        for (token, after, message) in [
            ("01234", "0o1234", "Use 0o for octal literals."),
            ("0O1234", "0o1234", "Use 0o for octal literals."),
            ("0X1AC", "0x1AC", "Use 0x for hexadecimal literals."),
            ("0B10101", "0b10101", "Use 0b for binary literals."),
            (
                "0d1234",
                "1234",
                "Do not use prefixes for decimal literals.",
            ),
            (
                "0D1990",
                "1990",
                "Do not use prefixes for decimal literals.",
            ),
        ] {
            for sign in ["", "-", "+"] {
                let source = format!("# header\n  {sign}{token}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert_eq!(found[0].message, message);
                let fixed_offenses =
                    crate::cops::testing::correction_offenses(&NumericLiteralPrefix, &source);
                let edit = fixed_offenses[0].correction.as_ref().unwrap();
                let mut fixed = source.clone();
                fixed.replace_range(edit.range.clone(), &edit.replacement);
                assert_eq!(fixed, format!("# header\n  {sign}{after}\n"));
            }
        }
    }
    #[test]
    fn keeps_pinned_nonmatching_forms_and_literal_boundaries() {
        for s in [
            "0",
            "0o101",
            "0b101",
            "0x101",
            "101",
            "0Xabc",
            "0XFF_FF",
            "01_234",
            "0d1_234",
            "01234r",
            "0XABi",
            "01234ri",
            "0.1234",
            "'01234'",
            "# 01234\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn pinned_source_ranges_and_hex_exponent_split_are_explicit() {
        for (s, line, col, after) in [
            ("-\n01234", 1, 1, "-\n0o1234"),
            ("+ 0B10101", 1, 3, "+ 0b10101"),
            ("0XDEAD", 1, 1, "0xDEAD"),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (line, col));
            crate::cops::testing::assert_correction(&NumericLiteralPrefix, s, after);
        }
        // Match the pinned IntegerNode mixin/regexes, not every uppercase token.
        for s in ["- 01234", "0XEEEE", "0Xeeee", "0XabE12", "0XAB_CD"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn decimal_leading_zeroes_never_become_octal() {
        for (s, expected) in [("0d0008", "8"), ("0D000123", "123"), ("0d000", "0")] {
            let found = crate::cops::testing::correction_offenses(&NumericLiteralPrefix, s);
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].correction.as_ref().unwrap().replacement, expected);
        }
    }
}
