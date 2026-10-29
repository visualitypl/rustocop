//! `Style/SymbolLiteral` — prefer plain ASCII word symbols.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SymbolLiteral;
impl Cop for SymbolLiteral {
    fn name(&self) -> &'static str {
        "Style/SymbolLiteral"
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
        let Some(symbol) = node.as_symbol_node() else {
            return;
        };
        let Some(open) = symbol
            .opening_loc()
            .filter(|l| matches!(l.as_slice(), b":\"" | b":'"))
        else {
            return;
        };
        if !symbol
            .closing_loc()
            .is_some_and(|l| matches!(l.as_slice(), b"\"" | b"'"))
        {
            return;
        }
        let Some(value) = symbol.value_loc() else {
            return;
        };
        let bytes = value.as_slice();
        // Upstream's ASCII [A-Za-z_]\\w*: intentionally not the larger Ruby
        // identifier/operator grammar. Source evidence excludes all escapes.
        if !bytes
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
            || !bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            return;
        }
        let loc = symbol.location();
        let (line, col) = file.line_col(open.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use strings for word-like symbol literals.".into(),
            line,
            col,
            end_line: file.last_line_of(symbol.location()),
            correction: cx
                .corrections_requested()
                .then(|| {
                    // Quoted endings are lexical boundaries even without space.
                    // A plain symbol can absorb suffixes/keywords or identifier bytes.
                    if file.bytes.get(loc.end_offset()).is_some_and(|b| {
                        b.is_ascii_alphanumeric() || !b.is_ascii() || b"_?!=".contains(b)
                    }) {
                        return None;
                    }
                    super::literal_spelling::safe_edit(
                        file,
                        cx,
                        Correction {
                            range: loc.start_offset()..loc.end_offset(),
                            replacement: format!(":{}", std::str::from_utf8(bytes).ok()?),
                        },
                    )
                })
                .flatten(),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/symbol_literal_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_word_symbols() {
        let source = "x = { :\"test\" => 0, :\"other\" => 1 }\n";
        let found: Vec<_> = lint_str(source)
            .into_iter()
            .filter(|o| o.cop == SymbolLiteral.name())
            .collect();
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.message.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (1, 7, "Do not use strings for word-like symbol literals."),
                (1, 21, "Do not use strings for word-like symbol literals.")
            ]
        );
        assert_correction(&SymbolLiteral, source, "x = { :test => 0, :other => 1 }\n");
    }
    #[test]
    fn token_boundary_hazards_keep_offenses_without_edits() {
        for source in [
            r#"p :"a"?1:2"#,
            r#"p :"a"!=:b"#,
            r#"p :"a"and true"#,
            r#"p :"a"or false"#,
        ] {
            let found = crate::cops::testing::correction_offenses(&SymbolLiteral, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn single_quotes_keywords_and_ascii_identifier_grammar() {
        for name in [
            "a", "_", "_a", "a0", "A_B", "if", "class", "nil", "BEGIN", "__END__",
        ] {
            let before = format!(":'{name}'");
            assert_correction(&SymbolLiteral, &before, &format!(":{name}"));
        }
    }
    #[test]
    fn accepts_nonwords_labels_and_escaped_payloads() {
        for source in [
            r#"p :"t o""#,
            r#"p :"\tab""#,
            r#"p :"1""#,
            r#"p :"foo?""#,
            r#"p :"foo!""#,
            r#"p :"foo=""#,
            r#"p :"+""#,
            r#"p :"\x61""#,
            r#"p :"café""#,
            "p %s[foo]",
            "p %i[foo bar]",
            "p %I[foo #@bar]",
            "p :foo",
            "p 'foo': 1",
            "p \"foo\": 1",
        ] {
            assert!(
                lint_str(source)
                    .iter()
                    .all(|o| o.cop != SymbolLiteral.name()),
                "{source}"
            );
        }
    }
}
