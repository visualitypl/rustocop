//! `Layout/SpaceInsideBlockBraces` — spaces in brace blocks and lambdas.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

use super::brace_spacing::is_ruby_whitespace;

pub struct SpaceInsideBlockBraces;
impl Cop for SpaceInsideBlockBraces {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideBlockBraces"
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
        let (left, right, body) = if let Some(block) = node.as_block_node() {
            (block.opening_loc(), block.closing_loc(), block.body())
        } else if let Some(lambda) = node.as_lambda_node() {
            (lambda.opening_loc(), lambda.closing_loc(), lambda.body())
        } else {
            return;
        };
        if left.as_slice() != b"{" || right.as_slice() != b"}" {
            return;
        }
        let (left, right) = (left.start_offset(), right.start_offset());
        let single_line = file.line_of(left) == file.line_of(right);
        // The pinned cop leaves multiline blocks with no body alone, even
        // if they contain parameters or comments. Empty hashes differ.
        if body.is_none() && !single_line {
            return;
        }
        let inside = &file.bytes[left + 1..right];
        if inside.is_empty() {
            return;
        }
        if inside.iter().copied().all(is_ruby_whitespace) {
            super::brace_spacing::report(
                self,
                node,
                cx,
                file,
                left + 1..right,
                left + 1..right,
                "",
                "Space inside empty braces detected.",
                out,
            );
            return;
        }
        if !is_ruby_whitespace(inside[0]) {
            let pipe = inside[0] == b'|';
            let highlight = if pipe {
                left..left + 2
            } else {
                left + 1..left + 2
            };
            let message = if pipe {
                "Space between { and | missing."
            } else {
                "Space missing inside {."
            };
            super::brace_spacing::report(
                self,
                node,
                cx,
                file,
                highlight,
                left + 1..left + 1,
                " ",
                message,
                out,
            );
        }
        // With the fixed `space` style, a multiline closing brace is exempt,
        // even when it immediately follows the body on that line.
        if single_line && !is_ruby_whitespace(*inside.last().unwrap()) {
            super::brace_spacing::report(
                self,
                node,
                cx,
                file,
                right..right + 1,
                right..right,
                " ",
                "Space missing inside }.",
                out,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_inside_block_braces_spec.rb`
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
            .filter(|o| o.cop == SpaceInsideBlockBraces.name())
            .collect()
    }
    #[test]
    fn pinned_exact_offenses_and_corrections() {
        for (before, after, columns, messages) in [
            (
                "each {puts }",
                "each { puts }",
                vec![7],
                vec!["Space missing inside {."],
            ),
            (
                "each { puts}",
                "each { puts }",
                vec![12],
                vec!["Space missing inside }."],
            ),
            (
                "each {puts}",
                "each { puts }",
                vec![7, 11],
                vec!["Space missing inside {.", "Space missing inside }."],
            ),
            (
                "each { }",
                "each {}",
                vec![7],
                vec!["Space inside empty braces detected."],
            ),
            (
                "each {|x| puts }",
                "each { |x| puts }",
                vec![6],
                vec!["Space between { and | missing."],
            ),
            (
                "[1, 2, 3].each {_1 * 2}",
                "[1, 2, 3].each { _1 * 2 }",
                vec![17, 23],
                vec!["Space missing inside {.", "Space missing inside }."],
            ),
            (
                "[1, 2, 3].each {it * 2}",
                "[1, 2, 3].each { it * 2 }",
                vec![17, 23],
                vec!["Space missing inside {.", "Space missing inside }."],
            ),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                columns,
                "{before}"
            );
            for (o, message) in found.iter().zip(messages) {
                assert_eq!((o.line, o.end_line), (1, 1));
                assert_eq!(o.message, message);
                assert_eq!(o.severity, crate::offense::Severity::Convention);
            }
            assert_correction(&SpaceInsideBlockBraces, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn pinned_multiline_parameters_lambdas_and_nested_blocks() {
        for (before, after) in [
            ("each {|x|\n  puts\n}", "each { |x|\n  puts\n}"),
            ("each {|x| puts}", "each { |x| puts }"),
            ("->(x) {x}", "->(x) { x }"),
            ("-> { }", "-> {}"),
            ("foo {bar {baz}}", "foo { bar { baz } }"),
            ("each {;}", "each { ; }"),
            ("each {||}", "each { || }"),
            ("each {|;x|x}", "each { |;x|x }"),
            ("each {puts\n}", "each { puts\n}"),
            ("each {# Comment\n  puts\n}", "each { # Comment\n  puts\n}"),
            ("each { \t }\r\n", "each {}\r\n"),
        ] {
            assert_correction(&SpaceInsideBlockBraces, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
        let found = offenses("each {|x|\n  puts\n}");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 6, 1));
    }
    #[test]
    fn literal_text_interpolation_and_multiline_calls() {
        for (before, after) in [
            (
                "\"each {x} #{[1].map {it}}\"",
                "\"each {x} #{[1].map { it }}\"",
            ),
            ("%W[each{x}#{[1].map {_1}}]", "%W[each{x}#{[1].map { _1 }}]"),
            ("%I[each{x}#{[1].map {_1}}]", "%I[each{x}#{[1].map { _1 }}]"),
            ("%r[each{x}#{[1].map {_1}}]", "%r[each{x}#{[1].map { _1 }}]"),
            ("%x[each{x}#{[1].map {_1}}]", "%x[each{x}#{[1].map { _1 }}]"),
            (
                "x = <<~H\n  each{x} #{[1].map {_1}}\nH\n",
                "x = <<~H\n  each{x} #{[1].map { _1 }}\nH\n",
            ),
            (
                "f {puts <<~H}\n  each{x}\nH\n",
                "f { puts <<~H }\n  each{x}\nH\n",
            ),
            (
                "each {x}\n__END__\neach {x}\n",
                "each { x }\n__END__\neach {x}\n",
            ),
            ("\u{feff}f {é}\r\n", "\u{feff}f { é }\r\n"),
            ("foo(\n 1) {x}", "foo(\n 1) { x }"),
            ("->(x =\n 1) {x}", "->(x =\n 1) { x }"),
            ("f {\\\n x\n}", "f { \\\n x\n}"),
        ] {
            assert_correction(&SpaceInsideBlockBraces, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
        let found = offenses("\u{feff}f {é}\r\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 4), (1, 5)]
        );
        // Spacing after the closing parameter pipe belongs to a different cop.
        assert!(offenses("f { |x|x }").is_empty());
    }

    #[test]
    fn single_line_braces_after_multiline_calls_and_lambda_parameters() {
        for (before, after, positions, messages) in [
            (
                "f(\n 1) {x}",
                "f(\n 1) { x }",
                vec![(2, 6), (2, 7)],
                vec!["Space missing inside {.", "Space missing inside }."],
            ),
            (
                "f(\n 1) { }",
                "f(\n 1) {}",
                vec![(2, 6)],
                vec!["Space inside empty braces detected."],
            ),
            (
                "->(x =\n 1) { }",
                "->(x =\n 1) {}",
                vec![(2, 6)],
                vec!["Space inside empty braces detected."],
            ),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{before:?}"
            );
            assert_eq!(
                found.iter().map(|o| o.message.as_str()).collect::<Vec<_>>(),
                messages
            );
            assert_correction(&SpaceInsideBlockBraces, before, after);
            assert!(offenses(after).is_empty(), "{after:?}");
        }
    }

    #[test]
    fn vertical_tabs_are_whitespace_not_block_contents() {
        let source = "f {\u{000b}}";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 4));
        assert_eq!(found[0].message, "Space inside empty braces detected.");
        assert_correction(&SpaceInsideBlockBraces, source, "f {}");
        for (before, after) in [
            ("f {\u{000b}x}", "f {\u{000b}x }"),
            ("f {x\u{000b}}", "f { x\u{000b}}"),
        ] {
            assert_correction(&SpaceInsideBlockBraces, before, after);
            assert!(offenses(after).is_empty(), "{after:?}");
        }
        for source in [
            "f {\u{000b}x\u{000b}}",
            "f {\u{000b}|x| x\u{000b}}",
            "f {\n\u{000b}}",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
            assert!(
                crate::cops::testing::correction_offenses(&SpaceInsideBlockBraces, source)
                    .is_empty()
            );
        }
    }

    #[test]
    fn pinned_and_synthetic_no_offense_lookalikes() {
        for source in [
            "",
            "each {}",
            "each { puts }",
            "each{ puts }",
            "each { |x| puts }",
            "->(x) { x }",
            "each { \"f\" }",
            "each { %(\n) }",
            "each { # Comment\n}",
            "  each {\n  }",
            "each {\n  puts\n}",
            "each { |x|\n  puts\n}",
            "each {|x|\n}",
            "-> {\n}",
            "each { |x|\n x}",
            "each {\n x}",
            "each do\n puts\nend",
            "-> do\n puts\nend",
            "{key: 1}",
            "\"#{x}\"",
            "f(get: \"#{x}\")",
            "'each {x}'",
            "%q{each {x}}",
            "/each {x}/",
            "# each {x}\n",
            "x = <<~H\neach {x}\nH\n",
            "__END__\neach {x}\n",
            "BEGIN {x}",
            "END {x}",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }
}
