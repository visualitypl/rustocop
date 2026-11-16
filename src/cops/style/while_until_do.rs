//! `Style/WhileUntilDo` — redundant `do` on multiline loops.
use super::while_until_modifier::Loop;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct WhileUntilDo;
impl Cop for WhileUntilDo {
    fn name(&self) -> &'static str {
        "Style/WhileUntilDo"
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
        let Some(c) = Loop::of(node) else { return };
        let Some(do_keyword) = c.do_keyword.as_ref() else {
            return;
        };
        let (line, col) = file.line_col(do_keyword.start_offset());
        if file.line_of(node.location().start_offset()) == file.last_line_of(node.location())
            || c.body
                .as_ref()
                .is_some_and(|b| file.line_of(b.location().start_offset()) == line)
        {
            return;
        }
        let range = c.predicate.location().end_offset()..do_keyword.end_offset();
        let correction = if cx.corrections_requested()
            && range.len() <= 16 * 1024
            && file.bytes[range.start..do_keyword.start_offset()]
                .iter()
                .all(u8::is_ascii_whitespace)
            // `do` terminates an open range/flip-flop, even below an
            // unparenthesized wrapper. Deleting it can consume the first body
            // expression as the right endpoint and change loop execution.
            && !super::multiline_if_then::ends_with_open_range(&c.predicate)
        {
            Some(Correction {
                range,
                replacement: String::new(),
            })
        } else {
            None
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!("Do not use `do` with multi-line `{}`.", c.word()),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop while_until_do_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn multiline_do_positions_messages_and_corrections() {
        for word in ["while", "until"] {
            for body in ["", "  work\n"] {
                let source = format!("{word} cond do\n{body}end\n");
                let o: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/WhileUntilDo")
                    .collect();
                assert_eq!(o.len(), 1, "{source}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 12, 1));
                assert_eq!(
                    o[0].message,
                    format!("Do not use `do` with multi-line `{word}`.")
                );
                assert_correction(&WhileUntilDo, &source, &format!("{word} cond\n{body}end\n"));
            }
        }
    }
    #[test]
    fn trailing_open_ranges_retain_do_and_exact_diagnostics() {
        use crate::cops::testing::correction_offenses;
        for word in ["while", "until"] {
            for dots in ["..", "..."] {
                for predicate in [
                    format!("false{dots}"),
                    format!("!false{dots}"),
                    format!("not false{dots}"),
                    format!("true && false{dots}"),
                    format!("false || false{dots}"),
                    format!("flag = false{dots}"),
                    format!("consume false{dots}"),
                ] {
                    for nl in ["\n", "\r\n"] {
                        for gap in [" ", "\n  ", " \\\n  "] {
                            let source = format!("{word} {predicate}{gap}do\n work\n break\nend\n")
                                .replace('\n', nl);
                            assert_eq!(
                                crate::parser::parse(source.as_bytes()).errors().count(),
                                0,
                                "{source}"
                            );
                            let found = correction_offenses(&WhileUntilDo, &source);
                            assert_eq!(found.len(), 1, "{source}");
                            let (line, col) = if gap.contains('\n') {
                                (2, 3)
                            } else {
                                (1, (word.len() + predicate.len() + 3) as u32)
                            };
                            assert_eq!(
                                (found[0].line, found[0].col, found[0].end_line),
                                (line, col, line),
                                "{source}"
                            );
                            assert_eq!(
                                found[0].message,
                                format!("Do not use `do` with multi-line `{word}`.")
                            );
                            assert!(found[0].correction.is_none(), "{source}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn closed_range_wrappers_and_finite_ranges_still_correct() {
        for word in ["while", "until"] {
            for dots in ["..", "..."] {
                for predicate in [
                    format!("(false{dots})"),
                    format!("!(false{dots})"),
                    format!("true && (false{dots})"),
                    format!("(flag = false{dots})"),
                    format!("consume(false{dots})"),
                    format!("false{dots}true"),
                ] {
                    assert_correction(
                        &WhileUntilDo,
                        &format!("{word} {predicate} do\n work\n break\nend\n"),
                        &format!("{word} {predicate}\n work\n break\nend\n"),
                    );
                }
            }
        }
    }
    #[test]
    fn explicit_continuations_keep_do_as_a_separator() {
        use crate::cops::testing::correction_offenses;
        for word in ["while", "until"] {
            for nl in ["\n", "\r\n"] {
                let source = format!("{word} true \\\n  do\n -1\n break\nend\n").replace('\n', nl);
                assert_eq!(crate::parser::parse(source.as_bytes()).errors().count(), 0);
                let found = correction_offenses(&WhileUntilDo, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert!(found[0].correction.is_none(), "{source}");
                // The continuation is safely closed within this predicate.
                let predicate = "(true && \\\n true)";
                assert_correction(
                    &WhileUntilDo,
                    &format!("{word} {predicate} do\n work\nend\n").replace('\n', nl),
                    &format!("{word} {predicate}\n work\nend\n").replace('\n', nl),
                );
            }
        }
    }
    #[test]
    fn same_line_body_or_no_do_are_accepted() {
        for s in [
            "while cond do work end",
            "until cond do work end",
            "while cond do work\nend",
            "until cond do work\nend",
            "while a ||\n b do work\nend",
            "while cond\nend",
            "until cond\nend",
            "work while cond",
        ] {
            assert!(
                lint_str(s).iter().all(|o| o.cop != "Style/WhileUntilDo"),
                "{s}"
            );
        }
    }
}
