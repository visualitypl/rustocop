//! `Style/EmptyHeredoc` — raw-empty string heredocs, never command heredocs.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EmptyHeredoc;
impl Cop for EmptyHeredoc {
    fn name(&self) -> &'static str {
        "Style/EmptyHeredoc"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_string_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(string) = node.as_string_node() else {
            return;
        };
        let Some(opening) = string
            .opening_loc()
            .filter(|l| l.as_slice().starts_with(b"<<"))
        else {
            return;
        };
        if !string.content_loc().as_slice().is_empty() {
            return;
        }
        let Some(closing) = string.closing_loc() else {
            return;
        };
        let start = opening.start_offset();
        let end = closing.end_offset();
        let closing_start = file.line_start(file.line_of(closing.start_offset()));
        let correction = cx
            .corrections_requested()
            .then(|| {
                // One envelope keeps all header tails, comments, queued siblings and
                // both physical edits atomic. Bound work before copying interior text.
                if end - start > 4096 || super::correction::touches_directive(file, start..end) {
                    return None;
                }
                let after_line = file.line_of(end);
                if (after_line..=after_line + 1)
                    .any(|line| !super::correction::comment_relocation_safe(file.line_text(line)))
                {
                    return None;
                }
                let tail =
                    std::str::from_utf8(&file.bytes[opening.end_offset()..closing_start]).ok()?;
                super::literal_spelling::safe_edit(
                    file,
                    cx,
                    Correction {
                        range: start..end,
                        replacement: format!("\"\"{tail}"),
                    },
                )
            })
            .flatten();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use an empty string literal instead of heredoc.".into(),
            line,
            col,
            end_line: line,
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/empty_heredoc_spec.rb (MIT, Copyright
    //! (c) 2012-26 Bozhidar Batsov). Fixed double-quote policy.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_diagnostics_and_all_corrections() {
        for op in ["<<~", "<<-", "<<"] {
            for (prefix, suffix, col) in [
                ("", "", 1),
                ("do_something(", ")", 14),
                ("do_something(", ", arg)", 14),
            ] {
                let before = format!("{prefix}{op}EOS{suffix}\nEOS\n");
                let found: Vec<_> = lint_str(&before)
                    .into_iter()
                    .filter(|o| o.cop == EmptyHeredoc.name())
                    .collect();
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, col, 1)
                );
                assert_eq!(
                    found[0].message,
                    "Use an empty string literal instead of heredoc."
                );
                assert_correction(&EmptyHeredoc, &before, &format!("{prefix}\"\"{suffix}\n"));
            }
        }
    }
    #[test]
    fn raw_not_decoded_empty_and_commands_are_accepted() {
        for source in [
            "<<~EOS\n  Hello.\nEOS\n",
            "<<~EOS\n\nEOS\n",
            "<<~EOS\n  \nEOS\n",
            "<<~EOS\n\\\nEOS\n",
            "foo(<<~`SH`)\nSH\n",
            "<<~EOS\n#{''}\nEOS\n",
        ] {
            assert!(
                correction_offenses(&EmptyHeredoc, source).is_empty(),
                "{source}"
            );
        }
    }
}
