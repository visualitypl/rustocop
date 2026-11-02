//! `Lint/UnescapedBracketInRegexp` — literal closing brackets outside sets.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct UnescapedBracketInRegexp;
impl Cop for UnescapedBracketInRegexp {
    fn name(&self) -> &'static str {
        "Lint/UnescapedBracketInRegexp"
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
        let regexp = crate::cops::style::regexp_literal::Regexp::new(node);
        let content = if let Some(regexp) = &regexp {
            regexp.opening.end_offset()..regexp.closing.start_offset()
        } else {
            let Some(call) = node.as_call_node() else {
                return;
            };
            if !matches!(call.name().as_slice(), b"new" | b"compile")
                || call
                    .call_operator_loc()
                    .is_some_and(|loc| loc.as_slice() == b"&.")
                || !call
                    .receiver()
                    .is_some_and(|r| crate::cops::ast::is_global_const(&r, &["Regexp"]))
            {
                return;
            }
            struct Interpolation(bool);
            impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Interpolation {
                fn enter(&mut self, node: Node<'pr>) {
                    self.0 |= dynamic_string(&node);
                }
            }
            let mut interpolation = Interpolation(false);
            // RuboCop's send excludes its attached block (the block is its
            // parent). Arguments, including nested literal blocks, are children.
            if let Some(arguments) = call.arguments() {
                crate::cops::walk::walk(&mut interpolation, &arguments.as_node());
            }
            if interpolation.0 {
                return;
            }
            let Some(string) = call
                .arguments()
                .and_then(|a| a.arguments().iter().next())
                .and_then(|n| n.as_string_node())
            else {
                return;
            };
            let (Some(opening), Some(closing)) = (string.opening_loc(), string.closing_loc())
            else {
                return;
            };
            let range = opening.end_offset()..closing.start_offset();
            // Parser::Source::Buffer normalizes CRLF before comparing source
            // to value. Preserve original raw boundaries for findings/edits.
            if range.start > range.end || !verbatim(&file.bytes[range.clone()], string.unescaped())
            {
                return;
            }
            range
        };
        let correctable = cx.corrections_requested() && closed_defined_literal(node, cx, file);
        for (range, (line, col)) in
            super::duplicate_regexp_character_class_element::unescaped_brackets(
                regexp.as_ref(),
                content,
                file,
            )
        {
            let correction = correctable.then(|| crate::offense::Correction {
                range: range.clone(),
                replacement: "\\]".into(),
            });
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Regular expression has `]` without escape.".into(),
                line,
                col,
                end_line: file.line_of(range.end - 1),
                correction,
            });
        }
    }
}

fn verbatim(raw: &[u8], value: &[u8]) -> bool {
    raw.iter()
        .enumerate()
        .filter_map(|(i, b)| (*b != b'\r' || raw.get(i + 1) != Some(&b'\n')).then_some(b))
        .eq(value.iter())
}

// Prism's parser-AST translator also composes multiline static Ruby strings
// into dstr nodes. A final newline alone does not add a second part; double-
// quoted line continuations glue lines. Regexp/word-array fragments (no opening
// token) are not independent Ruby string expressions.
fn dynamic_string(node: &Node<'_>) -> bool {
    if node.as_interpolated_string_node().is_some() {
        return true;
    }
    let Some(string) = node.as_string_node() else {
        return false;
    };
    let Some(opening) = string.opening_loc() else {
        return false;
    };
    let noninterpolating =
        opening.as_slice().ends_with(b"'") || opening.as_slice().starts_with(b"%q");
    let content = string.content_loc();
    let bytes = content.as_slice();
    let mut slashes = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' && i + 1 < bytes.len() && (noninterpolating || slashes % 2 == 0) {
            return true;
        }
        if b == b'\\' {
            slashes += 1;
        } else if b != b'\r' {
            slashes = 0;
        }
    }
    false
}

// Escaping changes Regexp#source and native warnings, and constructor strings
// change values/evaluation. Only a consumed, closed [defined?(STATIC_REGEXP)]
// is proved: defined? recognizes it without creating a Regexp. No calls,
// bindings, delayed execution, source/DATA observers or additional statements.
// Keep the proof intentionally smaller than detection. Central main-parse
// encoding/line/code-object/heap/heredoc/atomic guards still apply to every edit.
fn closed_defined_literal(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let Some(regexp) = node.as_regular_expression_node() else {
        return false;
    };
    if file.data_start().is_some()
        || !regexp
            .content_loc()
            .as_slice()
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b']')
    {
        return false;
    }
    let mut ancestors = cx.ancestors();
    if ancestors
        .next()
        .is_none_or(|n| n.as_defined_node().is_none())
    {
        return false;
    }
    let Some(array) = ancestors.next().and_then(|n| n.as_array_node()) else {
        return false;
    };
    let Some(statements) = ancestors.next().and_then(|n| n.as_statements_node()) else {
        return false;
    };
    array.elements().len() == 1
        && statements.body().len() == 1
        && ancestors
            .next()
            .is_some_and(|n| n.as_program_node().is_some())
        && ancestors.next().is_none()
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/unescaped_bracket_in_regexp_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnescapedBracketInRegexp")
            .collect()
    }
    #[test]
    fn all_pinned_specs_sources_controls_and_canonical_adaptations() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/unescaped_bracket_in_regexp.json"
        ))
        .unwrap();
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let found = offenses(source);
            let actual: Vec<_> = found
                .iter()
                .map(|o| serde_json::json!([o.line, o.col, o.end_line, o.message]))
                .collect();
            assert_eq!(serde_json::json!(actual), row["findings"], "{source}");
        }
    }
    #[test]
    fn deep_pattern_and_wide_constructor_use_bounded_source_work() {
        // Exercise the scanner directly: the pipeline still enforces its
        // mandatory parser/resource limits before running ordinary cops.
        // No native regexp construction or candidate parses are involved.
        let source = format!("/{}a]{}/", "(?:".repeat(8_000), ")".repeat(8_000));
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let found = crate::cops::lint::duplicate_regexp_character_class_element::unescaped_brackets(
            None,
            1..source.len() - 1,
            &file,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, (1, 24_003));
        let source = format!("Regexp.new('{}]')", "a".repeat(100_000));
        let found = offenses(&source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].col, 100_013);
    }
    #[test]
    fn literal_brackets_positions_and_message() {
        for (source, cols) in [
            ("/abc]123/", vec![5]),
            (r"/abc\\]123/", vec![7]),
            ("/abc]123]/i", vec![5, 9]),
            ("%r{abc]123}", vec![7]),
            ("/[^]]x]/", vec![7]),
            ("/[a]]/", vec![5]),
            (r"/[\é]]/", vec![6]),
        ] {
            let found = offenses(&format!("# keep\n{source}\n"));
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                cols,
                "{source}"
            );
            for o in found {
                assert_eq!((o.line, o.end_line), (2, 2));
                assert_eq!(o.message, "Regular expression has `]` without escape.");
            }
        }
    }
    #[test]
    fn escaped_first_and_character_class_lookalikes() {
        for source in [
            "/]/",
            "%r{]}",
            r"/abc\]123/",
            "/[abc]/",
            "/[^]]/",
            "/[^]x]/",
            "/foo[^]]bar/",
            r"/[^]\]]/",
            "/(?<=[<>=:])/",
            r"/[\§]/",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn canonical_escaping_only_for_closed_unevaluated_consumed_literal() {
        let registry = crate::cops::Registry::new();
        let cop = registry
            .cops()
            .iter()
            .find(|c| c.name() == "Lint/UnescapedBracketInRegexp")
            .unwrap();
        for (source, safe) in [
            ("[defined?(/abc]123/)]\n", true),
            ("/abc]123/\n", false),
            ("[defined?(Regexp.new('abc]123'))]\n", false),
            ("[defined?(/abc]123/)]\np __LINE__\n", false),
            ("[defined?(/abc]123/)]\n__END__\ndata\n", false),
        ] {
            let found = crate::pipeline::lint_str_with_corrections(&[cop.as_ref()], source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].correction.is_some(), safe, "{source}");
            if safe {
                assert_eq!(
                    crate::corrections::apply(source.as_bytes(), &found)
                        .unwrap()
                        .bytes,
                    b"[defined?(/abc\\]123/)]\n"
                );
            }
        }
    }
    #[test]
    fn control_meta_escape_failed_trees_and_token_boundaries() {
        let source = "Regexp.new('\\C]')\nRegexp.compile('\\M]')\nRegexp.new('\\c\\c]a]')\n";
        let found = offenses(source);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 17), (3, 19)]
        );
        for offense in found {
            assert_eq!(offense.end_line, 3);
            assert_eq!(
                offense.message,
                "Regular expression has `]` without escape."
            );
        }
    }
    #[test]
    fn verbatim_constructors_only() {
        for method in ["new", "compile"] {
            for receiver in ["Regexp", "::Regexp"] {
                for quote in ["'", "\"", "%q{", "%Q{"] {
                    let close = if quote.ends_with('{') { "}" } else { quote };
                    let source = format!("{receiver}.{method}({quote}abc]123{close}, 'i')");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(found[0].col, source.find(']').unwrap() as u32 + 1);
                }
            }
            for pattern in [
                "']'",
                "'[abc]'",
                r"'abc\]123'",
                r#""a\\.b]""#,
                r##""#{value}]""##,
                "'*'",
                "'+'",
                "'{42}'",
            ] {
                assert!(
                    offenses(&format!("Regexp.{method}({pattern})")).is_empty(),
                    "{pattern}"
                );
            }
        }
        for source in [
            "Other::Regexp.new('abc]')",
            "object.new('abc]')",
            "Regexp.new(pattern)",
            "Regexp&.new('abc]')",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
