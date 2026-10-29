//! `Naming/HeredocDelimiterNaming` — meaningful fixed-policy labels, no edits.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct HeredocDelimiterNaming;
impl Cop for HeredocDelimiterNaming {
    fn name(&self) -> &'static str {
        "Naming/HeredocDelimiterNaming"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(h) = super::heredoc::Heredoc::of(node, file) else {
            return;
        };
        let label = &file.bytes[h.delimiter_range(file)];
        // Ruby /\\w/ and /\\s/ here are ASCII, not POSIX Unicode classes.
        // Only character heads may supply ASCII in legacy encodings.
        let word = file.heredoc_has_ascii_word(label);
        let forbidden = label
            .split(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c))
            .any(|part| {
                part.eq_ignore_ascii_case(b"END")
                    || part.len() >= 3
                        && part[..2].eq_ignore_ascii_case(b"EO")
                        && file.heredoc_ascii_letter(&part[2..])
            });
        if word && !forbidden {
            return;
        }
        let location = if h.empty { &h.opening } else { &h.closing };
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use meaningful heredoc delimiters.".into(),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop naming/heredoc_delimiter_naming_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    #[test]
    fn forbidden_and_nonword_labels_have_exact_diagnostics_and_no_edits() {
        for op in ["<<", "<<-", "<<~"] {
            for quote in ["'", "\"", "`"] {
                for label in ["END", "end", "EOS", "eoF", "+", "!", " x END y "] {
                    let source = format!("{op}{quote}{label}{quote}\n  foo\n{label}\n");
                    let found: Vec<_> = lint_str(&source)
                        .into_iter()
                        .filter(|o| o.cop == HeredocDelimiterNaming.name())
                        .collect();
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (3, 1));
                    assert_eq!(found[0].message, "Use meaningful heredoc delimiters.");
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn opening_capture_retries_after_quotes_and_backtracks_optional_modifier() {
        for label in ["\"<<sql", "`<<sql", "\"<<~sql", "\"<<-sql"] {
            let source = format!("p <<'{label}'\nbody\n{label}\n");
            assert!(
                lint_str(&source)
                    .iter()
                    .all(|o| o.cop != HeredocDelimiterNaming.name()),
                "{source}"
            );
        }
        for op in ["<<", "<<~", "<<-"] {
            let source = format!("p {op}'\"sql'\nbody\n\"sql\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == HeredocDelimiterNaming.name())
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1));
            assert_eq!(found[0].message, "Use meaningful heredoc delimiters.");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn fixed_pattern_is_not_a_substring_or_unicode_word_rule() {
        for label in [
            "SQL", "SQL_CODE", "BASE64", "ENDING", "EOF_X", "aEND", "EO", "EOXX", "x-END-y",
            "endé", "éEND",
        ] {
            let source = format!("<<~'{label}'\nfoo\n{label}\n");
            assert!(
                lint_str(&source)
                    .iter()
                    .all(|o| o.cop != HeredocDelimiterNaming.name()),
                "{source}"
            );
        }
    }
}
