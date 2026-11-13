//! `Style/QuotedSymbols` — fixed double quotes, inheriting StringLiterals' policy.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct QuotedSymbols;
impl Cop for QuotedSymbols {
    fn name(&self) -> &'static str {
        "Style/QuotedSymbols"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_symbol_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(symbol) = node.as_symbol_node() else {
            return;
        };
        let Some(open) = symbol
            .opening_loc()
            .filter(|l| matches!(l.as_slice(), b":'" | b"'"))
        else {
            return;
        };
        let Some(close) = symbol
            .closing_loc()
            .filter(|l| matches!(l.as_slice(), b"'" | b"':"))
        else {
            return;
        };
        let range = open.end_offset()..close.start_offset();
        let raw = &file.bytes[range.clone()];
        // Parser-gem dispatches empty and physically split standalone symbols
        // as dsym, not sym. A final newline alone remains a sym. Quoted LABELS
        // remain sym even when empty or multiline.
        if open.as_slice().starts_with(b":")
            && (raw.is_empty() || raw[..raw.len() - 1].contains(&b'\n'))
        {
            return;
        }
        let heads: Vec<_> = super::literal_spelling::character_heads(file, range.clone()).collect();
        #[cfg(test)]
        SCANNED_HEADS.with(|n| n.set(n.get() + heads.len()));
        // Union of upstream wrong_quotes? and invalid_double_quotes?: the
        // latter permits harmless/unknown escapes, but never real escapes or
        // interpolation. Character boundaries exclude legacy trailing bytes.
        let unsafe_quotes = heads.iter().enumerate().any(|(i, &b)| {
            b == b'"'
                || (b == b'#' && matches!(heads.get(i + 1), Some(b'@' | b'$' | b'{')))
                || (b == b'\\'
                    && i.checked_sub(1).is_none_or(|p| heads[p] != b'\\')
                    && heads
                        .get(i + 1)
                        .is_some_and(|b| b"aAbcdefkMnprsStuUxzZ01234567".contains(b)))
        });
        if unsafe_quotes {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                super::literal_spelling::safe_edit(
                    file,
                    cx,
                    quoted_correction(file, &open, &close)?,
                )
            })
            .flatten();
        let (line, col) = file.line_col(open.start_offset());
        out.push(Offense {
            cop: self.name(), severity: self.severity(),
            message: "Prefer double-quoted symbols unless you need single quotes to avoid extra backslashes for escaping.".into(),
            line, col, end_line: file.last_line_of(symbol.location()), correction,
        });
    }
}

/// Convert only single-quote escape rules at encoded character boundaries.
/// Preserve physical newlines and double unknown backslashes: simply changing
/// quotes (as upstream does for e.g. \\q) would lose decoded bytes. No candidate
/// parsing, dynamic evaluation or whole-source scans are needed.
fn quoted_correction(
    file: &SourceFile,
    open: &ruby_prism::Location<'_>,
    close: &ruby_prism::Location<'_>,
) -> Option<Correction> {
    #[cfg(test)]
    CORRECTION_BUILDS.with(|n| n.set(n.get() + 1));
    let range = open.end_offset()..close.start_offset();
    let mut replacement = Vec::with_capacity(range.len() + 4);
    if open.as_slice().starts_with(b":") {
        replacement.push(b':');
    }
    replacement.push(b'"');
    let mut offsets = file.character_offsets(range.clone()).peekable();
    while let Some(i) = offsets.next() {
        let end = offsets.peek().copied().unwrap_or(range.end);
        if file.bytes[i] == b'\\' {
            match offsets.peek().map(|&n| file.bytes[n]) {
                Some(b'\'') => {
                    offsets.next();
                    replacement.push(b'\'');
                }
                Some(b'\\') => {
                    offsets.next();
                    replacement.extend_from_slice(b"\\\\");
                }
                _ => replacement.extend_from_slice(b"\\\\"),
            }
        } else {
            replacement.extend_from_slice(&file.bytes[i..end]);
        }
    }
    replacement.push(b'"');
    if close.as_slice().ends_with(b":") {
        replacement.push(b':');
    }
    Some(Correction {
        range: open.start_offset()..close.end_offset(),
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
thread_local! {
    static SCANNED_HEADS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CORRECTION_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/quoted_symbols_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov), matching double_quotes settings.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_diagnostic_and_all_distinct_correction_pairs() {
        let found: Vec<_> = lint_str("# heading\n  :'a'\n")
            .into_iter()
            .filter(|o| o.cop == QuotedSymbols.name())
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Prefer double-quoted symbols unless you need single quotes to avoid extra backslashes for escaping."
        );
        for (before, after) in [
            (":'a'", ":\"a\""),
            (r":'o\'clock'", r#":"o'clock""#),
            (r":'foo\\bar'", r#":"foo\\bar""#),
            ("{ :'a' => value }", "{ :\"a\" => value }"),
        ] {
            assert_correction(&QuotedSymbols, before, after);
        }
    }
    #[test]
    fn pinned_lookalikes() {
        for source in [
            ":a",
            ":\"a\"",
            r##":"#{a}""##,
            r##":"#@test""##,
            r##":"#$test""##,
            r##":"#@@test""##,
            r#":"a\nb""#,
            r#":'"'"#,
            r#":"'""#,
            ":'a\n  bc'",
            r#":"my\"quote""#,
            r#":'\n'"#,
            r#":'\u0041'"#,
            r#":'#{x}'"#,
            r#":'#@x'"#,
            r#":'#$x'"#,
            ":''",
            ":\"\"",
            "%s[hi]",
            "%i[hi]",
            "{ :\"a\" => value }",
        ] {
            assert!(
                lint_str(source)
                    .iter()
                    .all(|o| o.cop != QuotedSymbols.name()),
                "{source}"
            );
        }
    }
    #[test]
    fn source_work_is_local_linear_and_correction_generation_is_lazy() {
        let source = ":'abc-def'\n".repeat(400);
        SCANNED_HEADS.with(|n| n.set(0));
        CORRECTION_BUILDS.with(|n| n.set(0));
        assert_eq!(
            lint_str(&source)
                .iter()
                .filter(|o| o.cop == QuotedSymbols.name())
                .count(),
            400
        );
        SCANNED_HEADS.with(|n| assert_eq!(n.get(), 400 * 7));
        CORRECTION_BUILDS.with(|n| assert_eq!(n.get(), 0));
        SCANNED_HEADS.with(|n| n.set(0));
        let found = crate::cops::testing::correction_offenses(&QuotedSymbols, &source);
        assert_eq!(found.iter().filter(|o| o.correction.is_some()).count(), 400);
        SCANNED_HEADS.with(|n| assert_eq!(n.get(), 400 * 7));
        CORRECTION_BUILDS.with(|n| assert_eq!(n.get(), 400));
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let facts = file.class_method_bindings(&root);
        assert!(std::ptr::eq(facts, file.class_method_bindings(&root)));
        assert!(facts.visits < 400 * 4);
        assert!(facts.observer_visits < 400 * 4);
    }
    #[test]
    fn unknown_escape_corrections_preserve_decoded_backslashes() {
        for (before, after) in [
            (r":'a\v'", r#":"a\\v""#),
            (r":'a\8'", r#":"a\\8""#),
            (r":'a\\\n'", r#":"a\\\\n""#),
        ] {
            assert_correction(&QuotedSymbols, before, after);
        }
    }
    #[test]
    fn quoted_labels_and_nonword_payloads() {
        for (before, after) in [
            ("p 'a-b': 1", "p \"a-b\": 1"),
            ("p '': 1", "p \"\": 1"),
            ("p 'a\nb': 1", "p \"a\nb\": 1"),
            (":'abc-def'", ":\"abc-def\""),
            (":'café'", ":\"café\""),
            (":'a\r\n'", ":\"a\r\n\""),
            (r":'a\q'", r#":"a\\q""#),
        ] {
            assert_correction(&QuotedSymbols, before, after);
        }
    }
}
