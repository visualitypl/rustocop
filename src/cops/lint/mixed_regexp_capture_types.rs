//! `Lint/MixedRegexpCaptureTypes` — named and numbered parsed capture groups.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MixedRegexpCaptureTypes;
impl Cop for MixedRegexpCaptureTypes {
    fn name(&self) -> &'static str {
        "Lint/MixedRegexpCaptureTypes"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // RegexpNode#interpolation? tests begin children, including constant-
        // only/empty #{...}. Shorthand #@ivar/#$gvar children are not begin:
        // their raw-source quirk is preserved by the shared scanner's View.
        let interpolated = if let Some(n) = node.as_interpolated_regular_expression_node() {
            n.parts()
                .iter()
                .any(|p| p.as_embedded_statements_node().is_some())
        } else if let Some(n) = node.as_interpolated_match_last_line_node() {
            n.parts()
                .iter()
                .any(|p| p.as_embedded_statements_node().is_some())
        } else {
            false
        };
        if interpolated {
            return;
        }
        let Some(regexp) = crate::cops::style::regexp_literal::Regexp::new(node) else {
            return;
        };
        if !super::duplicate_regexp_character_class_element::mixed_captures(&regexp, file) {
            return;
        }
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not mix named captures and numbered captures in a Regexp literal.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/mixed_regexp_capture_types_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/MixedRegexpCaptureTypes")
            .collect()
    }
    #[test]
    fn pinned_mixed_capture() {
        let found = offenses("nil\n  /(?<foo>bar)(baz)/\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Do not mix named captures and numbered captures in a Regexp literal."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn inline_options_follow_pinned_capture_tree_not_runtime_group_scopes() {
        for source in [
            "/((?x))#(?<n>a)/",
            "/(?<n>(?x))#(b)/",
            "/(?=(?x))#(b)(?<n>a)/",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("/(?<n>a)(?-x:#(b))/x").len(), 1);
    }
    #[test]
    fn negated_charset_options_have_no_successful_capture_tree() {
        for options in [
            "-a", "-d", "-u", "a-u", "u-a", "d-u", "i-ma", "x--u", "au-imd",
        ] {
            for pattern in [
                format!("(?<n>a)(b)(?{options})c"),
                format!("(?<n>a)(b)(?{options}:c)"),
                format!("(?{options})(?<n>a)(b)"),
                format!("(?{options}:(?<n>a)(b))"),
            ] {
                for suffix in ["", "x"] {
                    let source = format!("/{pattern}/{suffix}");
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
        }
    }
    #[test]
    fn charset_enabling_and_imx_negation_preserve_mixed_capture_findings() {
        for options in [
            "a", "d", "u", "au", "imxadu", "a-imx", "d-i", "u-m", "adu-x", "i-i", "im-x", "-imx",
            "--x", "i--x", "-x-i", "x--", "-",
        ] {
            for pattern in [
                format!("(?<n>a)(b)(?{options})c"),
                format!("(?<n>a)(b)(?{options}:c)"),
                format!("(?{options})(?<n>a)(b)"),
                format!("(?{options}:(?<n>a)(b))"),
            ] {
                for suffix in ["", "x"] {
                    let source = format!("nil\n  /{pattern}/{suffix}");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (2, 3));
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn shorthand_children_and_whole_regexp_locations() {
        for source in [
            "/(?<n>a)(b)#@value/",
            "/(?<n>a)(b)#$value/",
            "/(?<n>a)(b)#@@value/",
            "if /(?<n>a)(b)/; end",
            "%r{(?<n>a)(b)}",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for option in ["e", "s"] {
            assert!(offenses(&format!("/(?<n>a)(b)/{option}")).is_empty());
        }
    }
    #[test]
    fn invalid_numeric_like_names_have_no_successful_capture_tree() {
        for name in ["0", "1", "12", "1a", "-", "-1", "-a"] {
            for pattern in [format!("(?<{name}>a)(b)"), format!("(?'{name}'a)(b)")] {
                for option in ["", "i", "m", "x", "n", "u", "imx"] {
                    let source = format!("/{pattern}/{option}");
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
        }
    }
    #[test]
    fn numeric_references_must_resolve_to_plain_capture_identifiers() {
        for reference in [
            r"\1", r"\k<1>", r"\k'1'", r"\k<-2>", r"\g<1>", r"\g'-2'", r"\k<1+0>",
        ] {
            let source = format!("/(?<n>a)(b){reference}/");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for reference in [
            r"\2", r"\k<2>", r"\k'2'", r"\k<-1>", r"\g<2>", r"\g'-1'", r"\k<2+0>", r"\g<0>",
            r"\k<n>", r"\g'n'",
        ] {
            let source = format!("nil\n  /(?<n>a)(b){reference}/");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert!(found[0].correction.is_none());
        }
        for (pattern, expected) in [
            (r"(b)(?<n>a)\1", 1),
            (r"(b)(?<n>a)\2", 0),
            (r"(?<n>a)\g<+1>(b)", 1),
            (r"(b)\g<+1>(?<n>a)", 0),
            (r"(?<n>a)(b)(?(1)c|d)", 0),
            (r"(?<n>a)(b)(?(2)c|d)", 1),
            (r"(?<n>a)(b)(?(<n>)c|d)", 1),
        ] {
            let source = format!("/{pattern}/");
            assert_eq!(offenses(&source).len(), expected, "{source}");
        }
    }
    #[test]
    fn nonnumeric_capture_names_are_not_restricted_to_ruby_local_identifiers() {
        for name in ["a1", "_", "+a", "a-b", "a b", "é", "a:b", "@foo"] {
            for pattern in [format!("(?<{name}>a)(b)"), format!("(?'{name}'a)(b)")] {
                let source = format!("/{pattern}/");
                assert_eq!(offenses(&source).len(), 1, "{source}");
            }
        }
    }
    #[test]
    fn reference_tokens_preserve_literal_fallback_and_reference_kind_boundaries() {
        for (pattern, expected) in [
            (r"(?<n>a)(b)\k<>", 0),
            (r"(?<n>a)(b)\g<>", 0),
            (r"(?<n>a)(b)\k<1a>", 1),
            (r"(?<n>a)(b)\g<1a>", 1),
            (r"(?<n>a)(b)\k<", 1),
            (r"(?<n>a)(b)\g<00>", 0),
            (r"(?<n>a)(b)\k<0+1>", 0),
            (r"(?<n>a)(b)\g<-1+0>", 1),
            (r"(?<+1>a)(b)\k<+1>", 1),
            (r"(?<+1>a)(b)\g<+1>", 0),
            (r"(?<a>a)(?<a-b>b)(c)\k<a-b>", 0),
            (r"(?<a-b>a)(b)\g<a-b>", 1),
            (r"(?<n>a)(b)(?(<2>)c|d)", 0),
            (r"(?<n>a)(b)(?(n)c|d)", 0),
        ] {
            let source = format!("/{pattern}/");
            assert_eq!(offenses(&source).len(), expected, "{source}");
        }
    }
    #[test]
    fn repeated_literal_reference_prefixes_do_not_rescan_missing_delimiters() {
        for prefix in [r"\k<a", r"\g<a", r"\k<1a"] {
            let source = format!("/(?<n>a)(b){}/", prefix.repeat(10_000));
            assert_eq!(offenses(&source).len(), 1);
        }
    }
    #[test]
    fn source_tree_validation_and_option_lifetimes_are_shared() {
        // Duplicate sets reuse the proven source-tree validation too, rather
        // than preserving known false positives.
        for (source, mixed, duplicates) in [
            (r"/(?<1>a)(b)[aa]/", 0, 0),
            (r"/(?<n>a)(b)\1[aa]/", 0, 0),
            (r"/((?x))#(?<n>a)[aa]/", 0, 0),
            (r"/(?<n>a)(b)\2[aa]/", 1, 1),
            (r"/(?<n>a)(b)(?u-a)[aa]/", 0, 0),
            (r"/(?<n>a)(b)(?d-u:[aa])/", 0, 0),
            (r"/(?<n>a)(b)(?a-imx:[aa])/", 1, 1),
        ] {
            let found = lint_str(source);
            assert_eq!(
                found
                    .iter()
                    .filter(|o| o.cop == "Lint/MixedRegexpCaptureTypes")
                    .count(),
                mixed,
                "{source}"
            );
            let older: Vec<_> = found
                .iter()
                .filter(|o| o.cop == "Lint/DuplicateRegexpCharacterClassElement")
                .collect();
            assert_eq!(older.len(), duplicates, "{source}");
            assert!(older.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn pinned_no_offenses() {
        for source in [
            "/(?<foo>foo?<bar>bar)/",
            "/(?<=>)(<br>)(?=><)/",
            "/(foo)(bar)/",
            "/(?<foo>bar)(?:bar)/",
            "/data = ({\"words\":.+}}}[^}]*})/m",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for value in [
            "var",
            "@var",
            "@@var",
            "$var",
            "do_something",
            "CONST",
            "'x'",
            "1",
            "",
        ] {
            let source = format!("var = '(\\d+)'\n/(?<foo>#{{{value}}}*)(plain)/\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
}
