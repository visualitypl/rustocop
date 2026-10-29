//! `Layout/SpaceAfterSemicolon` — pinned default separator spacing.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAfterSemicolon;

impl Cop for SpaceAfterSemicolon {
    fn name(&self) -> &'static str {
        "Layout/SpaceAfterSemicolon"
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
        if node.as_program_node().is_none() {
            return;
        }
        for token in file.punctuation(node) {
            if file.bytes[token.offset] != b';' || !token.missing_after {
                continue;
            }
            let (line, col) = file.line_col(token.offset);
            out.push(Offense {
                correction: cx.corrections_requested().then(|| Correction {
                    range: token.offset + 1..token.offset + 1,
                    replacement: " ".into(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: "Space missing after semicolon.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_after_semicolon_spec.rb`
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
            .filter(|o| o.cop == "Layout/SpaceAfterSemicolon")
            .collect()
    }

    #[test]
    fn pinned_missing_spaces_and_corrections() {
        for (before, after, col) in [
            ("x = 1;y = 2", "x = 1; y = 2", 6),
            ("test { ;}", "test { ; }", 8),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].end_line, found[0].col),
                (1, 1, col)
            );
            assert_eq!(found[0].message, "Space missing after semicolon.");
            assert_eq!(found[0].severity, crate::offense::Severity::Convention);
            assert_correction(&SpaceAfterSemicolon, before, after);
        }
    }

    #[test]
    fn inline_statements_definitions_and_block_locals() {
        for (before, after) in [
            ("def f;nil;end", "def f; nil; end"),
            ("a = 1;b = 2;a + b", "a = 1; b = 2; a + b"),
            ("f { |a;b| b }", "f { |a; b| b }"),
            (
                "x = <<~H;p x\ntext;literal\nH\n",
                "x = <<~H; p x\ntext;literal\nH\n",
            ),
            ("\u{feff}p \"é\";p 2\r\n", "\u{feff}p \"é\"; p 2\r\n"),
        ] {
            assert_correction(&SpaceAfterSemicolon, before, after);
        }
        let found = offenses("\u{feff}p \"é\";p 2\r\n");
        assert_eq!((found[0].line, found[0].col), (1, 6));
    }

    #[test]
    fn literals_globals_comments_data_and_line_boundaries() {
        for source in [
            "",
            "# x;y\n",
            "=begin\nx;y\n=end\n",
            "'a;b'",
            "\"a;b\"",
            "/a;b/",
            "%q;a\\;b;",
            "%Q;a\\;b;",
            "%w;a\\;b;",
            "%W;a\\;b;",
            "%i;a\\;b;",
            "%I;a\\;b;",
            "%r;a\\;b;",
            "%x;a\\;b;",
            "%s;a\\;b;",
            ":\"a;b\"",
            "?;",
            "p $;",
            "$; = 1",
            "$; ||= 1",
            "$; &&= 1",
            "$; += 1",
            "$;, x = []",
            "x = <<~H\na;b\nH\n",
            "x = <<'H'\na;b\nH\n",
            "__END__\nx;y\n",
            "x;\ny",
            "x;\r\ny",
            "x;\\\ny",
            "x;\\\r\ny",
            "f { |a; b| }",
            "(nil;)",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn interpolation_code_including_percent_arrays() {
        for (before, after) in [
            ("\"a;b#{x;y}\"", "\"a;b#{x; y}\""),
            ("%W;a\\;b#{x;y};", "%W;a\\;b#{x; y};"),
            ("%I;a\\;b#{x;y};", "%I;a\\;b#{x; y};"),
            ("/a;b#{x;y}/", "/a;b#{x; y}/"),
            ("x = <<~H\n  a;b#{x;y}\nH\n", "x = <<~H\n  a;b#{x; y}\nH\n"),
        ] {
            assert_eq!(offenses(before).len(), 1, "{before:?}");
            assert_correction(&SpaceAfterSemicolon, before, after);
        }
    }

    #[test]
    fn ruby_eof_markers_are_not_code_but_literal_markers_are() {
        for marker in ['\0', '\u{4}', '\u{1a}'] {
            assert!(offenses(&format!("p 1\n{marker}f(1 ,2);g\n")).is_empty());
            let before = format!("p \"{marker}\";g\n{marker}f(1 ,2);g\n");
            let after = format!("p \"{marker}\"; g\n{marker}f(1 ,2);g\n");
            assert_eq!(offenses(&before).len(), 1);
            assert_correction(&SpaceAfterSemicolon, &before, &after);
        }
    }

    #[test]
    fn nested_heredoc_interpolation_is_code() {
        let before = "x = <<~OUT\n#{<<~IN}\n  #{f(1 ,2);g}\nIN\nOUT\n";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 12));
        assert_eq!(found[0].message, "Space missing after semicolon.");
        assert_correction(
            &SpaceAfterSemicolon,
            before,
            "x = <<~OUT\n#{<<~IN}\n  #{f(1 ,2); g}\nIN\nOUT\n",
        );
    }

    #[test]
    fn adjacent_comments_need_a_space_without_changing_comment_bytes() {
        for (before, after) in [
            ("x;# hi\ny\n", "x; # hi\ny\n"),
            ("x;# hi\r\ny\r\n", "x; # hi\r\ny\r\n"),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 2));
            assert_eq!(found[0].message, "Space missing after semicolon.");
            assert_correction(&SpaceAfterSemicolon, before, after);
        }
    }

    #[test]
    fn pinned_exceptions() {
        for source in ["x = 1;", "foo;; bar", "test { ; }", "\"#{ ;}\""] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
