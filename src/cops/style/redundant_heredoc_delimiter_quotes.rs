//! `Style/RedundantHeredocDelimiterQuotes` — redundant string heredoc quotes.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantHeredocDelimiterQuotes;
impl Cop for RedundantHeredocDelimiterQuotes {
    fn name(&self) -> &'static str {
        "Style/RedundantHeredocDelimiterQuotes"
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
        let (opening, closing, body) = if let Some(s) = node.as_string_node() {
            (
                s.opening_loc(),
                s.closing_loc(),
                Some(s.content_loc().start_offset()),
            )
        } else if let Some(s) = node.as_interpolated_string_node() {
            (
                s.opening_loc(),
                s.closing_loc(),
                s.parts().iter().next().map(|n| n.location().start_offset()),
            )
        } else {
            return;
        };
        let Some(opening) = opening.filter(|l| l.as_slice().starts_with(b"<<")) else {
            return;
        };
        let raw = opening.as_slice();
        let prefix = 2 + usize::from(matches!(raw.get(2), Some(b'~' | b'-')));
        let Some(&quote @ (b'\'' | b'"')) = raw.get(prefix) else {
            return;
        };
        let Some(closing) = closing else {
            return;
        };
        // Match the pin's stripped closing-label word policy for detection only.
        // Correction separately requires the actual opening label, untrimmed.
        let label = closing.as_slice().trim_ascii();
        if label
            .iter()
            .any(|b| !b.is_ascii_alphanumeric() && *b != b'_')
        {
            return;
        }
        if quote == b'\''
            && body.is_some_and(|body| {
                file.character_offsets(body..closing.start_offset())
                    .any(|i| {
                        let b = file.bytes[i];
                        // ASCII interpolation markers have single-byte widths. Look at
                        // raw neighbors only after a real character-head '#'.
                        b == b'\\'
                            || b == b'#'
                                && matches!(file.bytes.get(i + 1), Some(b'{' | b'@' | b'$'))
                    })
            })
        {
            return;
        }
        let replacement = format!(
            "{}{}",
            String::from_utf8_lossy(&raw[..prefix]),
            String::from_utf8_lossy(label)
        );
        let correction = if cx.corrections_requested()
            && raw.get(prefix + 1..raw.len() - 1) == Some(label)
            && label
                .first()
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            super::literal_spelling::safe_edit(
                file,
                cx,
                Correction {
                    range: opening.start_offset()..opening.end_offset(),
                    replacement: replacement.clone(),
                },
            )
        } else {
            None
        };
        let (line, col) = file.line_col(opening.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(opening),
            message: format!(
                "Remove the redundant heredoc delimiter quotes, use `{replacement}` instead."
            ),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_heredoc_delimiter_quotes_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_diagnostics_and_all_corrections() {
        for (op, quote, body) in [
            ("<<~", "'", "  no string interpolation style text\n"),
            ("<<-", "'", "  no string interpolation style text\n"),
            ("<<", "'", "  no string interpolation style text\n"),
            ("<<~", "\"", "  no string interpolation style text\n"),
            ("<<~", "\"", "  #{string} #{interpolation}\n"),
        ] {
            let before = format!("do_something({op}{quote}EOS{quote})\n{body}EOS\n");
            let after = format!("do_something({op}EOS)\n{body}EOS\n");
            let found: Vec<_> = lint_str(&before)
                .into_iter()
                .filter(|o| o.cop == RedundantHeredocDelimiterQuotes.name())
                .collect();
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 14, 1));
            assert_eq!(
                found[0].message,
                format!("Remove the redundant heredoc delimiter quotes, use `{op}EOS` instead.")
            );
            assert_correction(&RedundantHeredocDelimiterQuotes, &before, &after);
        }
    }
    #[test]
    fn pinned_lookalikes() {
        for source in [
            "<<~`EOS`\ncommand\nEOS\n",
            "<<~EOS\ntext\nEOS\n",
            "<<~'EOS'\n#{string} #{interpolation}\nEOS\n",
            "<<~'EOS'\n#{\nstring\n}\nEOS\n",
            "<<~'EOS'\n#@foo\nEOS\n",
            "<<~'EOS'\n#@@foo\nEOS\n",
            "<<~'EOS'\n#$foo\nEOS\n",
            "<<~'EDGE\"CASE'\ntext\nEDGE\"CASE\n",
            "<<~\"EDGE'CASE\"\ntext\nEDGE'CASE\n",
            "<<~'EDGE CASE'\ntext\nEDGE CASE\n",
            "<<~'EDGE　CASE'\ntext\nEDGE　CASE\n",
            "<<~'EOS'\nPreserve \\\nnewlines\nEOS\n",
        ] {
            assert!(
                correction_offenses(&RedundantHeredocDelimiterQuotes, source).is_empty(),
                "{source}"
            );
        }
    }
}
