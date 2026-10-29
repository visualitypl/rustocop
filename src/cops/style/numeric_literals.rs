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
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message:
                "Use underscores(_) as thousands separator and separate every 3 digits with them."
                    .into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file))
                .flatten(),
        });
    }
}
fn correction(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    // Group only the actual token. Folding a separated `- 12345` into
    // `-12_345` would remove an observable Integer#-@ dispatch in Ruby.
    let loc = node.location();
    let source = loc.as_slice();
    let sign = usize::from(matches!(source.first(), Some(b'+' | b'-')));
    let end = source
        .iter()
        .position(|b| matches!(b, b'.' | b'e' | b'E'))
        .unwrap_or(source.len());
    let int = &source[sign..end];
    // A spaced unary-minus can make upstream inspect a nondecimal payload.
    // Keep that offense, but don't interpret a base or reserialize its value.
    if int.starts_with(b"0") || !int.iter().all(|b| b.is_ascii_digit() || *b == b'_') {
        return None;
    }
    let digits: Vec<_> = int.iter().copied().filter(|&b| b != b'_').collect();
    let mut replacement = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            replacement.push('_');
        }
        replacement.push(char::from(*b));
    }
    if replacement.as_bytes() == int {
        // Only folding the unary operator could remove this source-based
        // offense on an already canonical short token; never emit a no-op.
        return None;
    }
    let edit = crate::offense::Correction {
        range: loc.start_offset() + sign..loc.start_offset() + end,
        replacement,
    };
    super::literal_spelling::safe_edit(file, cx, edit)
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
    fn source_based_detection_keeps_comments_bases_and_nested_signs_distinct() {
        for (source, col, after) in [
            ("- 12345", 1, "- 12_345"),
            ("+ 12345", 3, "+ 12_345"),
            ("- -12345", 1, "- -12_345"),
            ("- +12345", 1, "- +12_345"),
            ("- # hold\n12345", 1, "- # hold\n12_345"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_correction(&NumericLiterals, source, after);
        }
        // The pinned integer-part split sees the e in a folded sign's comment.
        assert!(offenses("- # keep\n12345").is_empty());
        for s in ["- 01234", "- 0d123456", "- 0XABC12345"] {
            let found = crate::cops::testing::correction_offenses(&NumericLiterals, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn physical_line_anchors_count_folded_source_but_do_not_fold_dispatch() {
        for s in ["- \n123", "- # hold\n1", "- # hold\n0"] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            let found = crate::cops::testing::correction_offenses(&NumericLiterals, s);
            assert!(found[0].correction.is_none(), "{s}");
        }
        assert!(offenses("-\n123").is_empty());
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
    #[test]
    fn separated_minus_has_pinned_detection_but_preserves_operator_dispatch() {
        let s = "a = -\n  12345\n";
        let found = offenses(s);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 5, 2));
        assert_correction(&NumericLiterals, s, "a = -\n  12_345\n");
    }
}
