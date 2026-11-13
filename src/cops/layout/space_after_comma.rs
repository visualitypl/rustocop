//! `Layout/SpaceAfterComma` — pinned default separator spacing.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAfterComma;

impl Cop for SpaceAfterComma {
    fn name(&self) -> &'static str {
        "Layout/SpaceAfterComma"
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
            if file.bytes[token.offset] != b',' || !token.missing_after {
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
                message: "Space missing after comma.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_after_comma_spec.rb`
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
            .filter(|o| o.cop == "Layout/SpaceAfterComma")
            .collect()
    }

    #[test]
    fn pinned_missing_spaces_and_corrections() {
        for (before, after, col) in [
            ("each { |s,t| }", "each { |s, t| }", 10),
            ("formats[0,1]", "formats[0, 1]", 10),
            ("a(1,2)", "a(1, 2)", 4),
            ("{ foo:bar,}", "{ foo:bar, }", 10),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].end_line, found[0].col),
                (1, 1, col)
            );
            assert_eq!(found[0].message, "Space missing after comma.");
            assert_eq!(found[0].severity, crate::offense::Severity::Convention);
            assert_correction(&SpaceAfterComma, before, after);
        }
    }

    #[test]
    fn calls_definitions_assignments_and_collections() {
        for (before, after) in [
            (
                "def f(a,b = 1,*c,d:,**e); end",
                "def f(a, b = 1, *c, d:, **e); end",
            ),
            ("a,b = 1,2", "a, b = 1, 2"),
            ("[1,2,]", "[1, 2,]"),
            ("{a: 1,b: 2,}", "{a: 1, b: 2, }"),
            ("f 1,2", "f 1, 2"),
            ("f(<<~H,2)\ntext,;\nH\n", "f(<<~H, 2)\ntext,;\nH\n"),
        ] {
            assert_correction(&SpaceAfterComma, before, after);
        }
    }

    #[test]
    fn literals_comments_globals_and_raw_data_are_not_commas() {
        for source in [
            "",
            "# f(1,2)\n",
            "=begin\nf(1,2)\n=end\n",
            "'a,b'",
            "\"a,b\"",
            "/a,b/",
            "%r,a\\,b,",
            "%q,a\\,b,",
            "%Q,a\\,b,",
            "%w,a\\,b,",
            "%W,a\\,b,",
            "%i,a\\,b,",
            "%I,a\\,b,",
            "%x,a\\,b,",
            "%s,a\\,b,",
            ":\"a,b\"",
            "?,",
            "p $,",
            "$, = 1",
            "$, ||= 1",
            "$, &&= 1",
            "$, += 1",
            "a = <<~H\na,b\nH\n",
            "a = <<'H'\na,b\nH\n",
            "__END__\nf(1,2)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn interpolation_is_code_even_in_percent_arrays_and_heredocs() {
        for (before, after) in [
            ("\"a,b#{f(1,2)}\"", "\"a,b#{f(1, 2)}\""),
            ("%W,a\\,b#{f(1,2)},", "%W,a\\,b#{f(1, 2)},"),
            ("%I,a\\,b#{f(1,2)},", "%I,a\\,b#{f(1, 2)},"),
            ("/a,b#{f(1,2)}/", "/a,b#{f(1, 2)}/"),
            (
                "x = <<~H\n  a,b#{f(1,2)}\nH\n",
                "x = <<~H\n  a,b#{f(1, 2)}\nH\n",
            ),
            (
                "x = <<~A,<<~B\na,b\nA\nc,d\nB\n",
                "x = <<~A, <<~B\na,b\nA\nc,d\nB\n",
            ),
        ] {
            assert_eq!(offenses(before).len(), 1, "{before:?}");
            assert_correction(&SpaceAfterComma, before, after);
        }
    }

    #[test]
    fn ruby_eof_markers_are_not_code_but_literal_markers_are() {
        for marker in ['\0', '\u{4}', '\u{1a}'] {
            assert!(offenses(&format!("p 1\n{marker}f(1 ,2);g\n")).is_empty());
            let before = format!("f(\"{marker}\",2)\n{marker}f(1 ,2);g\n");
            let after = format!("f(\"{marker}\", 2)\n{marker}f(1 ,2);g\n");
            assert_eq!(offenses(&before).len(), 1);
            assert_correction(&SpaceAfterComma, &before, &after);
            for prefix in [
                format!("'{marker}'\n"),
                format!("/{marker}/\n"),
                format!("%W(a{marker})\n"),
                format!("?{marker}\n"),
                format!("# {marker}\n"),
                format!("=begin\n{marker}\n=end\n"),
                format!("x = <<~H\n{marker}\nH\n"),
                format!("x = <<~OUT\n#{{<<~IN}}\n{marker}\nIN\nOUT\n"),
            ] {
                let before = format!("{prefix}f(1,2)\n{marker}f(1,2)\n");
                let after = format!("{prefix}f(1, 2)\n{marker}f(1,2)\n");
                assert_eq!(offenses(&before).len(), 1, "{before:?}");
                assert_correction(&SpaceAfterComma, &before, &after);
            }
        }
    }

    #[test]
    fn nested_heredoc_interpolation_is_code() {
        let before = "x = <<~OUT\n#{<<~IN}\n  #{f(1 ,2);g}\nIN\nOUT\n";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 9));
        assert_eq!(found[0].message, "Space missing after comma.");
        assert_correction(
            &SpaceAfterComma,
            before,
            "x = <<~OUT\n#{<<~IN}\n  #{f(1 , 2);g}\nIN\nOUT\n",
        );
    }

    #[test]
    fn adjacent_comments_need_a_space_without_changing_comment_bytes() {
        for (before, after) in [
            ("[1,# hi\n2]\n", "[1, # hi\n2]\n"),
            ("[1,# hi\r\n2]\r\n", "[1, # hi\r\n2]\r\n"),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 3));
            assert_eq!(found[0].message, "Space missing after comma.");
            assert_correction(&SpaceAfterComma, before, after);
        }
    }

    #[test]
    fn preserves_newlines_trailing_commas_and_comments() {
        for source in [
            "[1,\n2,\n]",
            "[1,\r\n2,\r\n]",
            "f(1,\\\n2)",
            "f(1,\\\r\n2)",
            "[1, # note\n2]",
            "f { |a,;b| }",
            "f(1,\t2)",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn multibyte_bom_and_crlf_positions_and_corrections() {
        let before = "\u{feff}f(\"é\",2)\r\nf(1,2)\r\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 6), (2, 4)]
        );
        assert_correction(
            &SpaceAfterComma,
            before,
            "\u{feff}f(\"é\", 2)\r\nf(1, 2)\r\n",
        );
    }

    #[test]
    fn pinned_trailing_commas_and_existing_spaces() {
        for source in ["each { |s, t,| }", "formats[0,]", "{ foo:bar, }", "a(1, 2)"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
