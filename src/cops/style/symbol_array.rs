//! `Style/SymbolArray` — fixed percent style, MinSize 2, Ruby >= 2.0.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{ArrayNode, Node};
pub struct SymbolArray;
impl Cop for SymbolArray {
    fn name(&self) -> &'static str {
        "Style/SymbolArray"
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
        let Some(array) = node.as_array_node() else {
            return;
        };
        let Some(open) = array.opening_loc() else {
            return;
        };
        let percent = matches!(open.as_slice().get(..2), Some(b"%i" | b"%I"));
        if percent {
            if !complex(&array, file) {
                return;
            }
        } else if open.as_slice() != b"["
            || array.elements().len() < 2
            || array
                .elements()
                .iter()
                .any(|n| !crate::cops::ast::is_reference_symbol(&n, file))
            || complex(&array, file)
            || super::word_array::ambiguous_context(cx)
            || super::correction::contains_comment(
                file,
                array.location().start_offset()..array.location().end_offset(),
            )
        {
            return;
        }
        let replacement = if percent {
            brackets(&array, file)
        } else if cx.corrections_requested() {
            words(&array, file)
        } else {
            None
        };
        let message = if percent {
            match &replacement {
                Some(s) if !s.contains('\n') => format!("Use `{s}` for an array of symbols."),
                _ => "Use an array literal `[...]` for an array of symbols.".into(),
            }
        } else {
            "Use `%i` or `%I` for an array of symbols.".into()
        };
        let loc = array.location();
        let correction = if cx.corrections_requested() {
            replacement.and_then(|replacement| {
                if super::correction::contains_comment(file, loc.start_offset()..loc.end_offset()) {
                    return None;
                }
                super::literal_spelling::safe_edit(
                    file,
                    cx,
                    Correction {
                        range: loc.start_offset()..loc.end_offset(),
                        replacement,
                    },
                )
            })
        } else {
            None
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.last_line_of(loc),
            correction,
        });
    }
}
/// Replace multibyte characters by opaque non-delimiters. Never mistake a
/// Windows-31J trailing byte for ASCII bracket/backslash syntax.
fn append_content(out: &mut Vec<u8>, bytes: &[u8], utf8: bool, file: &SourceFile) {
    #[cfg(test)]
    CLASSIFIED_BYTES.set(CLASSIFIED_BYTES.get() + bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let width = if utf8 {
            match bytes[i] {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf4 => 4,
                _ => 1,
            }
        } else {
            file.encoded_character_width(&bytes[i..])
        };
        out.push(if bytes[i] < 128 { bytes[i] } else { b'x' });
        i += width.max(1);
    }
}
fn complex(array: &ArrayNode<'_>, file: &SourceFile) -> bool {
    for n in array.elements().iter() {
        // This is an ordered early RETURN, not an all-elements exemption.
        if matches!(n.location().as_slice(), b"[" | b"]" | b"(" | b")") {
            return false;
        }
        let mut content = Vec::new();
        if let Some(s) = n.as_symbol_node() {
            append_content(
                &mut content,
                s.unescaped(),
                s.is_forced_utf8_encoding(),
                file,
            );
        } else if let Some(s) = n.as_interpolated_symbol_node() {
            for p in s.parts().iter() {
                if let Some(s) = p.as_string_node() {
                    append_content(
                        &mut content,
                        s.unescaped(),
                        s.is_forced_utf8_encoding(),
                        file,
                    )
                } else {
                    append_content(&mut content, p.location().as_slice(), false, file)
                }
            }
        }
        if content.contains(&b' ') {
            return true;
        }
        let mut rest = Vec::new();
        let mut i = 0;
        while i < content.len() {
            let (open, close) = match content[i] {
                b'[' => (b'[', b']'),
                b'(' => (b'(', b')'),
                _ => {
                    rest.push(content[i]);
                    i += 1;
                    continue;
                }
            };
            let mut end = i + 1;
            while end < content.len()
                && !super::is_ruby_whitespace(content[end])
                && content[end] != open
                && content[end] != close
            {
                end += 1;
            }
            if content.get(end) == Some(&close) {
                i = end + 1
            } else {
                rest.push(content[i]);
                i += 1
            }
        }
        if rest.iter().any(|b| b"[]()".contains(b)) {
            return true;
        }
    }
    false
}
fn escaped(n: &Node<'_>, file: &SourceFile, percent: bool, upper: bool) -> Option<String> {
    let s = n.as_symbol_node()?;
    if (!file.is_utf8() && !s.is_forced_utf8_encoding() && !s.unescaped().is_ascii())
        || (s.is_forced_binary_encoding() && !s.unescaped().is_ascii())
    {
        return None;
    }
    let text = std::str::from_utf8(s.unescaped()).ok()?;
    if percent && text.is_empty() {
        return None;
    }
    // The reference detector strips parenthesis pairs too: `([)` is simple
    // there, but its square bracket is still syntax to our canonical %i[...].
    // Only leave square brackets raw when every prefix is balanced and the
    // final depth is zero. Otherwise escape ALL of them, including paired ones,
    // so each word is independently safe inside the outer percent delimiter.
    let escape_brackets = percent
        && text.bytes().try_fold(0usize, |depth, b| match b {
            b'[' => Some(depth + 1),
            b']' => depth.checked_sub(1),
            _ => Some(depth),
        }) != Some(0);
    let mut out = String::new();
    // A non-ASCII Unicode escape preserves the actual forced-UTF8 flag (an
    // ASCII-only escape does not). Prefer one wherever it occurs in the name.
    // Otherwise keep balanced [] pairs visible to the percent lexer: escaping
    // just one side would unbalance it. All-bracket names escape every side.
    let utf8_marker = text.chars().position(|c| !c.is_ascii()).or_else(|| {
        if percent {
            text.chars().position(|c| !matches!(c, '[' | ']'))
        } else {
            Some(0)
        }
    });
    for (i, c) in text.chars().enumerate() {
        // A Unicode escape forces the payload to UTF-8, not the source file.
        // In a legacy envelope EVERY non-ASCII glyph must stay escaped; one
        // marker alone would leak raw UTF-8 bytes into the native encoding.
        if upper
            && s.is_forced_utf8_encoding()
            && (utf8_marker == Some(i)
                || utf8_marker.is_none()
                || (!file.is_utf8() && !c.is_ascii()))
        {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
            continue;
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' if !percent => out.push_str("\\\""),
            '#' if upper => out.push_str("\\#"),
            ' ' if percent => out.push_str("\\ "),
            '[' | ']' if escape_brackets => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    Some(out)
}
fn physical_lines(raw: &[u8], out: &mut String) {
    for line in raw.split_inclusive(|b| *b == b'\n') {
        if line.ends_with(b"\n") {
            out.push_str(if line.ends_with(b"\r\n") {
                "\r\n"
            } else {
                "\n"
            });
        }
    }
}
fn words(array: &ArrayNode<'_>, file: &SourceFile) -> Option<String> {
    #[cfg(test)]
    WORD_BUILDS.set(WORD_BUILDS.get() + 1);
    let upper = array.elements().iter().any(|n| {
        n.as_symbol_node().is_some_and(|s| {
            s.is_forced_utf8_encoding()
                || s.unescaped().iter().any(|b| b.is_ascii_control())
                || std::str::from_utf8(s.unescaped()).is_ok_and(|s| s.chars().any(char::is_control))
        })
    });
    let mut out = if upper { "%I[" } else { "%i[" }.to_owned();
    let open = array.opening_loc()?;
    let close = array.closing_loc()?;
    let base = array.location().start_offset();
    let raw = array.location().as_slice();
    let mut cursor = open.end_offset();
    for n in array.elements().iter() {
        let gap = super::word_array::word_separator(
            &raw[cursor - base..n.location().start_offset() - base],
        )?;
        if cursor != open.end_offset() && gap.is_empty() {
            out.push(' ')
        } else {
            out.push_str(&gap)
        }
        out.push_str(&escaped(&n, file, true, upper)?);
        physical_lines(n.location().as_slice(), &mut out);
        cursor = n.location().end_offset();
    }
    out.push_str(&super::word_array::word_separator(
        &raw[cursor - base..close.start_offset() - base],
    )?);
    out.push(']');
    Some(out)
}
fn bare_symbol(bytes: &[u8]) -> bool {
    use std::sync::LazyLock;
    static IDENT: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
        regex::bytes::Regex::new(r"\A(?:[a-zA-Z_][a-zA-Z_0-9]*[!?]?|@@?[a-zA-Z_][a-zA-Z_0-9]*|\$[1-9][0-9]*|\$[a-zA-Z_][a-zA-Z_0-9]*)\z").unwrap()
    });
    IDENT.is_match(bytes)
        || [
            b"|".as_slice(),
            b"^",
            b"&",
            b"<=>",
            b"==",
            b"===",
            b"=~",
            b">",
            b">=",
            b"<",
            b"<=",
            b"<<",
            b">>",
            b"+",
            b"-",
            b"*",
            b"/",
            b"%",
            b"**",
            b"~",
            b"+@",
            b"-@",
            b"[]",
            b"[]=",
            b"`",
            b"!",
            b"!=",
            b"!~",
            b"$!",
            b"$\"",
            b"$$",
            b"$&",
            b"$'",
            b"$*",
            b"$+",
            b"$,",
            b"$/",
            b"$;",
            b"$:",
            b"$.",
            b"$<",
            b"$=",
            b"$>",
            b"$?",
            b"$@",
            b"$\\",
            b"$_",
            b"$`",
            b"$~",
            b"$0",
            b"$-0",
            b"$-F",
            b"$-I",
            b"$-K",
            b"$-W",
            b"$-a",
            b"$-d",
            b"$-i",
            b"$-l",
            b"$-p",
            b"$-v",
            b"$-w",
        ]
        .contains(&bytes)
}
fn brackets(array: &ArrayNode<'_>, file: &SourceFile) -> Option<String> {
    let open = array.opening_loc()?;
    let close = array.closing_loc()?;
    let base = array.location().start_offset();
    let raw = array.location().as_slice();
    let mut cursor = open.end_offset();
    let mut out = "[".to_owned();
    for (i, n) in array.elements().iter().enumerate() {
        // Dynamic symbols stay reported, but rebuilding code-bearing percent
        // fragments is not a proven spelling/encoding/binding-preserving edit.
        let s = n.as_symbol_node()?;
        out.push_str(
            std::str::from_utf8(&raw[cursor - base..n.location().start_offset() - base]).ok()?,
        );
        out.push(':');
        if bare_symbol(s.unescaped()) && !s.is_forced_utf8_encoding() {
            out.push_str(std::str::from_utf8(s.unescaped()).ok()?)
        } else {
            out.push('"');
            out.push_str(&escaped(&n, file, false, true)?);
            out.push('"');
        }
        if i + 1 < array.elements().len() {
            out.push(',')
        }
        physical_lines(n.location().as_slice(), &mut out);
        cursor = n.location().end_offset();
    }
    out.push_str(std::str::from_utf8(&raw[cursor - base..close.start_offset() - base]).ok()?);
    out.push(']');
    Some(out)
}
#[cfg(test)]
thread_local! {
    static CLASSIFIED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static WORD_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop symbol_array_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == SymbolArray.name())
            .collect()
    }
    #[test]
    fn bounded_matrix_work_and_lazy_builds() {
        for n in [1500, 3000, 6000] {
            let s = format!("[{}[:'complex content',:two]]", "[:one,:two],".repeat(n));
            CLASSIFIED_BYTES.set(0);
            WORD_BUILDS.set(0);
            assert_eq!(offenses(&s).len(), n);
            assert_eq!(WORD_BUILDS.get(), 0);
            assert!(CLASSIFIED_BYTES.get() <= s.len());
            let s = "[:one,:two]\n".repeat(n);
            CLASSIFIED_BYTES.set(0);
            WORD_BUILDS.set(0);
            let o = correction_offenses(&SymbolArray, &s);
            assert_eq!(o.iter().filter(|o| o.correction.is_some()).count(), n);
            assert_eq!(WORD_BUILDS.get(), n);
            assert!(CLASSIFIED_BYTES.get() <= s.len());
        }
        let s = format!("[:\"{}\",:two]", "é".repeat(100_000));
        CLASSIFIED_BYTES.set(0);
        assert_eq!(offenses(&s).len(), 1);
        assert!(CLASSIFIED_BYTES.get() <= s.len());
    }
    #[test]
    fn reference_empty_and_split_symbols_are_dynamic_not_static_candidates() {
        for s in [
            r#"[:foo,:""]"#,
            "[:foo,:'']",
            "[:foo,%s||]",
            "[:\"a\nb\",:two]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        assert_eq!(offenses("[:\"a\n\",:two]").len(), 1);
        // Unlike WordArray there is no matrix-wide complex-content exemption.
        assert_eq!(offenses("[[:one,:two],[:'complex content',:two]]").len(), 1);
    }
    #[test]
    fn continuations_preserve_physical_lines_without_becoming_symbol_data() {
        for nl in ["\n", "\r\n"] {
            let s = format!("[:\"one\\{nl}two\", :three]");
            assert_eq!(offenses(&s).len(), 1, "{s}");
            assert_correction(&SymbolArray, &s, &format!("%i[onetwo{nl} three]"));
            assert_correction(
                &SymbolArray,
                &format!("[:one, \\{nl} :two]"),
                &format!("%i[one {nl} two]"),
            );
        }
    }
    #[test]
    fn delimiter_pair_stripping_uses_ruby_ascii_whitespace_including_vertical_tab() {
        for ws in [' ', '\t', '\n', '\r', '\u{b}', '\u{c}'] {
            for (open, close) in [('(', ')'), ('[', ']')] {
                let s = format!("[:foo,:\"{open}{ws}{close}\"]");
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
        assert_eq!(offenses("[:foo,:\"(\u{a0})\"]").len(), 1);
    }
    #[test]
    fn raw_unicode_controls_require_interpreted_percent_syntax() {
        for c in ['\u{85}', '\u{9f}'] {
            assert_correction(
                &SymbolArray,
                &format!("[:\"{c}\",:two]"),
                &format!("%I[\\u{{{:x}}} two]", c as u32),
            );
        }
    }
    #[test]
    fn parenthesis_pairs_can_hide_unbalanced_square_delimiters() {
        for (name, word) in [
            ("([)", r"(\[)"),
            ("(])", r"(\])"),
            ("([[)", r"(\[\[)"),
            ("(][)", r"(\]\[)"),
            ("([x)", r"(\[x)"),
            ("([[])", r"(\[\[\])"),
            ("([])([)", r"(\[\])(\[)"),
            ("([])", "([])"),
            ("([x])", "([x])"),
        ] {
            let source = format!("[:\"{name}\",:two]");
            let expected = format!("%i[{word} two]");
            assert_eq!(offenses(&source).len(), 1, "{source}");
            assert_correction(&SymbolArray, &source, &expected);
            assert!(offenses(&expected).is_empty(), "{expected}");
        }
    }
    #[test]
    fn forced_utf8_marker_preserves_balanced_percent_delimiters() {
        for (source, expected) in [
            (r#"[:"[\u4e2d]", :two]"#, r"%I[[\u{4e2d}] \u{74}wo]"),
            (r#"[:"[]\u4e2d[]", :two]"#, r"%I[[]\u{4e2d}[] \u{74}wo]"),
            (r#"[:"\u4e2d", :"[]"]"#, r"%I[\u{4e2d} []]"),
            // An ASCII prefix must not steal the non-ASCII UTF-8 marker.
            (r#"[:"A\u4e2d\u6587", :two]"#, r"%I[A\u{4e2d}文 \u{74}wo]"),
        ] {
            assert_correction(&SymbolArray, source, expected);
            assert!(offenses(expected).is_empty());
        }
    }
    #[test]
    fn forced_utf8_legacy_sources_escape_every_non_ascii_codepoint() {
        for encoding in ["US-ASCII", "binary", "Windows-31J", "EUC-JP", "ISO-8859-1"] {
            for (source, expected) in [
                (r#"[:"\u{e9 f1}",:two]"#, r"%I[\u{e9}\u{f1} \u{74}wo]"),
                (
                    r#"[:"A\u4e2d\u6587",:two]"#,
                    r"%I[A\u{4e2d}\u{6587} \u{74}wo]",
                ),
                (
                    r#"[:"[\u4e2d\u6587]",:two]"#,
                    r"%I[[\u{4e2d}\u{6587}] \u{74}wo]",
                ),
                (
                    r#"[:"([\u4e2d\u6587)",:two]"#,
                    r"%I[(\[\u{4e2d}\u{6587}) \u{74}wo]",
                ),
                // The same writer handles the inverse percent-to-brackets edit.
                (
                    r"%I[\u{e9 f1}\ two three]",
                    r#"[:"\u{e9}\u{f1} two", :"\u{74}hree"]"#,
                ),
            ] {
                let header = format!("# encoding: {encoding}\n");
                assert_correction(
                    &SymbolArray,
                    &format!("{header}{source}"),
                    &format!("{header}{expected}"),
                );
                assert!(offenses(&format!("{header}{expected}")).is_empty());
            }
        }
    }
    #[test]
    fn cross_delimiter_and_multiglyph_edits_preserve_ast_payload_and_non_ascii_flags() {
        fn symbols(source: &[u8]) -> Vec<(Vec<u8>, bool, bool)> {
            let parsed = crate::parser::parse(source);
            assert_eq!(parsed.errors().count(), 0, "{source:?}");
            #[derive(Default)]
            struct Symbols(Vec<(Vec<u8>, bool, bool)>);
            impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Symbols {
                fn enter(&mut self, n: Node<'pr>) {
                    if let Some(s) = n.as_symbol_node() {
                        self.0.push((
                            s.unescaped().to_vec(),
                            s.is_forced_utf8_encoding(),
                            s.is_forced_binary_encoding(),
                        ));
                    }
                }
            }
            let mut symbols = Symbols::default();
            crate::cops::walk::walk(&mut symbols, &parsed.node());
            symbols.0
        }
        for encoding in [
            "UTF-8",
            "US-ASCII",
            "binary",
            "Windows-31J",
            "EUC-JP",
            "ISO-8859-1",
        ] {
            for fsl in [
                "",
                "# frozen_string_literal: true\n",
                "# frozen_string_literal: false\n",
            ] {
                for nl in ["\n", "\r\n"] {
                    for literal in [
                        r#"[:"([)",:two]"#,
                        r#"[:"(])",:two]"#,
                        r#"[:"([[)",:two]"#,
                        r#"[:"(][)",:two]"#,
                        r#"[:"([x)",:two]"#,
                        r#"[:"\u{e9 f1}",:two]"#,
                        r#"[:"A\u4e2d\u6587",:two]"#,
                        r#"[:"[\u4e2d\u6587]",:two]"#,
                        r#"[:"([\u4e2d\u6587)",:two]"#,
                        r"%I[\u{e9 f1}\ two three]",
                    ] {
                        let source =
                            format!("# encoding: {encoding}\n{fsl}x={literal}\ny=[:three,:four]\n")
                                .replace('\n', nl);
                        let offenses = correction_offenses(&SymbolArray, &source);
                        assert_eq!(offenses.len(), 2, "{source}");
                        assert!(offenses.iter().all(|o| o.correction.is_some()), "{source}");
                        let after = crate::corrections::apply(source.as_bytes(), &offenses)
                            .unwrap()
                            .bytes;
                        let before_values = symbols(source.as_bytes());
                        let after_values = symbols(&after);
                        assert_eq!(before_values.len(), after_values.len());
                        for (before, after) in before_values.iter().zip(&after_values) {
                            assert_eq!(before.0, after.0, "{source}");
                            // Prism can propagate explicit_encoding to following
                            // bare ASCII symbols, but resets it at a new %I.
                            // Ruby always downgrades these interned names to
                            // US-ASCII; runtime tests check that identity too.
                            // For non-ASCII payloads the forced flags are real
                            // encoding evidence and must match exactly.
                            if !before.0.is_ascii() {
                                assert_eq!(before, after, "{source}");
                            }
                        }
                        assert!(
                            after.starts_with(format!("# encoding: {encoding}{nl}").as_bytes())
                        );
                        assert_eq!(
                            after.iter().filter(|&&b| b == b'\n').count(),
                            source.bytes().filter(|&b| b == b'\n').count()
                        );
                        if encoding != "UTF-8" {
                            assert!(after.is_ascii(), "{source}: {after:?}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn original_source_columns_survive_unicode_and_normalizing_encoding_prefixes() {
        for source in [
            "x=\"é\"; [:one,:two]",
            "# encoding: UTF8-MAC\nx=\"e\u{301}\"; [:one,:two]",
        ] {
            let o = offenses(source);
            assert_eq!(o.len(), 1);
            assert_eq!(
                (o[0].line, o[0].col),
                if source.starts_with('#') {
                    (2, 9)
                } else {
                    (1, 8)
                }
            );
        }
    }
    #[test]
    fn command_looking_interpolation_is_only_parsed_and_never_rebuilt() {
        for s in [
            r#"%I[foo\ bar#{`touch /tmp/not-executed-task132`} baz]"#,
            r#"%I[foo\ bar#{%x[exit 99]} baz]"#,
        ] {
            assert_eq!(crate::parser::parse(s.as_bytes()).errors().count(), 0);
            let o = correction_offenses(&SymbolArray, s);
            assert_eq!(o.len(), 1);
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_bracket_diagnostic() {
        let o = offenses("# header\n  [:one, :two, :three]\n");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
        assert_eq!(o[0].message, "Use `%i` or `%I` for an array of symbols.");
    }
    #[test]
    fn pinned_accepts_fixed_threshold_dynamic_splats_and_blocks() {
        for s in [
            "[]",
            "[:one]",
            "[:one, :two, \"three\"]",
            "%i(one two three)",
            "%i[zero (one) [two] three[4] five[six] seven(8) nine(ten) ([]) [] ()]",
            "%I[one_#{two} three #{four}_five six#{seven}eight [nine_#{ten}]]",
            "[:one, :two, :\"space here\"]",
            "[:one, :\")\", :two, :\"(\", :\"]\"]",
            "[:one, :two, :\"[ ]\", :\"( )\"]",
            "%i(one [ ] two)",
            "%i[one ( ) two]",
            "[:one,*other]",
            "[:one,:\"#{effect}\"]",
            "foo [:bar,:baz] { qux }",
            "[:one, # comment\n :two]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn all_applicable_pinned_correction_calls_at_fixed_spelling() {
        for (s, want) in [
            ("[:one, :two, :three]", "%i[one two three]"),
            ("[:\"\t\", :\"two\n\", :three]", "%I[\\t two\\n\n three]"),
            ("[:one,\n:two, :three,\n:four]", "%i[one\ntwo three\nfour]"),
            (r#"[:"\t", :"\n", :three]"#, r#"%I[\t \n three]"#),
            ("foo([:bar, :baz]) { qux }", "foo(%i[bar baz]) { qux }"),
            (
                "[:one, :two, :three]\n%i(a b c d)",
                "%i[one two three]\n%i(a b c d)",
            ),
            (
                "[:one, :two, :three]\n%i(a b)",
                "%i[one two three]\n%i(a b)",
            ),
            (r"%i[one \[ \] two]", r#"[:one, :"[", :"]", :two]"#),
            (
                r"%i[one two \[three\ four\ five\]]",
                r#"[:one, :two, :"[three four five]"]"#,
            ),
            (r"%i[one two \[\[\]]", r#"[:one, :two, :"[[]"]"#),
            (r"%i(one \( \) two)", r#"[:one, :"(", :")", :two]"#),
            (r"%i(one two \(\(\))", r#"[:one, :two, :"(()"]"#),
            (
                r"%i(one two \(three\ four\ five\))",
                r#"[:one, :two, :"(three four five)"]"#,
            ),
            ("[\n:foo,\n:bar,\n:baz\n]", "%i[\nfoo\nbar\nbaz\n]"),
            (
                "[:foo, :bar, :baz,\n:boz, :buz,\n:biz]",
                "%i[foo bar baz\nboz buz\nbiz]",
            ),
            ("[:a, :'b[]', :'c[][]']", "%i[a b[] c[][]]"),
        ] {
            assert_correction(&SymbolArray, s, want);
            assert!(offenses(want).is_empty(), "{want}");
        }
    }
    #[test]
    fn ordered_exact_delimiter_exemption_and_pair_stripping() {
        for s in [
            "%i([ foo\\ bar)",
            "%i(] foo\\ bar)",
            "%i[( foo\\ bar]",
            "%i[) foo\\ bar]",
            "%i[[] ()]",
            "[:'[]', :'()']",
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), usize::from(s.starts_with('[')), "{s}: {o:?}");
        }
        for s in [
            r"%i[foo\ bar (]",
            r"%i(foo\ bar [)",
            "[:'([)]', :foo]",
            "%I[foo\\ bar#{effect} baz]",
        ] {
            assert_eq!(offenses(s).len(), usize::from(s.starts_with('%')), "{s}");
        }
    }
    #[test]
    fn dynamic_percent_offenses_remain_uncorrected() {
        for s in ["%I[foo\\ bar#{effect} baz]", "%I[foo\\ bar #@value]"] {
            let o = correction_offenses(&SymbolArray, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
}
