//! `Style/NumericLiterals` — five-character minimum, non-strict decimal grouping.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NumericLiterals;
impl Cop for NumericLiterals {
    fn name(&self) -> &'static str {
        "Style/NumericLiterals"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(node, Node::IntegerNode { .. } | Node::FloatNode { .. })
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(loc) = super::numeric_literal::location(node, cx, true) else {
            return;
        };
        let int = super::numeric_literal::integer_part(loc.as_slice(), file);
        if int.starts_with(b"0") {
            return;
        }
        // Upstream counts source characters after CRLF normalization. Folded
        // signs may include comments/newlines; count only up to MinDigits.
        let mut rest = int;
        for _ in 0..5 {
            if rest.is_empty() {
                return;
            }
            let width = if rest.starts_with(b"\r\n") {
                2
            } else {
                file.encoded_character_width(rest)
            };
            rest = &rest[width..];
        }
        // Non-strict: short trailing groups are fine; internal groups must
        // contain three digits. MinDigits counts source underscores upstream,
        // and Ruby's ^/$ anchors match physical lines in folded sign ranges.
        static BAD: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {
            regex::bytes::Regex::new(r"(?m-u:^[0-9]+$|[0-9]{4}|_[0-9]{1,2}_)").unwrap()
        });
        if !BAD.is_match(int) {
            return;
        }
        let (line, col) = file.line_col(loc.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| correction(&loc))
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message:
                "Use underscores(_) as thousands separator and separate every 3 digits with them."
                    .into(),
            correction,
        });
    }
}
/// RuboCop's `format_number`.
fn correction(loc: &ruby_prism::Location<'_>) -> Option<crate::offense::Correction> {
    let source: Vec<u8> = loc
        .as_slice()
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace() && *b != 0x0b)
        .collect();
    let split = source
        .iter()
        .position(|b| matches!(b, b'.' | b'e' | b'E'))
        .unwrap_or(source.len());
    let (int_part, rest) = source.split_at(split);
    let (negative, digits) = match int_part.split_first() {
        Some((b'-', d)) => (true, d),
        Some((b'+', d)) => (false, d),
        _ => (false, int_part),
    };
    if digits.is_empty()
        || digits.starts_with(b"_")
        || digits.ends_with(b"_")
        || !digits.iter().all(|b| b.is_ascii_digit() || *b == b'_')
    {
        return None;
    }
    let digits: Vec<u8> = digits.iter().copied().filter(|&b| b != b'_').collect();
    let first = digits
        .iter()
        .position(|&b| b != b'0')
        .unwrap_or(digits.len() - 1);
    let digits = &digits[first..];
    let mut formatted = String::new();
    if negative && digits != b"0" {
        formatted.push('-');
    }
    for (i, b) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            formatted.push('_');
        }
        formatted.push(char::from(*b));
    }
    formatted.push_str(std::str::from_utf8(rest).ok()?);
    Some(crate::offense::Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: formatted,
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/numeric_literals_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/NumericLiterals")
            .collect()
    }
    #[test]
    fn groups_integer_parts_and_leaves_fraction_and_exponent_exact() {
        for (before, after) in [
            ("12345", "12_345"),
            ("123456.789", "123_456.789"),
            ("123_456_78_90_00", "123_456_789_000"),
            ("1_8192", "18_192"),
            ("12345e3", "12_345e3"),
            ("-123456", "-123_456"),
            ("-123456.78", "-123_456.78"),
            ("12345.6e3", "12_345.6e3"),
            ("12345.6E3", "12_345.6E3"),
            (
                "123456789012345678901234567890",
                "123_456_789_012_345_678_901_234_567_890",
            ),
        ] {
            let source = format!("# header\n  {before}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                "Use underscores(_) as thousands separator and separate every 3 digits with them."
            );
            assert_correction(&NumericLiterals, &source, &format!("# header\n  {after}\n"));
        }
    }
    #[test]
    fn accepts_short_suffix_groups_small_numbers_and_other_bases() {
        for s in [
            "123_456_789_00",
            "819_2",
            "1_23",
            "123_456",
            "123_456.55",
            "123",
            "-1230",
            "123.456",
            "0b1010101010101",
            "01717171717171",
            "0xab11111111bb",
            "0d123456",
            "10e10",
            "3e12345",
            "12.345e3",
            "12345r",
            "12345i",
            "12345.67ri",
            "'12345'",
            "# 12345\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn minimum_digits_counts_normalized_source_characters() {
        for source in ["-#é\n1\n", "-\r\n123\n"] {
            assert!(offenses(source).is_empty(), "{source:?}");
            assert!(crate::cops::testing::correction_offenses(&NumericLiterals, source).is_empty());
        }
        for source in ["-#é\n12\n", "-\r\n1234\n"] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
        }
    }
}
