//! `Lint/ArrayLiteralInRegexp` — direct array-valued regexp interpolations.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ArrayLiteralInRegexp;
impl Cop for ArrayLiteralInRegexp {
    fn name(&self) -> &'static str {
        "Lint/ArrayLiteralInRegexp"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_embedded_statements_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(interpolation) = node.as_embedded_statements_node() else {
            return;
        };
        if !cx.parent().is_some_and(|p| {
            matches!(
                p,
                Node::InterpolatedRegularExpressionNode { .. }
                    | Node::InterpolatedMatchLastLineNode { .. }
            )
        }) {
            return;
        }
        let Some(array) = interpolation
            .statements()
            .and_then(|s| s.body().iter().last())
            .and_then(|n| n.as_array_node())
        else {
            return;
        };
        let mut character_class = true;
        let mut literal = true;
        let mut values = Vec::new();
        for element in array.elements().iter() {
            match literal_value(&element, file) {
                Some((single, value)) => {
                    character_class &= single;
                    values.push(regexp_escape(&value));
                }
                None => {
                    literal = false;
                    break;
                }
            }
        }
        // Each value's `to_s`, escaped, as a character class or alternation.
        let correction = (literal && cx.corrections_requested()).then(|| Correction {
            range: node.location().start_offset()..node.location().end_offset(),
            replacement: if character_class {
                format!("[{}]", values.concat())
            } else {
                format!("(?:{})", values.join("|"))
            },
        });
        let category = if !literal {
            "alternation or a character class"
        } else if character_class {
            "a character class"
        } else {
            "alternation"
        };
        let (line, col) = reference_line_col(node.location().start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use {category} instead of interpolating an array in a regexp."),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction,
        });
    }
}

// Only the pinned str/sym/int/float/true/false/nil types, not constant flow,
// grouping, calls, nested arrays or rational/imaginary literals. Float#to_s
// always has more than one character; integer spelling is irrelevant (0x1).
/// Whether the value's `to_s` is one character, and that `to_s` (rubocop-ast's
/// `value`, or the source of `true`, `false` and `nil`).
fn literal_value(node: &Node<'_>, file: &SourceFile) -> Option<(bool, String)> {
    match node {
        Node::IntegerNode { .. } => {
            let value = node.as_integer_node().unwrap().value();
            let (negative, words) = value.to_u32_digits();
            Some((
                words.len() == 1 && (words[0] == 0 || (!negative && words[0] < 10)),
                crate::cops::style::redundant_format::integer_to_s(node),
            ))
        }
        Node::FloatNode { .. } => Some((
            false,
            crate::cops::style::redundant_format::float_to_s(node.as_float_node().unwrap().value()),
        )),
        Node::TrueNode { .. } | Node::FalseNode { .. } | Node::NilNode { .. } => Some((
            false,
            String::from_utf8_lossy(node.location().as_slice()).into_owned(),
        )),
        Node::SourceLineNode { .. } => {
            let line = file.line_of(node.location().start_offset());
            Some((line < 10, line.to_string()))
        }
        Node::StringNode { .. } => {
            let s = node.as_string_node().unwrap();
            let opening = s.opening_loc();
            let single = opening.as_ref().is_some_and(|l| {
                l.as_slice().starts_with(b"'")
                    || l.as_slice().starts_with(b"%q")
                    || l.as_slice().starts_with(b"<<'")
                    || l.as_slice().starts_with(b"<<-'")
                    || l.as_slice().starts_with(b"<<~'")
            });
            let parts = crate::cops::ast::require_literal::parts(
                s.content_loc().as_slice(),
                s.unescaped(),
                single,
                file,
            );
            if parts.len() != 1
                || (s.unescaped().is_empty()
                    && opening.is_some_and(|l| {
                        // Character literals stay str even when a Unicode
                        // escape decodes to an empty value.
                        !matches!(l.as_slice(), b"'" | b"\"" | b"?")
                            || s.content_loc().as_slice().contains(&b'\n')
                    }))
            {
                return None;
            }
            Some((
                character_count(&parts[0], file) == 1,
                String::from_utf8_lossy(&parts[0]).into_owned(),
            ))
        }
        Node::InterpolatedStringNode { .. } => {
            let s = node.as_interpolated_string_node().unwrap();
            let opening = s.opening_loc()?;
            if !opening.as_slice().starts_with(b"<<") {
                return None;
            }
            // visit_heredoc drops empty str children and glues adjacent
            // non-newline-terminated fragments. Prism can retain an empty
            // continuation part even in a fully static squiggly heredoc.
            let single = opening.as_slice().ends_with(b"'");
            let mut children: Vec<Vec<u8>> = vec![];
            for part in s.parts().iter() {
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
            (children.len() == 1).then(|| {
                (
                    character_count(&children[0], file) == 1,
                    String::from_utf8_lossy(&children[0]).into_owned(),
                )
            })
        }
        Node::SymbolNode { .. } => {
            let s = node.as_symbol_node().unwrap();
            let raw = s.value_loc();
            // The reference builds an empty dsym for absent raw content,
            // but a sym for a nonempty escape that decodes to empty. Physical
            // line continuations can still leave it with no str children.
            if s.unescaped().is_empty()
                && raw
                    .as_ref()
                    .is_none_or(|l| l.as_slice().is_empty() || l.as_slice().contains(&b'\n'))
            {
                return None;
            }
            let single = s
                .opening_loc()
                .is_some_and(|l| l.as_slice() == b":'" || l.as_slice().starts_with(b"%s"));
            let parts = crate::cops::ast::require_literal::parts(
                raw.as_ref().map_or(s.unescaped(), |l| l.as_slice()),
                s.unescaped(),
                single,
                file,
            );
            (parts.len() == 1).then(|| {
                (
                    character_count(&parts[0], file) == 1,
                    String::from_utf8_lossy(&parts[0]).into_owned(),
                )
            })
        }
        Node::SourceFileNode { .. } => {
            let path = file.source_path().as_os_str().as_encoded_bytes();
            Some((
                path.utf8_chunks()
                    .map(|c| c.valid().chars().count() + c.invalid().len())
                    .sum::<usize>()
                    == 1,
                String::from_utf8_lossy(path).into_owned(),
            ))
        }
        _ => None,
    }
}

/// Ruby's `Regexp.escape`.
fn regexp_escape(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\x0c' => out.push_str("\\f"),
            '\x0b' => out.push_str("\\v"),
            ' ' => out.push_str("\\ "),
            '[' | ']' | '{' | '}' | '(' | ')' | '|' | '-' | '*' | '.' | '\\' | '?' | '+' | '^'
            | '$' | '#' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}
// These cops retain the reference buffer's leading BOM character in columns.
// Reuse the accepted encoding projection without changing prior cop policies.
pub(crate) fn reference_line_col(offset: usize, file: &SourceFile) -> (u32, u32) {
    let (line, col) = super::duplicate_match_pattern::buffer_line_col(offset, file);
    (
        line,
        col + u32::from(line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf")),
    )
}
fn character_count(value: &[u8], file: &SourceFile) -> usize {
    if file.match_pattern_binary() {
        value.len()
    } else {
        value
            .utf8_chunks()
            .map(|c| c.valid().chars().count() + c.invalid().len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/array_literal_in_regexp_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ArrayLiteralInRegexp")
            .collect()
    }
    fn check(array: &str, category: &str) {
        let source = format!("nil\n /#{{{array}}}/\n");
        let found = offenses(&source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            format!("Use {category} instead of interpolating an array in a regexp.")
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn pinned_character_classes() {
        for array in [
            "%w[a]",
            "%w[a b c]",
            "[\"a\", \"b\", \"c\"]",
            "%i[a b c]",
            "[:a, :b, :c]",
            "[1, 2, 3]",
            "%w[^ - $ |]",
            "%w[あ い 🐧]",
        ] {
            check(array, "a character class");
        }
    }
    #[test]
    fn pinned_alternations() {
        for array in [
            "%w[foo]",
            "%w[foo bar baz]",
            "%w[a b cat]",
            "[1.0, 2.5, 4.7]",
            "[\"a\", 5, 18.9]",
            "%w[^^ -- $$ || ++ **]",
            "%w[true false nil]",
            "[true, false, nil]",
            "%w[❤️ 💚 💙]",
        ] {
            check(array, "alternation");
        }
    }
    #[test]
    fn empty_unicode_literals_and_dynamic_empty_lookalikes() {
        for (array, category) in [
            (r#"[:"\u{}"]"#, "alternation"),
            (r#"[?\u{}]"#, "alternation"),
            (r#"[:"\u{ }"]"#, "alternation"),
            (r#"[?\u{ }]"#, "alternation"),
            (r#"["\u{}"]"#, "alternation"),
            (r#"[:""]"#, "alternation or a character class"),
            ("[:'']", "alternation or a character class"),
            ("[%s()]", "alternation or a character class"),
            (r#"[%Q(\u{})]"#, "alternation or a character class"),
            ("[:\"\\u{}\\\n\"]", "alternation or a character class"),
        ] {
            let source = format!("/#{{{array}}}/");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 2), "{source}");
            assert_eq!(
                found[0].message,
                format!("Use {category} instead of interpolating an array in a regexp."),
                "{source}"
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_unknowns() {
        for array in [
            "[foo]",
            "[\"#{foo}\"]",
            "[:\"#{foo}\"]",
            "[`foo`]",
            "[1r]",
            "[1i]",
            "[1..2]",
            "[1...2]",
            "[/abc/]",
            "[[]]",
            "[{}]",
        ] {
            check(array, "alternation or a character class");
        }
    }
    #[test]
    fn pinned_no_offense_and_direct_parent_boundaries() {
        for source in [
            "/[abc]/",
            "/#{foo}/",
            "\"#{%w[a b c]}\"",
            "/#{}/",
            "/#{([1])}/",
            "/#{begin; [1]; end}/",
            "/#{[1].freeze}/",
            "/#{[1]; nil}/",
            "%I[#{[1]}]",
            "`#{[1]}`",
            ":\"#{[1]}\"",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("if /#{nil; [1]}/\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 5));
    }
}
