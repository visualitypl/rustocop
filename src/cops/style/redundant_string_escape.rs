//! `Style/RedundantStringEscape` — unnecessary punctuation escapes in strings.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantStringEscape;
impl Cop for RedundantStringEscape {
    fn name(&self) -> &'static str {
        "Style/RedundantStringEscape"
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
        // The existing main preorder dispatch visits this typed owner before
        // its following static fragment. Never infer name boundaries from a
        // last byte: CP932 ソ/マ end in ASCII-looking 0x5c/0x7d.
        if cx.corrections_requested()
            && let Some(embedded) = node.as_embedded_variable_node()
            && matches!(
                embedded.variable(),
                Node::InstanceVariableReadNode { .. }
                    | Node::ClassVariableReadNode { .. }
                    | Node::GlobalVariableReadNode { .. }
            )
        {
            file.record_string_escape_variable_end(embedded.location().end_offset());
        }
        let Some(string) = node.as_string_node() else {
            return;
        };
        // Prism represents static parts of strings, regexps, commands and symbols
        // alike as StringNodes. Only the upstream on_str dispatch participates;
        // a dsym's static fragments have no string delimiters upstream.
        let mut opening = string.opening_loc();
        let mut closing = string.closing_loc();
        if opening.is_none() {
            for parent in cx.ancestors() {
                if let Some(s) = parent.as_interpolated_string_node() {
                    opening = s.opening_loc();
                    closing = s.closing_loc();
                } else if let Some(a) = parent.as_array_node() {
                    opening = a.opening_loc();
                    closing = a.closing_loc();
                } else {
                    return;
                }
                if opening.is_some() {
                    break;
                }
            }
        }
        let Some(opening) = opening else {
            return;
        };
        let raw = opening.as_slice();
        let heredoc = raw.starts_with(b"<<");
        let words = raw.starts_with(b"%W") || raw.starts_with(b"%w");
        if raw == b"?" || raw.starts_with(b"%q") || raw.starts_with(b"%w") || raw.ends_with(b"'") {
            return;
        }
        let content = string.content_loc();
        let range = content.start_offset()..content.end_offset();
        let mut chars = file.character_offsets(range.clone()).peekable();
        let mut escaped_hash = None;
        let mut compound_target = false;
        while let Some(at) = chars.next() {
            // A control/meta prefix consumes one character or a nested escape.
            // Keep scanning nested escapes for diagnostics, but never edit them.
            let inside_compound = std::mem::take(&mut compound_target);
            if file.bytes[at] != b'\\' {
                continue;
            }
            let Some(next) = chars.next() else {
                break;
            };
            let end = chars.peek().copied().unwrap_or(range.end);
            let escaped = &file.bytes[next..end];
            let b = file.bytes[next];
            if b == b'c' {
                compound_target = true;
            } else if matches!(b, b'C' | b'M') && file.bytes.get(end) == Some(&b'-') {
                chars.next(); // the prefix's hyphen, not its target
                compound_target = true;
            }
            if b == b'#' {
                escaped_hash = Some(next);
            }
            let alnum = file.character_alphanumeric(&file.bytes[next..range.end.min(next + 6)]);
            let disables = b == b'#'
                && (matches!(file.bytes.get(end), Some(b'{' | b'$' | b'@'))
                    || file.bytes.get(end..end + 2) == Some(b"\\{"))
                || matches!(b, b'{' | b'$' | b'@')
                    && at >= 2
                    && file.bytes[at - 1] == b'#'
                    && file.bytes[at - 2] != b'\\';
            if b == b'\\'
                || b == b'\n'
                || alnum
                || disables
                || b == b' ' && (words || heredoc)
                || !heredoc
                    && (raw.last() == Some(&b)
                        || closing.as_ref().and_then(|c| c.as_slice().first()) == Some(&b))
            {
                continue;
            }
            let (line, col) = file.line_col(at);
            // Unbraced variable interpolation ends immediately before a static
            // fragment. Underscore is the only reported ASCII identifier suffix
            // (letters/digits are already excluded); deleting its slash can
            // extend the preceding name. Typed ownership works for every
            // encoding; braced interpolation is a different AST node.
            let may_extend_variable = b == b'_'
                && at == range.start
                && string.opening_loc().is_none()
                && file.follows_string_escape_variable(at);
            let correction = if cx.corrections_requested()
                && !inside_compound
                && !may_extend_variable
                // Ruby also treats physical vertical tab as word whitespace;
                // Rust's is_ascii_whitespace deliberately excludes it.
                && b.is_ascii() && !b.is_ascii_whitespace() && b != b'\x0b'
                // Do not activate interpolation, even where the pin's two-byte
                // lookbehind is too conservative about a preceding slash run.
                // Only \#\{ guarantees the hash escape itself stays: for @/$,
                // deleting both individually redundant slashes is unsafe.
                && !(matches!(b, b'{' | b'$' | b'@') && at > 0 && file.bytes[at-1] == b'#' && !(b == b'{' && escaped_hash == Some(at-1)))
            {
                super::literal_spelling::safe_edit(
                    file,
                    cx,
                    Correction {
                        range: at..next,
                        replacement: String::new(),
                    },
                )
            } else {
                None
            };
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: line,
                message: format!(
                    "Redundant escape of {} inside string literal.",
                    String::from_utf8_lossy(escaped)
                ),
                correction,
            });
        }
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_string_escape_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    const WRAPS: [(&str, &str); 6] = [
        ("\"", "\""),
        ("%Q(", ")"),
        ("%Q!", "!"),
        ("%(", ")"),
        ("%W[", "]"),
        ("<<~MYHEREDOC\n", "\nMYHEREDOC"),
    ];
    #[test]
    fn pinned_diagnostics_and_shared_corrections() {
        for (l, r) in WRAPS {
            for (before, after, at, ch) in [
                (r"\##{whatever}", "##{whatever}", 0, '#'),
                (r"\#whatever", "#whatever", 0, '#'),
                (r"#\{whatever\}", r"#\{whatever}", 11, '}'),
                (r"\#\{whatever}", r"\#{whatever}", 2, '{'),
                (r"\#", "#", 0, '#'),
                (r"\'", "'", 0, '\''),
            ] {
                let source = format!("{l}{before}{r}\n");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == RedundantStringEscape.name())
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                let (line, col) = if l.contains('\n') {
                    (2, at + 1)
                } else {
                    (1, l.len() + at + 1)
                };
                assert_eq!(
                    (found[0].line, found[0].col),
                    (line, col as u32),
                    "{source}"
                );
                assert_eq!(
                    found[0].message,
                    format!("Redundant escape of {ch} inside string literal.")
                );
                assert_correction(&RedundantStringEscape, &source, &format!("{l}{after}{r}\n"));
            }
            if r != "\"" && !l.starts_with("<<") {
                assert_correction(
                    &RedundantStringEscape,
                    &format!("{l}\\\"{r}\n"),
                    &format!("{l}\"{r}\n"),
                );
                assert_correction(
                    &RedundantStringEscape,
                    &format!("{l}#{{\"\\{r}\"}}{r}\n"),
                    &format!("{l}#{{\"{r}\"}}{r}\n"),
                );
            }
        }
        assert_correction(
            &RedundantStringEscape,
            "\"\\'\"\\\n  \"\\'\"\n",
            "\"'\"\\\n  \"'\"\n",
        );
    }
    #[test]
    fn pinned_indented_heredoc_correction_calls() {
        for (before, after) in [
            (r"\##{whatever}", "##{whatever}"),
            (r"\#whatever", "#whatever"),
            (r"\#", "#"),
            (r"\'", "'"),
        ] {
            assert_correction(
                &RedundantStringEscape,
                &format!("<<~MYHEREDOC\n  {before}\nMYHEREDOC\n"),
                &format!("<<~MYHEREDOC\n  {after}\nMYHEREDOC\n"),
            );
        }
    }

    #[test]
    fn common_escaped_delimiters_and_single_quote_lookalikes() {
        for (l, r) in WRAPS
            .into_iter()
            .chain([("'", "'"), ("%q(", ")"), ("%w(", ")")])
            .filter(|(l, _)| !l.starts_with("<<"))
        {
            for content in [format!("\\{r}a\\{r}"), format!("\\{r}#{{my_lvar}}")] {
                let source = format!("{l}{content}{r}\n");
                assert!(
                    correction_offenses(&RedundantStringEscape, &source).is_empty(),
                    "{source}"
                );
            }
        }
        for (l, r) in [
            ("'", "'"),
            ("%q(", ")"),
            ("%w(", ")"),
            ("<<~'MYHEREDOC'\n", "\nMYHEREDOC"),
        ] {
            for content in [r"\#whatever", r"\#", r"\'", r#"\""#, "foo\\!\nbar"] {
                let source = format!("{l}{content}{r}\n");
                assert!(
                    correction_offenses(&RedundantStringEscape, &source).is_empty(),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn composing_escape_deletions_cannot_activate_variable_interpolation() {
        for (before, after) in [
            (r##""\#\@foo""##, r##""#\@foo""##),
            (r##""\#\$foo""##, r##""#\$foo""##),
            (r##""\#\@@foo""##, r##""#\@@foo""##),
        ] {
            let found = correction_offenses(&RedundantStringEscape, before);
            assert_eq!(found.len(), 2);
            assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 1);
            assert_correction(&RedundantStringEscape, before, after);
        }
    }

    #[test]
    fn variable_suffix_compound_escapes_and_vertical_tabs_are_not_deleted() {
        for source in [
            r##""#@name\_""##,
            r##""#@@name\_""##,
            r##""#$name\_""##,
            r#""\c\?""#,
            r#""\C-\?""#,
            r#""\M-\C-\?""#,
            r#""\c\M-\?""#,
            "%W[a\\\u{b}b]",
            "<<~EOS\n  \\\u{b}text\nEOS\n",
        ] {
            let found = correction_offenses(&RedundantStringEscape, source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert!(found[0].correction.is_none(), "{source:?}");
        }
        // Braced interpolation, punctuation after a variable, and independent
        // escapes after a complete compound escape remain correctable.
        for (source, expected) in [
            (r##""#{@name}\_""##, r##""#{@name}_""##),
            (r##""#@name!\_""##, r##""#@name!_""##),
            (r#""\c?\!""#, r#""\c?!""#),
            (r#""\M-\C-?\!""#, r#""\M-\C-?!""#),
        ] {
            assert_correction(&RedundantStringEscape, source, expected);
        }
    }

    #[test]
    fn typed_variable_ownership_is_lazy_and_linear_in_main_dispatch() {
        use crate::{cops::run_cops_with_corrections, parser};
        // Thousands of alternating typed parts in ONE owner catch accidental
        // per-fragment sibling scans. Each unbraced owner contributes one end;
        // braced expressions do not. Punctuation separates a suffix from that end.
        let source = format!(
            "\"{}\"",
            "#@ソ\\_#{@ソ}\\_#@@マ\\_#$ソ\\_#@ソ!\\_".repeat(2048)
        );
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = parser::parse(&file.bytes);
        file.record_encoding(&parsed);
        assert_eq!(parsed.errors().count(), 0);
        let active: Vec<&dyn Cop> = vec![&RedundantStringEscape];
        let mut plain = Vec::new();
        assert!(run_cops_with_corrections(&active, &parsed, &file, &mut plain, false).is_empty());
        assert_eq!(plain.len(), 5 * 2048);
        assert_eq!(file.string_escape_variable_count(), None);
        let mut corrected = Vec::new();
        assert!(
            run_cops_with_corrections(&active, &parsed, &file, &mut corrected, true).is_empty()
        );
        assert_eq!(corrected.len(), 5 * 2048);
        assert_eq!(file.string_escape_variable_count(), Some(4 * 2048));
        assert_eq!(
            corrected.iter().filter(|o| o.correction.is_some()).count(),
            2 * 2048
        );
        for (i, _) in source.match_indices("\\_") {
            // Main-parse ownership, not a suffix-byte heuristic.
            let prefix = &source[..i];
            assert_eq!(
                file.follows_string_escape_variable(i),
                prefix.ends_with("#@ソ") || prefix.ends_with("#@@マ") || prefix.ends_with("#$ソ")
            );
        }
    }

    #[test]
    fn symbol_fragments_do_not_receive_string_delimiters() {
        for source in [
            r##":"\##{value}""##,
            r##":"\!#{value}""##,
            r"%I[\##{value}]",
            r"%I[\!#{value}]",
        ] {
            assert!(
                correction_offenses(&RedundantStringEscape, source).is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn common_allowed_escapes_and_noninterpolating_literals() {
        for (l, r) in WRAPS.into_iter().chain([
            ("'", "'"),
            ("%q(", ")"),
            ("%w(", ")"),
            ("<<~'MYHEREDOC'\n", "\nMYHEREDOC"),
        ]) {
            for content in [
                "a",
                r"\\foo\\",
                r"\#$foo",
                r"#\$foo",
                r"\#@foo",
                r"#\@foo",
                r"\#@@foo",
                r"#\@@foo",
                r"\#{my_var}",
                r"#\{my_var}",
                "foo\\\nbar",
                r"foo\n",
                r"\d",
                r"fo\157",
                r"fo\x6f",
                r"fo\u006f",
                r"f\u{006f 006f}",
                r"\cc \C-c",
                r"\M-c",
                r"\M-\C-c \M-\cc \c\M-c",
                r"\c? \C-?",
            ] {
                let source = format!("{l}{content}{r}\n");
                assert!(
                    correction_offenses(&RedundantStringEscape, &source).is_empty(),
                    "{source}"
                );
            }
        }
        for source in [
            r"/\#/",
            r"%x{\#}",
            "?a",
            r"?\n",
            r#""\"""#,
            r##":"\"#{value}""##,
            r#""\\\\ \\'foo\\' \\\\""#,
            "%W[foo\\  bar\\  baz]",
            "%w[foo\\  bar\\  baz]",
            "<<~X\n\\ text\nX\n",
            "<<~X\n#{foo.gsub(/[a-z]+/, '\\\"')}\nX\n",
            "<<~X\n#{\n<<~'Y'\n\\#\nY\n}\nX\n",
        ] {
            assert!(
                correction_offenses(&RedundantStringEscape, source).is_empty(),
                "{source}"
            );
        }
    }
}
