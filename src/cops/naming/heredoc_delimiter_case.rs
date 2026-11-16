//! `Naming/HeredocDelimiterCase` — fixed uppercase heredoc labels.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct HeredocDelimiterCase;
impl Cop for HeredocDelimiterCase {
    fn name(&self) -> &'static str {
        "Naming/HeredocDelimiterCase"
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
        let Some(h) = super::heredoc::Heredoc::of(node, file) else {
            return;
        };
        let label = &file.bytes[h.delimiter_range(file)];
        if !file.identifier_uppercase_changes(label) {
            return;
        }
        if let Some(upper) = file.heredoc_uppercase(label)
            && file.bytes[h.body.clone()]
                .split(|&b| b == b'\n')
                .any(|line| {
                    file.heredoc_normalized(super::heredoc::strip(line))
                        .as_ref()
                        == upper
                })
        {
            return;
        }
        // RuboCop upcases the opening and the terminator's delimiter.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let opening = std::str::from_utf8(h.opening.as_slice()).ok()?;
                let delimiter = std::str::from_utf8(label).ok()?;
                let closing = h.closing.as_slice();
                let eol =
                    usize::from(closing.ends_with(b"\n")) + usize::from(closing.ends_with(b"\r\n"));
                let end = h.closing.end_offset() - eol;
                let start = end.checked_sub(label.len())?;
                (&file.bytes[start..end] == label).then_some(())?;
                crate::cops::style::correction::combine(
                    file,
                    vec![
                        Correction {
                            range: h.opening.start_offset()..h.opening.end_offset(),
                            replacement: opening.to_uppercase(),
                        },
                        Correction {
                            range: start..end,
                            replacement: delimiter.to_uppercase(),
                        },
                    ],
                )
            })
            .flatten();
        let (line, col) = file.line_col(h.closing.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use uppercase heredoc delimiters.".into(),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop naming/heredoc_delimiter_case_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_uppercase_forms_and_exact_corrections() {
        for op in ["<<", "<<-", "<<~"] {
            for quote in ["", "'", "\"", "`"] {
                for label in ["sql", "Sql"] {
                    let source = format!("{op}{quote}{label}{quote}\n  foo\n{label}\n");
                    let found: Vec<_> = lint_str(&source)
                        .into_iter()
                        .filter(|o| o.cop == HeredocDelimiterCase.name())
                        .collect();
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
                    assert_eq!(found[0].message, "Use uppercase heredoc delimiters.");
                    assert_correction(
                        &HeredocDelimiterCase,
                        &source,
                        &source.replace(label, "SQL"),
                    );
                }
            }
        }
    }
    #[test]
    fn opening_capture_retries_after_quotes_and_backtracks_optional_modifier() {
        for label in ["\"<<sql", "`<<sql", "\"<<~sql", "\"<<-sql"] {
            let source = format!("p <<'{label}'\nbody\n{label}\n");
            let found = crate::cops::testing::correction_offenses(&HeredocDelimiterCase, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1));
            assert_eq!(found[0].message, "Use uppercase heredoc delimiters.");
        }
        for source in ["p <<~'\"sql'\nbody\n\"sql\n", "p <<'\"sql'\nbody\n\"sql\n"] {
            assert!(
                lint_str(source)
                    .iter()
                    .all(|o| o.cop != HeredocDelimiterCase.name()),
                "{source}"
            );
        }
    }

    #[test]
    fn utf8_mac_partial_compositions_suppress_uppercase_body_collisions() {
        for name in ["UTF8-MAC", "UTF-8-HFS"] {
            for (label, body) in [("å\u{301}", "Ǻ"), ("ǻ", "Å\u{301}")] {
                let source = format!("# coding: {name}\np <<~'{label}'\n{body}\n{label}\n");
                assert!(
                    lint_str(&source)
                        .iter()
                        .all(|o| o.cop != HeredocDelimiterCase.name()),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn uppercase_nonword_collisions_and_lookalikes() {
        for source in [
            "<<~SQL\nfoo\nSQL\n",
            "<<~'+'\nfoo\n+\n",
            "<<~sql\n  SQL\nsql\n",
            "'<<~sql'\n# <<~sql\n",
            "array << value\n",
        ] {
            assert!(
                lint_str(source)
                    .iter()
                    .all(|o| o.cop != HeredocDelimiterCase.name()),
                "{source}"
            );
        }
    }
}
