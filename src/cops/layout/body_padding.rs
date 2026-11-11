//! Shared pinned `no_empty_lines` checks. Only the immediately adjacent empty
//! line is reported at each edge; additional padding needs another explicit pass.

use crate::{
    cops::{
        Cop, NodeContext,
        style::correction::{line_value_boundary, touches_directive},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub(super) fn check(
    cop: &dyn Cop,
    kind: &str,
    (first, last): (u32, u32),
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    if first >= last {
        return;
    }
    report(cop, kind, (first + 1, "beginning"), cx, file, out);
    // RuboCop deduplicates the two edges of a one-blank-line empty body.
    if last - 1 != first + 1 {
        report(cop, kind, (last - 1, "end"), cx, file, out);
    }
}

/// The closed observer hold for the body cops added in v1.0
/// (`begin` and block bodies): withhold the corrections of `found`, keeping
/// the findings, when the file holds a listed source observer. The accepted
/// class, module and method body cops keep their earlier guards unchanged.
pub(super) fn hold_for_listed_observers(
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    found: &mut [Offense],
) {
    if found.iter().all(|o| o.correction.is_none()) {
        return;
    }
    let Some(root) = cx.ancestors().next_back() else {
        return;
    };
    if file
        .class_method_bindings(root)
        .has_listed_source_observer(file.data_start().is_some())
    {
        for offense in found {
            offense.correction = None;
        }
    }
}

pub(super) fn report(
    cop: &dyn Cop,
    kind: &str,
    (line, edge): (u32, &str),
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    if !file.line_text(line).is_empty() {
        return;
    }
    out.push(Offense {
        correction: line_removal(cx, file, line),
        cop: cop.name(),
        severity: cop.severity(),
        message: format!("Extra empty line detected at {kind} body {edge}."),
        line,
        end_line: line,
        col: 1,
    });
}

/// The guarded removal of the whole blank or whitespace-only `line` (after
/// line 1), with its own line ending; `None` when not requested or held.
pub(super) fn line_removal(
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    line: u32,
) -> Option<Correction> {
    let range = file.line_start(line)..file.line_start(line + 1);
    cx.corrections_requested()
        .then(|| line_value_boundary(file))
        .filter(|&boundary| range.start >= boundary)
        // After an escaped newline, this blank line supplies the statement
        // boundary. Removing it can turn body code into header arguments even
        // when the result still parses. Conservatively include lookalikes.
        .filter(|_| !file.line_text(line - 1).ends_with(b"\\"))
        .filter(|_| {
            cx.ancestors()
                .next_back()
                .is_some_and(|root| !file.overlaps_literal(root, &range))
        })
        .filter(|_| !touches_directive(file, file.line_start(line - 1)..file.line_start(line + 2)))
        .map(|_| Correction {
            range,
            replacement: String::new(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::{
        layout::{
            EmptyLines, EmptyLinesAroundBeginBody, EmptyLinesAroundBlockBody,
            EmptyLinesAroundClassBody, EmptyLinesAroundMethodBody, EmptyLinesAroundModuleBody,
        },
        testing::{assert_correction, correction_offenses},
    };

    fn cops() -> [(&'static dyn Cop, &'static str); 7] {
        [
            (&EmptyLinesAroundClassBody, "class C"),
            (&EmptyLinesAroundModuleBody, "module M"),
            (&EmptyLinesAroundMethodBody, "def foo"),
            (&EmptyLinesAroundBeginBody, "begin"),
            (&EmptyLinesAroundBlockBody, "foo do"),
            (&EmptyLinesAroundBlockBody, "foo do |a,\n  b|"),
            (&EmptyLinesAroundBlockBody, "f = -> do"),
        ]
    }

    #[test]
    fn empty_bodies_deduplicate_but_repeated_padding_is_single_pass() {
        for (cop, header) in cops() {
            let before = format!("{header}\n\nend\n");
            let found = correction_offenses(cop, &before);
            assert_eq!(found.len(), 1);
            assert_eq!(
                (found[0].line, found[0].col),
                (header.lines().count() as u32 + 1, 1)
            );
            assert!(found[0].message.ends_with("beginning."));
            assert_correction(cop, &before, &format!("{header}\nend\n"));
            assert_correction(
                cop,
                &format!("{header}\n\n\n\n\nend\n"),
                &format!("{header}\n\n\nend\n"),
            );
        }
    }

    #[test]
    fn explicit_continuations_keep_the_following_blank_statement_boundary() {
        for (cop, header) in cops().into_iter().chain([
            (&EmptyLinesAroundMethodBody as &dyn Cop, "def self.foo"),
            (&EmptyLinesAroundClassBody as &dyn Cop, "class C < parent"),
            (&EmptyLinesAroundClassBody as &dyn Cop, "class << self"),
        ]) {
            for newline in ["\n", "\r\n"] {
                let before =
                    format!("{header} \\{newline}{newline}  work{newline}{newline}end{newline}");
                let after = format!("{header} \\{newline}{newline}  work{newline}end{newline}");
                let found = correction_offenses(cop, &before);
                assert_eq!(found.len(), 2, "{before:?}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (header.lines().count() as u32 + 1, 1)
                );
                assert!(found[0].correction.is_none(), "{before:?}");
                assert!(found[1].correction.is_some());
                assert_correction(cop, &before, &after);
            }
        }
    }

    #[test]
    fn exact_crlf_ranges_and_multibyte_comments_are_preserved() {
        for (cop, header) in cops() {
            let before = format!(
                "\u{feff}# 日本語\r\n{header}\r\n\r\n  # é\r\n  work\r\n  # tail\r\n\r\nend\r\n__END__\r\n\r\nraw\r\n"
            );
            let after = before
                .replace("\r\n\r\n  #", "\r\n  #")
                .replace("# tail\r\n\r\n", "# tail\r\n");
            let found = correction_offenses(cop, &before);
            assert_eq!(found.len(), 2);
            for o in found {
                let correction = o.correction.unwrap();
                assert_eq!(&before.as_bytes()[correction.range], b"\r\n");
                assert!(correction.replacement.is_empty());
            }
            assert_correction(cop, &before, &after);
        }
    }

    #[test]
    fn later_line_values_withhold_across_scopes_but_not_earlier_values() {
        for (cop, header) in cops() {
            for tail in [
                "p __LINE__\n",
                "class Lines\n  def self.line; __LINE__; end\nend\n",
                "p \"#{__LINE__}\"\n",
                "# __LINE__ lookalike\n",
            ] {
                let before = format!("{header}\n\n  work\n\nend\n{tail}");
                assert_eq!(correction_offenses(cop, &before).len(), 2);
                assert_correction(cop, &before, &before);
            }
            assert_correction(
                cop,
                &format!("p __LINE__\n{header}\n\n  work\n\nend\n"),
                &format!("p __LINE__\n{header}\n  work\nend\n"),
            );
        }
    }

    #[test]
    fn comments_are_never_removed_and_directive_boundaries_withhold_fixes() {
        for (cop, header) in cops() {
            for prefix in ["rubocop", "rustocop"] {
                for directive in [
                    "disable-next Lint/Debugger",
                    "disable Lint/Debugger",
                    "enable Lint/Debugger",
                ] {
                    let before = format!(
                        "{header} # {prefix}:{directive}\n\n  work\n  # {prefix}:{directive}\n\nend\n"
                    );
                    assert_eq!(correction_offenses(cop, &before).len(), 2);
                    assert_correction(cop, &before, &before);
                    let before = format!(
                        "{header}\n\n  # {prefix}:{directive}\n  work\n\nend # {prefix}:{directive}\n"
                    );
                    assert_correction(cop, &before, &before);
                }
                for target in [
                    cop.name(),
                    cop.name().split('/').nth(1).unwrap(),
                    "Layout",
                    "all",
                ] {
                    for action in ["disable", "todo"] {
                        let before =
                            format!("# {prefix}:{action} {target}\n{header}\n\n  work\n\nend\n");
                        assert!(correction_offenses(cop, &before).is_empty(), "{before}");
                        assert!(
                            crate::pipeline::lint_str(&before)
                                .iter()
                                .all(|o| o.cop != cop.name())
                        );
                    }
                }
                let before = format!(
                    "{header}\n  # {prefix}:disable-next {}\n\nend\n",
                    cop.name()
                );
                // A blank detaches disable-next; detection remains, but do
                // not delete that blank and attach it to `end` instead.
                assert_correction(cop, &before, &before);
            }
        }
    }

    #[test]
    fn literal_text_and_data_are_not_body_padding() {
        for (cop, header) in cops() {
            for literal in [
                "'a\n\nb'",
                "\"a\n\nb\"",
                "/a\n\nb/",
                "%q(a\n\nb)",
                "%Q(a\n\nb)",
                "%w(a\n\nb)",
                "%W(a\n\nb)",
                "<<~H\n  a\n\n  b\nH",
            ] {
                let source = format!("{header}\n  value = {literal}\nend\n__END__\n\n\nraw\n");
                assert!(correction_offenses(cop, &source).is_empty(), "{source}");
            }
        }
        // Header expressions may start heredocs before their bodies. Pinned
        // detection still sees the empty line, but it is not safe code padding.
        for (cop, header) in [
            (
                &EmptyLinesAroundClassBody as &dyn Cop,
                "class C < build(<<H)",
            ),
            (
                &EmptyLinesAroundMethodBody as &dyn Cop,
                "def foo(arg = <<H)",
            ),
        ] {
            let before = format!("{header}\n\nH\n  work\nend\n");
            assert_eq!(correction_offenses(cop, &before).len(), 1);
            assert_correction(cop, &before, &before);
        }
    }

    #[test]
    fn interpolation_code_is_checked_but_percent_array_fixes_are_withheld() {
        for (cop, header) in cops() {
            let code = format!("{header}\n\n  1\n\nend");
            let fixed = format!("{header}\n  1\nend");
            for (open, close, safe) in [("\"#{", "}\"", true), ("%W(a#{", "})", false)] {
                let before = format!("x = {open}{code}{close}\n");
                let after = if safe {
                    format!("x = {open}{fixed}{close}\n")
                } else {
                    before.clone()
                };
                assert_correction(cop, &before, &after);
            }
        }
    }

    #[test]
    fn mixed_empty_lines_overlap_selection_is_order_independent_and_converges() {
        for (cop, header) in cops() {
            for body in ["  work\n\n\n\n", ""] {
                let before = format!("{header}\n\n\n\n{body}end\n");
                let expected = format!(
                    "{header}\n{}end\n",
                    if body.is_empty() { "" } else { "  work\n" }
                );
                let mut results = Vec::new();
                for selected in [
                    [cop, &EmptyLines as &dyn Cop],
                    [&EmptyLines as &dyn Cop, cop],
                ] {
                    let found = crate::pipeline::lint_str_with_corrections(&selected, &before);
                    let result = crate::corrections::apply(before.as_bytes(), &found)
                        .unwrap()
                        .bytes;
                    let first_pass = format!(
                        "{header}\n{}end\n",
                        if body.is_empty() { "" } else { "  work\n\n" }
                    );
                    assert_eq!(result, first_pass.as_bytes());
                    let remaining =
                        crate::pipeline::lint_str_with_corrections(&selected, &first_pass);
                    let second = crate::corrections::apply(&result, &remaining)
                        .unwrap()
                        .bytes;
                    assert_eq!(second, expected.as_bytes());
                    assert!(
                        crate::pipeline::lint_str_with_corrections(&selected, &expected).is_empty()
                    );
                    results.push(result);
                }
                assert_eq!(results[0], results[1]);
            }
        }
    }
}
