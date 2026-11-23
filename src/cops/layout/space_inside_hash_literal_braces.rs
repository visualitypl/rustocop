//! `Layout/SpaceInsideHashLiteralBraces` — spaces in nonempty hashes, none in empty hashes.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

use super::brace_spacing::is_ruby_whitespace;

pub struct SpaceInsideHashLiteralBraces;
impl Cop for SpaceInsideHashLiteralBraces {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideHashLiteralBraces"
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
        let (left, right) = if let Some(hash) = node.as_hash_node() {
            (Some(hash.opening_loc()), Some(hash.closing_loc()))
        } else if let Some(pattern) = node.as_hash_pattern_node() {
            (pattern.opening_loc(), pattern.closing_loc())
        } else {
            return;
        };
        let (Some(left), Some(right)) = (left, right) else {
            return;
        };
        if left.as_slice() != b"{" || right.as_slice() != b"}" {
            return;
        }
        let (left, right) = (left.start_offset(), right.start_offset());
        let inside = &file.bytes[left + 1..right];
        if inside.iter().copied().all(is_ruby_whitespace) {
            if !inside.is_empty() {
                super::brace_spacing::report(
                    self,
                    node,
                    cx,
                    file,
                    left + 1..right,
                    left + 1..right,
                    "",
                    "Space inside empty hash literal braces detected.",
                    out,
                );
            }
            return;
        }
        // A following comment or line break exempts the opening gap. AST
        // delimiters distinguish hashes/patterns from interpolation and blocks.
        if !is_ruby_whitespace(inside[0])
            && !inside.starts_with(b"\\\n")
            && !inside.starts_with(b"\\\r\n")
            && file
                .comments()
                .binary_search_by_key(&(left + 1), |r| r.start)
                .is_err()
        {
            super::brace_spacing::report(
                self,
                node,
                cx,
                file,
                left..left + 1,
                left + 1..left + 1,
                " ",
                "Space inside { missing.",
                out,
            );
        }
        let root = cx.ancestors().next_back().unwrap_or(node);
        let protected = file.literal_token_ranges(root);
        let last = right - 1;
        let after = protected.partition_point(|r| r.start <= last);
        let literal_delimiter = after > 0 && protected[after - 1].contains(&last);
        if !is_ruby_whitespace(*inside.last().unwrap())
            || (literal_delimiter && !matches!(file.bytes[last], b'\n' | b'\r'))
        {
            super::brace_spacing::report(
                self,
                node,
                cx,
                file,
                right..right + 1,
                right..right,
                " ",
                "Space inside } missing.",
                out,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_inside_hash_literal_braces_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == SpaceInsideHashLiteralBraces.name())
            .collect()
    }
    #[test]
    fn pinned_nonempty_hashes_and_patterns() {
        for (before, after, positions) in [
            (
                "h = {a: 1, b: 2}",
                "h = { a: 1, b: 2 }",
                vec![(1, 5), (1, 16)],
            ),
            ("h = {a => 1}", "h = { a => 1 }", vec![(1, 5), (1, 12)]),
            ("h = { a: 1}", "h = { a: 1 }", vec![(1, 11)]),
            ("h = {a: '{'}", "h = { a: '{' }", vec![(1, 5), (1, 12)]),
            (
                "case foo\nin {k1: 0, k2: 1}\nend",
                "case foo\nin { k1: 0, k2: 1 }\nend",
                vec![(2, 4), (2, 17)],
            ),
            (
                "foo in {k1: 0, k2: 1}",
                "foo in { k1: 0, k2: 1 }",
                vec![(1, 8), (1, 21)],
            ),
            (
                "foo => {k1: 0, k2: 1}",
                "foo => { k1: 0, k2: 1 }",
                vec![(1, 8), (1, 21)],
            ),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{before}"
            );
            for o in &found {
                let brace = if before.lines().nth(o.line as usize - 1).unwrap().as_bytes()
                    [o.col as usize - 1]
                    == b'{'
                {
                    "{"
                } else {
                    "}"
                };
                assert_eq!(o.message, format!("Space inside {brace} missing."));
                assert_eq!(o.severity, crate::offense::Severity::Convention);
            }
            assert_correction(&SpaceInsideHashLiteralBraces, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn pinned_empty_braces_and_nested_corrections() {
        for (before, after) in [
            ("h = { }", "h = {}"),
            ("h = {    }", "h = {}"),
            ("h = {\n}", "h = {}"),
            ("h = {\r\n \t}\r\n", "h = {}\r\n"),
            ("{}\n{key: 1}", "{}\n{ key: 1 }"),
            (
                "foo({key: value} => {key: value})",
                "foo({ key: value } => { key: value })",
            ),
            (
                "{{ a: 1 } => { b: { c: 2 }}}",
                "{ { a: 1 } => { b: { c: 2 } } }",
            ),
        ] {
            assert_correction(&SpaceInsideHashLiteralBraces, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
        let found = offenses("h = {\n}");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 6, 2));
        assert_eq!(
            found[0].message,
            "Space inside empty hash literal braces detected."
        );
    }
    #[test]
    fn multiline_token_exceptions_and_literal_delimiters() {
        for source in [
            "{\\\n a: 1 }",
            "{\\\r\n a: 1 }",
            "{\\\n}",
            "{ a: %q\nx\n}",
            "{ a: 1\\\n}",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        for (before, after) in [
            ("{ a: %q x }", "{ a: %q x  }"),
            ("{a: 1,\n b: 2}", "{ a: 1,\n b: 2 }"),
        ] {
            assert_correction(&SpaceInsideHashLiteralBraces, before, after);
        }
    }

    #[test]
    fn empty_hash_newline_corrections_are_withheld_for_line_values() {
        for source in [
            "x = {\n}\np __LINE__\n",
            "x = {\n}\ndef f; __LINE__; end\n",
            "x = <<~H\n  #{{\n}}\nH\n",
            "%W[#{{\n}}]",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&SpaceInsideHashLiteralBraces, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(&SpaceInsideHashLiteralBraces, source, source);
        }
        for source in [
            "x = {\n} # rubocop:disable Style\n",
            "x = {\n} # rustocop:disable-next Style\n1\n",
        ] {
            assert_correction(
                &SpaceInsideHashLiteralBraces,
                source,
                &source.replacen("{\n}", "{}", 1),
            );
        }
        // The highlight ends after the newline, so a directive on the next
        // line disables it, as Parser's `last_line` does.
        for prefix in ["rubocop", "rustocop"] {
            let source =
                format!("x = {{\n}} # {prefix}:disable Layout/SpaceInsideHashLiteralBraces\n");
            assert!(
                crate::pipeline::lint_str(&source)
                    .iter()
                    .all(|o| o.cop != SpaceInsideHashLiteralBraces.name()),
                "{source}"
            );
        }
    }

    #[test]
    fn literal_text_is_preserved_while_interpolation_is_code() {
        for (before, after) in [
            ("\"{a: 1} #{{a: 1}}\"", "\"{a: 1} #{{ a: 1 }}\""),
            ("%W[{a: 1}#{{a: 1}}]", "%W[{a: 1}#{{ a: 1 }}]"),
            ("%I[{a: 1}#{{a: 1}}]", "%I[{a: 1}#{{ a: 1 }}]"),
            ("%r[{a: 1}#{{a: 1}}]", "%r[{a: 1}#{{ a: 1 }}]"),
            ("%x[{a: 1}#{{a: 1}}]", "%x[{a: 1}#{{ a: 1 }}]"),
            (
                "x = <<~H\n  {a: 1} #{{a: 1}}\nH\n",
                "x = <<~H\n  {a: 1} #{{ a: 1 }}\nH\n",
            ),
            ("{a: <<~H}\n  {a: 1}\nH\n", "{ a: <<~H }\n  {a: 1}\nH\n"),
            ("{a: 1}\n__END__\n{a: 1}\n", "{ a: 1 }\n__END__\n{a: 1}\n"),
            ("\u{feff}{é: 1}\r\n", "\u{feff}{ é: 1 }\r\n"),
        ] {
            assert_eq!(offenses(before).len(), 2, "{before}");
            assert_correction(&SpaceInsideHashLiteralBraces, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
        let found = offenses("\u{feff}{é: 1}\r\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 1), (1, 6)]
        );
    }

    #[test]
    fn vertical_tabs_are_whitespace_not_hash_contents() {
        let source = "h = {\u{000b}}";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 6));
        assert_eq!(
            found[0].message,
            "Space inside empty hash literal braces detected."
        );
        assert_correction(&SpaceInsideHashLiteralBraces, source, "h = {}");
        for (before, after) in [
            ("h = {\u{000b}a: 1}", "h = {\u{000b}a: 1 }"),
            ("h = {a: 1\u{000b}}", "h = { a: 1\u{000b}}"),
        ] {
            assert_correction(&SpaceInsideHashLiteralBraces, before, after);
            assert!(offenses(after).is_empty(), "{after:?}");
        }
        for source in ["h = {\u{000b}a: 1\u{000b}}", "x in {\u{000b}a: 1\u{000b}}"] {
            assert!(offenses(source).is_empty(), "{source:?}");
            assert!(
                crate::cops::testing::correction_offenses(&SpaceInsideHashLiteralBraces, source)
                    .is_empty()
            );
        }
    }

    #[test]
    fn pinned_and_synthetic_no_offense_lookalikes() {
        for source in [
            "",
            "{}",
            "{ a: 1, b: 2 }",
            "{ a => 1 }",
            "x(a: b.c)",
            "f(get: \"#{x}\")",
            "Hash[{ x: 1 } => [1]]",
            "{ key: \"{\" }",
            "{\n a: 1,\n b: 2,\n}",
            "{# Comment\n a: 1\n}",
            "{ \n# Comment\n}",
            "each {puts}",
            "-> {x}",
            "\"#{x}\"",
            "foo in k1: 0, k2: 1",
            "x in A[a: 1]",
            "# {a: 1}\n",
            "'{a: 1}'",
            "/{a: 1}/",
            "%q{a: 1}",
            "%w[{a: 1}]",
            "x = <<~H\n{a: 1}\nH\n",
            "__END__\n{a: 1}\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }
}
