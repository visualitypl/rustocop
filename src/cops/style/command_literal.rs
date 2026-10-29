//! `Style/CommandLiteral` — backticks, except when the raw body contains backticks.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct CommandLiteral;
impl Cop for CommandLiteral {
    fn name(&self) -> &'static str {
        "Style/CommandLiteral"
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
        let (opening, closing) = if let Some(n) = node.as_x_string_node() {
            (n.opening_loc(), n.closing_loc())
        } else if let Some(n) = node.as_interpolated_x_string_node() {
            (n.opening_loc(), n.closing_loc())
        } else {
            return;
        };
        let open = opening.as_slice();
        if open.starts_with(b"<<") {
            return;
        }
        let inner = file.command_body_has_backtick(opening.end_offset()..closing.start_offset());
        let backticks = open == b"`";
        if backticks != inner {
            return;
        }
        let (line, col) = file.line_col(opening.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: if backticks {
                "Use `%x` around command string."
            } else {
                "Use backticks around command string."
            }
            .into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: cx
                .corrections_requested()
                .then(|| {
                    // Inner backticks are reported, never rewritten (upstream too).
                    // Ordinary %x and backticks share interpolation/escape rules,
                    // including %x'...'. Preserve every interior byte. Newline and
                    // backslash delimiters need distinct lexical proofs; withhold.
                    if inner
                        || open.len() != 3
                        || !open.starts_with(b"%x")
                        || !open[2].is_ascii_punctuation()
                        || open[2] == b'\\'
                    {
                        return None;
                    }
                    // Only the outermost eligible percent command assembles an
                    // envelope this pass. Nested bodies otherwise cause quadratic
                    // copying; their nonoverlapping edits can follow next pass.
                    if cx.ancestors().any(|a| {
                        let locs = a
                            .as_interpolated_x_string_node()
                            .map(|n| (n.opening_loc(), n.closing_loc()));
                        locs.is_some_and(|(o, c)| {
                            o.as_slice().starts_with(b"%x")
                                && !file.command_body_has_backtick(o.end_offset()..c.start_offset())
                        })
                    }) {
                        return None;
                    }
                    let edit = super::correction::combine(
                        file,
                        vec![
                            Correction {
                                range: opening.start_offset()..opening.end_offset(),
                                replacement: "`".into(),
                            },
                            Correction {
                                range: closing.start_offset()..closing.end_offset(),
                                replacement: "`".into(),
                            },
                        ],
                    )?;
                    super::literal_spelling::safe_edit(file, cx, edit)
                })
                .flatten(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/style/command_literal_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/CommandLiteral")
            .collect()
    }
    #[test]
    fn pinned_percent_diagnostics_and_corrections() {
        for (before, after, line, col, end_line) in [
            ("%x$ls$", "`ls`", 1, 1, 1),
            ("foo = %x(ls)", "foo = `ls`", 1, 7, 1),
            (
                "foo = %x(\n  ls\n  ls -l\n)",
                "foo = `\n  ls\n  ls -l\n`",
                1,
                7,
                4,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line)
            );
            assert_eq!(found[0].message, "Use backticks around command string.");
            assert_correction(&CommandLiteral, before, after);
        }
    }
    #[test]
    fn pinned_inner_backtick_diagnostics_without_corrections() {
        for source in [
            "foo = `echo \\`ls\\``",
            "foo = `\n  echo \\`ls\\`\n  echo \\`ls -l\\`\n`",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 7));
            assert_eq!(found[0].message, "Use `%x` around command string.");
            assert_correction(&CommandLiteral, source, source);
        }
    }
    #[test]
    fn raw_policy_includes_interpolation_code_not_decoded_backticks() {
        for (source, count) in [
            ("`#{\"`\"}`", 1),
            ("%x(#{\"`\"})", 0),
            (r"`\u0060`", 0),
            (r"%x(\u0060)", 1),
            ("%x(#{`inner`})", 0),
            ("%x(#{%x(inner)})", 2),
        ] {
            assert_eq!(offenses(source).len(), count, "{source}");
        }
        assert_correction(&CommandLiteral, r"%x(\u0060)", r"`\u0060`");
    }
    #[test]
    fn empty_unicode_and_scalar_columns() {
        let found = offenses("p \"é\"; %x()\n");
        assert_eq!((found[0].line, found[0].col), (1, 8));
        assert_correction(&CommandLiteral, "%x()", "``");
        assert_correction(&CommandLiteral, "%x(café)", "`café`");
    }
    #[test]
    fn unusual_delimiters_remain_reported_but_unmodified() {
        for source in ["%x hello ", "%x\thello\t", "%x\nhello\n", "%x\\hello\\"] {
            assert_eq!(offenses(source).len(), 1, "{source}");
            assert_correction(&CommandLiteral, source, source);
        }
    }
    #[test]
    fn pinned_accepted_commands_and_heredocs() {
        for source in [
            "foo = `ls`",
            "foo = `\n  ls\n  ls -l\n`",
            "foo = %x(echo `ls`)",
            "foo = %x(\n  echo `ls`\n  echo `ls -l`\n)",
            "<<`COMMAND`\n  ls\nCOMMAND\n",
            "p <<~`COMMAND`\n  #{value}\nCOMMAND\n",
            "'%x(ls)'",
            "\"`ls`\"",
            "# %x(ls)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
