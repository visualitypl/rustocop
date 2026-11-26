//! `Lint/MixedCaseRange` — suspicious mixed-category literal/regexp ranges.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MixedCaseRange;
impl Cop for MixedCaseRange {
    fn name(&self) -> &'static str {
        "Lint/MixedCaseRange"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut ranges = vec![];
        let mut in_regexp = false;
        if let Some(range) = node.as_range_node() {
            let (Some(left), Some(right)) = (range.left(), range.right()) else {
                return;
            };
            if let (Some(a), Some(b)) = (category(&left, file), category(&right, file))
                && a != b
            {
                let loc = range.location();
                let pos = file.reference_line_col(loc.start_offset());
                ranges.push((loc.start_offset()..loc.end_offset(), pos));
            }
        } else if let Some(regexp) = crate::cops::style::regexp_literal::Regexp::new(node) {
            ranges =
                super::duplicate_regexp_character_class_element::mixed_case_ranges(&regexp, file);
            in_regexp = true;
        }
        for (range, (line, col)) in ranges {
            // Only a regexp range is corrected: `A-z` becomes `A-Za-z`.
            let correction = (in_regexp && cx.corrections_requested())
                .then(|| {
                    regexp_range(&file.bytes[range.clone()]).map(|replacement| {
                        crate::offense::Correction {
                            range: range.clone(),
                            replacement,
                        }
                    })
                })
                .flatten();
            out.push(Offense {
                cop: self.name(), severity: self.severity(), correction,
                message: "Ranges from upper to lower case ASCII letters may include unintended characters. Instead of `A-z` (which also includes several symbols) specify each range individually: `A-Za-z` and individually specify any symbols.".into(),
                line, col, end_line: file.line_of(range.end.saturating_sub(1)),
            });
        }
    }
}
/// Upstream's `regexp_range`: each bound extended to the end of its own
/// letter or digit range (`A-z` gives `A-Za-z`).
fn regexp_range(source: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(source).ok()?;
    let mut parts = text.split('-');
    let (open, close) = (parts.next()?, parts.next()?);
    let range = |c: &str| -> Option<(char, char)> {
        let mut chars = c.chars();
        let ch = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        [('a', 'z'), ('A', 'Z'), ('0', '9')]
            .into_iter()
            .find(|(a, b)| (*a..=*b).contains(&ch))
    };
    let (open_range, close_range) = (range(open)?, range(close)?);
    let join = |a: String, b: String| if a == b { a } else { format!("{a}-{b}") };
    Some(format!(
        "{}{}",
        join(open.to_owned(), open_range.1.to_string()),
        join(close_range.0.to_string(), close.to_owned())
    ))
}

// None = not one Ruby character; Some(0/1/2) = other/lower/upper.
fn category(node: &Node<'_>, file: &SourceFile) -> Option<u8> {
    if node.as_source_file_node().is_some() {
        let value = file.source_path().as_os_str().as_encoded_bytes();
        return value_category(value, file);
    }
    if let Some(string) = node.as_interpolated_string_node() {
        let opening = string.opening_loc()?;
        if !opening.as_slice().starts_with(b"<<") {
            return None;
        }
        // Like the pinned visit_heredoc (and ArrayLiteralInRegexp), drop
        // empty continuation fragments and join non-newline-terminated str
        // children. Never fold interpolation, even when its value is literal.
        let single = opening.as_slice().ends_with(b"'");
        let mut children: Vec<Vec<u8>> = vec![];
        for part in string.parts().iter() {
            let part = part.as_string_node()?;
            for value in crate::cops::ast::require_literal::parts(
                part.content_loc().as_slice(),
                part.unescaped(),
                single,
                file,
            ) {
                if value.is_empty() {
                    continue;
                }
                if let Some(last) = children.last_mut().filter(|last| !last.ends_with(b"\n")) {
                    last.extend(value);
                } else {
                    children.push(value);
                }
            }
        }
        let [value] = children.as_slice() else {
            return None;
        };
        return value_category(value, file);
    }
    let string = node.as_string_node()?;
    let single = string.opening_loc().is_some_and(|l| {
        l.as_slice().starts_with(b"'")
            || l.as_slice().starts_with(b"%q")
            || l.as_slice().starts_with(b"<<'")
            || l.as_slice().starts_with(b"<<-'")
            || l.as_slice().starts_with(b"<<~'")
    });
    // Reuse the pinned source-buffer/main-parse escape alignment. In particular
    // UTF8-MAC raw glyphs compose before parsing; numeric escape results do not.
    let parts = crate::cops::ast::require_literal::parts(
        string.content_loc().as_slice(),
        string.unescaped(),
        single,
        file,
    );
    let [value] = parts.as_slice() else {
        return None;
    };
    value_category(value, file)
}
fn value_category(value: &[u8], file: &SourceFile) -> Option<u8> {
    let count = if file.match_pattern_binary() {
        value.len()
    } else {
        value
            .utf8_chunks()
            .map(|c| c.valid().chars().count() + c.invalid().len())
            .sum()
    };
    (count == 1).then_some(match value {
        [b'a'..=b'z'] => 1,
        [b'A'..=b'Z'] => 2,
        _ => 0,
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/mixed_case_range_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/MixedCaseRange")
            .collect()
    }
    #[test]
    fn literal_ranges_report_whole_expression_not_just_ascii_letter_pairs() {
        for range in [
            "'A'..'z'",
            "'A'...'z'",
            "'a'..'_'",
            "'0'..'Z'",
            "'é'..'z'",
            "\"\\u0041\"..'z'",
        ] {
            let found = offenses(&format!("foo = {range}\n"));
            assert_eq!(found.len(), 1, "{range}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 7, 1));
            assert!(
                found[0]
                    .message
                    .starts_with("Ranges from upper to lower case ASCII letters")
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn static_heredoc_continuations_reduce_to_one_character() {
        for source in [
            "'A'..<<~TEXT\n\\\n\nTEXT\n",
            "'A'...<<~TEXT\n  \\\n\nTEXT\n",
            "<<~TEXT..'A'\n\\\n\nTEXT\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert!(
                found[0]
                    .message
                    .starts_with("Ranges from upper to lower case ASCII letters")
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "'0'..<<~TEXT\n\\\n\nTEXT\n",
            "'A'..<<~TEXT\n\\\nz\nTEXT\n",
            "'A'..<<~TEXT\n\\\n\n\nTEXT\n",
            "'A'..<<~TEXT\nTEXT\n",
            "'A'..<<~'TEXT'\n\\\n\nTEXT\n",
            "'A'..<<~TEXT\n#{value}\nTEXT\n",
            "'A'..<<~TEXT\n#{'z'}\nTEXT\n",
            "'A'..(\"\" \"z\")\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn file_pseudoliteral_uses_supplied_source_path() {
        use crate::{
            config::ResolvedConfig,
            cops::{Registry, select_active},
            pipeline::lint_one,
        };
        use std::path::Path;
        let registry = Registry::new();
        let config = ResolvedConfig::default();
        let active = select_active(&registry, &config, &["Lint/MixedCaseRange".into()], &[]);
        for (path, source, n) in [
            ("A", "__FILE__..'z'", 1),
            ("z", "'A'..__FILE__", 1),
            ("A", "'A'..__FILE__", 0),
            ("example.rb", "__FILE__..'z'", 0),
        ] {
            let report = lint_one(
                Path::new(path),
                Path::new(path),
                Some(source.as_bytes()),
                &active,
                &config,
            );
            assert!(report.errors.is_empty());
            assert_eq!(report.result.offenses.len(), n, "{path}: {source}");
        }
    }
    #[test]
    fn accepted_literal_ranges_and_lookalikes() {
        for range in [
            "'A'..'Z'",
            "'aa'..'z'",
            "'a'..'zz'",
            "(..'z')",
            "('a'..)",
            "'0'..'_'",
            "'é'..'ö'",
            "1..10",
            "a..z",
            "\"#{a}\"..'z'",
            "('A')..'z'",
        ] {
            assert!(offenses(range).is_empty(), "{range}");
        }
    }
    #[test]
    fn regexp_literals_interpolations_octal_negation_extended_and_multiple_ranges() {
        for (source, positions) in [
            ("foo = /[A-z]/", vec![(1, 9)]),
            ("foo = /[#{A-z}A-z#{y}]/", vec![(1, 15)]),
            ("foo = /[_A-b;Z-a!]/", vec![(1, 10), (1, 14)]),
            (
                "foo = %r{[A-z]+/[A-z]+|all}.freeze || /_[A-z]/",
                vec![(1, 11), (1, 18), (1, 42)],
            ),
            (
                "foo = /\n  A-z # not a character class\n  [A-z]_..._\n/x",
                vec![(3, 4)],
            ),
            ("foo = /[^G-f]/", vec![(1, 10)]),
            (r"foo = /[_A-z123]/", vec![(1, 10)]),
            (r"foo = /[\001A-z123]/", vec![(1, 13)]),
            (r"foo = /[\1A-z123]/", vec![(1, 11)]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn regexp_escaped_bounds_and_nonletter_bounds_do_not_count() {
        for pattern in [
            r"[_a-zA-Z0-9;]",
            r"[\101-z]",
            r"[A-\172]",
            r"[\x01-z]",
            r"[A-\x7a]",
            r"[\x41-\x7a]",
            r"[\xA-\xf]",
            r"[\u0041-\u007a]",
            r"[\C-A-\cz]",
            r"[A\-z]",
            r"[a-[bc]]",
            "[[ ]]",
            "[_-a]",
            "[A-_]",
            r"[\–]",
        ] {
            assert!(
                offenses(&format!("foo = /{pattern}/")).is_empty(),
                "{pattern}"
            );
        }
    }
}
