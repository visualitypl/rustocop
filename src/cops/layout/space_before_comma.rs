//! `Layout/SpaceBeforeComma` — no same-line space before a comma.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceBeforeComma;

impl Cop for SpaceBeforeComma {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeComma"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
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
            if file.bytes[token.offset] != b',' {
                continue;
            }
            let Some(range) = &token.before else {
                continue;
            };
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                correction: cx.corrections_requested().then(|| Correction {
                    range: range.clone(),
                    replacement: String::new(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: "Space found before comma.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_before_comma_spec.rb`
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
            .filter(|o| o.cop == "Layout/SpaceBeforeComma")
            .collect()
    }

    #[test]
    fn pinned_spaces_and_corrections() {
        for (before, after, col) in [
            ("each { |s , t| }", "each { |s, t| }", 10),
            ("formats[0 , 1]", "formats[0, 1]", 10),
            ("a(1 , 2)", "a(1, 2)", 4),
            (
                "a(<<~STR , 2)\n  text\nSTR\n",
                "a(<<~STR, 2)\n  text\nSTR\n",
                9,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].end_line, found[0].col),
                (1, 1, col)
            );
            assert_eq!(found[0].message, "Space found before comma.");
            assert_eq!(found[0].severity, crate::offense::Severity::Convention);
            assert_correction(&SpaceBeforeComma, before, after);
        }
        assert!(offenses("a(1, 2)").is_empty());
    }

    #[test]
    fn definitions_assignments_collections_and_literal_token_boundaries() {
        for (before, after) in [
            ("def f(a ,b = 1 ,*c); end", "def f(a,b = 1,*c); end"),
            ("a ,b = 1 ,2", "a,b = 1,2"),
            ("[1 ,2 ,]", "[1,2,]"),
            ("{a: 1 ,b: 2 ,}", "{a: 1,b: 2,}"),
            ("f(1\t\t, 2)", "f(1, 2)"),
            ("f(%q x  , 2)", "f(%q x , 2)"),
            ("f(%q,x, , 2)", "f(%q,x,, 2)"),
            ("f(?\\s  , 2)", "f(?\\s, 2)"),
            (
                "f(<<~A , <<~B)\na ,b\nA\nc ,d\nB\n",
                "f(<<~A, <<~B)\na ,b\nA\nc ,d\nB\n",
            ),
        ] {
            assert_correction(&SpaceBeforeComma, before, after);
        }
    }

    #[test]
    fn ignores_literals_comments_data_and_leading_comma_indentation() {
        for source in [
            "",
            "# f(1 , 2)\n",
            "'a ,b'",
            "/a ,b/",
            "%w(a ,b)",
            "%i(a ,b)",
            "%q,a \\,b,",
            "%x(a ,b)",
            ":\"a ,b\"",
            "p $,",
            "$, = 1",
            "x = <<~H\na ,b\nH\n",
            "__END__\nf(1 ,2)\n",
            // Continuation newlines before commas must not be joined.
            "f(1 \\\n  ,2)",
            "f(1 \\\r\n  ,2)",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn interpolation_code_and_multibyte_crlf() {
        for (before, after) in [
            ("\"text ,#{f(1 ,2)}\"", "\"text ,#{f(1,2)}\""),
            ("%W(text ,#{f(1 ,2)})", "%W(text ,#{f(1,2)})"),
            ("x = <<~H\n  #{f(1 ,2)}\nH\n", "x = <<~H\n  #{f(1,2)}\nH\n"),
            ("\u{feff}f(\"é\" \t, 2)\r\n", "\u{feff}f(\"é\", 2)\r\n"),
        ] {
            assert_eq!(offenses(before).len(), 1, "{before:?}");
            assert_correction(&SpaceBeforeComma, before, after);
        }
        let found = offenses("\u{feff}f(\"é\" \t, 2)\r\n");
        assert_eq!((found[0].line, found[0].col), (1, 6));
    }

    #[test]
    fn ruby_eof_markers_are_not_code_but_literal_markers_are() {
        for marker in ['\0', '\u{4}', '\u{1a}'] {
            assert!(offenses(&format!("p 1\n{marker}f(1 ,2);g\n")).is_empty());
            let before = format!("f(\"{marker}\" ,2)\n{marker}f(1 ,2);g\n");
            let after = format!("f(\"{marker}\",2)\n{marker}f(1 ,2);g\n");
            assert_eq!(offenses(&before).len(), 1);
            assert_correction(&SpaceBeforeComma, &before, &after);
        }
    }

    #[test]
    fn nested_heredoc_interpolation_is_code() {
        let before = "x = <<~OUT\n#{<<~IN}\n  #{f(1 ,2);g}\nIN\nOUT\n";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 8));
        assert_eq!(found[0].message, "Space found before comma.");
        assert_correction(
            &SpaceBeforeComma,
            before,
            "x = <<~OUT\n#{<<~IN}\n  #{f(1,2);g}\nIN\nOUT\n",
        );
    }

    #[test]
    fn pinned_multiple_spaces() {
        let before = "each { |s  , t| a(1  , formats[0  , 1])}";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 10), (1, 20), (1, 33)]
        );
        assert_correction(
            &SpaceBeforeComma,
            before,
            "each { |s, t| a(1, formats[0, 1])}",
        );
    }
}
