//! `Style/RedundantCapitalW` — use noninterpolating word arrays when possible.
use super::literal_spelling::{character_heads, double_quotes_required, safe_edit};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct RedundantCapitalW;
impl Cop for RedundantCapitalW {
    fn name(&self) -> &'static str {
        "Style/RedundantCapitalW"
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
        let Some(opening) = array
            .opening_loc()
            .filter(|l| l.as_slice().starts_with(b"%W"))
        else {
            return;
        };
        if array.elements().iter().any(|n| {
            n.as_string_node().is_none_or(|s| {
                double_quotes_required(character_heads(
                    file,
                    s.content_loc().start_offset()..s.content_loc().end_offset(),
                ))
            })
        }) {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Detection follows the pin's syntactic policy; correction separately
                // proves each %w element's bytes against the MAIN parse payload.
                // Only paired backslashes can survive the detector. An allowed \"
                // escape would retain a backslash in %w, so keep its offense unedited.
                for element in array.elements().iter() {
                    let string = element.as_string_node()?;
                    let content = string.content_loc();
                    let range = content.start_offset()..content.end_offset();
                    let mut decoded = Vec::with_capacity(content.as_slice().len());
                    let mut characters = file.character_offsets(range.clone()).peekable();
                    while let Some(start) = characters.next() {
                        let byte = file.bytes[start];
                        // Consume only real ASCII escapes/CRLF, never a trailing
                        // byte of a multibyte source character.
                        if let Some(&next) = characters.peek()
                            && ((byte == b'\\' && file.bytes[next] == b'\\')
                                || (byte == b'\r' && file.bytes[next] == b'\n'))
                        {
                            characters.next();
                            decoded.push(file.bytes[next]);
                        } else {
                            let end = characters.peek().copied().unwrap_or(range.end);
                            decoded.extend_from_slice(&file.bytes[start..end]);
                        }
                    }
                    if decoded != string.unescaped()
                        || string.is_forced_utf8_encoding()
                        || string.is_forced_binary_encoding()
                    {
                        return None;
                    }
                }
                safe_edit(
                    file,
                    cx,
                    Correction {
                        range: opening.start_offset() + 1..opening.start_offset() + 2,
                        replacement: "w".into(),
                    },
                )
            })
            .flatten();
        let (line, col) = file.line_col(opening.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use `%W` unless interpolation is needed. If not, use `%w`.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/redundant_capital_w_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_diagnostic_and_corrections() {
        let found: Vec<_> = lint_str("# heading\n  %W(cat dog)\n")
            .into_iter()
            .filter(|o| o.cop == RedundantCapitalW.name())
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Do not use `%W` unless interpolation is needed. If not, use `%w`."
        );
        for (before, after) in [
            ("%W(cat dog)\n", "%w(cat dog)\n"),
            ("%W[cat dog]\n", "%w[cat dog]\n"),
        ] {
            assert_correction(&RedundantCapitalW, before, after);
        }
    }
    #[test]
    fn pinned_and_escape_lookalikes() {
        for source in [
            r#"["one", "two", "three"]"#,
            r#"["one", "two", "th#{?r}ee"]"#,
            "%W(c#{?a}t dog)",
            "%w(cat dog)",
            "%w(c#{?a}t dog)",
            "[\"one\", ?\n]",
            r#"["one space", "two", "three"]"#,
            r#"["one", "two", 3]"#,
            r#"["three"]"#,
            r#"["", "two", "three"]"#,
            "%W(it's fine)",
            "%W(#@foo)",
            "%W(#$foo)",
            r"%W(a\ b)",
            "%W(a\\\nb)",
        ] {
            assert!(
                correction_offenses(&RedundantCapitalW, source).is_empty(),
                "{source}"
            );
        }
        for escape in [
            r"\000", r"\001", r"\027", r"\002", r"\003", r"\004", r"\005", r"\006", r"\007",
            r"\00", r"\a", r"\s", r"\n", r"\!", r"\x41", r"\u{41}",
        ] {
            let source = format!("%W({escape})");
            assert!(
                correction_offenses(&RedundantCapitalW, &source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn empty_multiline_and_backslashes() {
        for (before, after) in [
            ("%W()", "%w()"),
            ("%W( )", "%w( )"),
            ("%W(a\r\nb)", "%w(a\r\nb)"),
            (r"%W(a\\b c)", r"%w(a\\b c)"),
            ("%W(\"hi\" there)", "%w(\"hi\" there)"),
        ] {
            assert_correction(&RedundantCapitalW, before, after);
        }
    }
    #[test]
    fn escaped_double_quote_is_reported_but_not_lowered_lossily() {
        let found = correction_offenses(&RedundantCapitalW, r#"%W(\"hi\")"#);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
}
