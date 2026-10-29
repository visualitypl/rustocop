//! `Layout/SpaceInsideParens` — fixed `no_space` inside ordinary parentheses.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceInsideParens;

impl Cop for SpaceInsideParens {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideParens"
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
            let Some(range) = &token.inside_parens else {
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
                message: "Space inside parentheses detected.".into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/layout/space_inside_parens_spec.rb`
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
            .filter(|o| o.cop == "Layout/SpaceInsideParens")
            .collect()
    }

    #[test]
    fn pinned_heredoc_opener_spacing() {
        let before = "f( <<~HEREDOC )\n  This is my text\nHEREDOC\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 3), (1, 14)]
        );
        assert_correction(
            &SpaceInsideParens,
            before,
            "f(<<~HEREDOC)\n  This is my text\nHEREDOC\n",
        );
    }

    #[test]
    fn pinned_no_offense_lookalikes_and_newline_comment_exceptions() {
        for source in [
            "",
            "list.inject(Tms.new) { |sum, (label, item)|\n}",
            "split(\"\\n\")",
            "f(\n  1)",
            "f( # Comment\n  1)",
            "f( \t# Comment\r\n  1\r\n )",
            "f( \n  1\n )",
            "f( \\\n  1\n )",
            "f( \\\r\n  1\r\n )",
            "f(\n )",
            "# f( x )\n",
            "=begin\nf( x )\n=end\n",
            "__END__\nf( x )\n",
            "'f( x )'",
            "\"f( x )\"",
            "/( x )/",
            "%q( x )",
            "%Q( x )",
            "%w( x )",
            "%W( x )",
            "%i( x )",
            "%I( x )",
            "%r( x )",
            "%x( x )",
            "%s( x )",
            ":\"( x )\"",
            "?(",
            "?)",
            "x = <<~H\nf( x )\nH\n",
            "x = <<'H'\nf( x )\nH\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn pinned_argument_parenthesis_token_exemption() {
        for source in [
            "f ( x)",
            "obj.f ( x)",
            "obj&.f ( x)",
            "f \\\n( x)",
            "f ( x), y",
            "def f; yield ( x); end",
            "def f; super ( x); end",
            "not ( x)",
            "defined? ( x)",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        // tLPAREN_ARG exempts only the opening side, not a following tRPAREN.
        for (before, after, col) in [
            ("f ( x )", "f ( x)", 6),
            ("f ( )", "f ()", 4),
            ("f ( ( x))", "f ( (x))", 6),
            ("return ( x)", "return (x)", 9),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(1, col)]
            );
            assert_correction(&SpaceInsideParens, before, after);
        }
    }

    #[test]
    fn pinned_multiline_string_token_start_line_exemption() {
        for source in [
            "f(\"a\\\nb\" )",
            "f(\"a\\\r\nb\" )",
            "f(\"a\\\nb\\\nc\" )",
            "f(\"a\\\n\" )",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        // Only simplified tSTRING spans are exempt, not separate closing tokens
        // or a later token on the closing parenthesis's line.
        for (before, after, col) in [
            ("f(\"a\nb\" )", "f(\"a\nb\")", 3),
            ("f('a\\\nb' )", "f('a\\\nb')", 3),
            ("f(%Q{a\\\nb} )", "f(%Q{a\\\nb})", 3),
            ("f(\"a#{1}\\\nb\" )", "f(\"a#{1}\\\nb\")", 3),
            ("f(\"a\\\nb\" \"c\" )", "f(\"a\\\nb\" \"c\")", 7),
            ("f(\"a\\\nb\", x )", "f(\"a\\\nb\", x)", 6),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(2, col)]
            );
            assert_correction(&SpaceInsideParens, before, after);
        }
        assert_correction(&SpaceInsideParens, "f( \"a\\\nb\" )", "f(\"a\\\nb\" )");
    }

    #[test]
    fn calls_groups_definitions_lambdas_and_nested_pairs() {
        for (before, after, columns) in [
            ("f(  )", "f()", vec![3]),
            ("obj&.f( 1 )", "obj&.f(1)", vec![8, 10]),
            (
                "x = ( ( a + b ) * c )",
                "x = ((a + b) * c)",
                vec![6, 8, 14, 20],
            ),
            (
                "def f( a, b = ( 1 ) ); end",
                "def f(a, b = (1)); end",
                vec![7, 16, 18, 20],
            ),
            ("def self.f( ); end", "def self.f(); end", vec![12]),
            ("->( a ) { a }", "->(a) { a }", vec![4, 6]),
            ("->( ) { 1 }", "->() { 1 }", vec![4]),
            ("f { |( a, b )| a }", "f { |(a, b)| a }", vec![7, 12]),
            (
                "def f; super( 1 ); yield( 2 ); end",
                "def f; super(1); yield(2); end",
                vec![14, 16, 26, 28],
            ),
            ("defined?( x )", "defined?(x)", vec![10, 12]),
            ("( ; x; )", "(; x;)", vec![2, 7]),
        ] {
            let found = offenses(before);
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                columns,
                "{before}"
            );
            assert_correction(&SpaceInsideParens, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }

    #[test]
    fn interpolation_is_code_but_literal_parentheses_are_not() {
        for (before, after) in [
            ("\"( x ) #{f( 1 )}\"", "\"( x ) #{f(1)}\""),
            ("%W(( x )#{f( 1 )})", "%W(( x )#{f(1)})"),
            ("%I(( x )#{f( 1 )})", "%I(( x )#{f(1)})"),
            ("%r(( x )#{f( 1 )})", "%r(( x )#{f(1)})"),
            ("%x(( x )#{f( 1 )})", "%x(( x )#{f(1)})"),
            (
                "x = <<~H\n  ( x )#{f( 1 )}\nH\n",
                "x = <<~H\n  ( x )#{f(1)}\nH\n",
            ),
            (
                "x = <<~OUT\n#{<<~IN}\n  #{f( 1 )}\nIN\nOUT\n",
                "x = <<~OUT\n#{<<~IN}\n  #{f(1)}\nIN\nOUT\n",
            ),
            (
                "f( <<~A, <<~B )\n( x )\nA\n( x )\nB\n",
                "f(<<~A, <<~B)\n( x )\nA\n( x )\nB\n",
            ),
        ] {
            assert_eq!(offenses(before).len(), 2, "{before:?}");
            assert_correction(&SpaceInsideParens, before, after);
            assert!(offenses(after).is_empty());
        }
    }

    #[test]
    fn preserves_token_delimiters_semicolons_crlf_bom_and_utf8_positions() {
        for (before, after) in [
            ("f( %q x  )", "f(%q x )"),
            ("f( %Q x  )", "f(%Q x )"),
            ("f(\t1\t)", "f(1)"),
            ("f(\x0b1\x0c)", "f(1)"),
            ("x = ( 1; 2 )\np __LINE__\n", "x = (1; 2)\np __LINE__\n"),
            (
                "x = ( 1; # Keep this comment\n  2 )\n",
                "x = (1; # Keep this comment\n  2)\n",
            ),
            ("\u{feff}f( \"é\" )\r\n", "\u{feff}f(\"é\")\r\n"),
        ] {
            assert_correction(&SpaceInsideParens, before, after);
        }
        let found = offenses("\u{feff}f( \"é\" )\r\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 3), (1, 7)]
        );
    }

    #[test]
    fn eof_markers_and_data_payloads_are_not_code() {
        for marker in ['\0', '\u{4}', '\u{1a}'] {
            let before = format!("f( \"{marker}\" )\n{marker}f( x )\n");
            let after = format!("f(\"{marker}\")\n{marker}f( x )\n");
            assert_eq!(offenses(&before).len(), 2);
            assert_correction(&SpaceInsideParens, &before, &after);
        }
        assert_correction(
            &SpaceInsideParens,
            "f( 1 )\n__END__\nf( x )\n",
            "f(1)\n__END__\nf( x )\n",
        );
    }

    #[test]
    fn character_literal_parentheses_are_not_empty_pairs() {
        for (before, after) in [("f(?( )", "f(?()"), ("f( ?) )", "f(?))")] {
            assert_correction(&SpaceInsideParens, before, after);
        }
    }

    #[test]
    fn pinned_spaces_inside_nonempty_and_empty_parens() {
        let before = "f( 3)\ng = (a + 3 )\nf( )\n";
        let found = offenses(before);
        assert_eq!(found.len(), 3);
        for (offense, position) in found.iter().zip([(1, 3), (2, 11), (3, 3)]) {
            assert_eq!((offense.line, offense.col), position);
            assert_eq!(offense.end_line, offense.line);
            assert_eq!(offense.message, "Space inside parentheses detected.");
            assert_eq!(offense.severity, crate::offense::Severity::Convention);
        }
        assert_correction(&SpaceInsideParens, before, "f(3)\ng = (a + 3)\nf()\n");
    }
}
