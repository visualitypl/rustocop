//! `Lint/MissingCopEnableDirective`: require final open disables to be closed.
//! Fixed MaxRangeSize is infinite; finite and self-closing ranges are acceptable.
use crate::{
    cops::Cop,
    directives::{
        comment_attachments,
        comments::{BAN, hits, registered_names, signed_hits},
    },
    offense::Offense,
    source::SourceFile,
};
use std::collections::HashSet;

pub struct MissingCopEnableDirective;
impl Cop for MissingCopEnableDirective {
    fn name(&self) -> &'static str {
        "Lint/MissingCopEnableDirective"
    }

    fn check_source(&self, file: &SourceFile, out: &mut Vec<Offense>) {
        let disabled: HashSet<_> = file
            .directive_disabled()
            .iter()
            .map(String::as_str)
            .collect();
        let mut candidates = Vec::new();
        let records = file.directive_records();
        let attachments = records
            .iter()
            .any(|d| d.mode.ends_with("next"))
            .then(|| comment_attachments(file));
        // The vocabulary is registry-bounded, not source-sized. Retain each
        // cop's first analysis insertion order, as CommentConfig does, since
        // Base reports only the first finding on a given comment range.
        for &cop in registered_names() {
            if disabled.contains(cop) || cop == BAN {
                continue;
            }
            let mut first = None;
            let mut open = None;
            let mut stack = Vec::new();
            for (index, d) in records.iter().enumerate() {
                if d.mode == "pop" {
                    if let Some(saved) = stack.pop() {
                        open = saved;
                    }
                    continue;
                }
                if d.mode == "push" {
                    stack.push(open);
                }
                if matches!(d.mode, "push" | "next") {
                    if d.mode == "next"
                        && (!d.standalone
                            || attachments
                                .as_ref()
                                .is_none_or(|a| !a.contains_key(&d.line)))
                    {
                        continue;
                    }
                    // Signed arguments are grouped by sign in first-sign order.
                    let sign = d.signed.first().map(|&(plus, _)| plus);
                    for (group, plus) in sign.into_iter().chain(sign.map(|p| !p)).enumerate() {
                        if let Some(arg) = d
                            .signed
                            .iter()
                            .position(|(p, n)| *p == plus && signed_hits(n, cop))
                        {
                            first.get_or_insert((index, group, arg));
                            if d.mode == "push" {
                                if plus {
                                    open = None;
                                } else if open.is_none() {
                                    open = Some(d);
                                }
                            }
                        }
                    }
                    continue;
                }
                if !d.signed.is_empty() {
                    continue;
                }
                let Some(arg) = d.names.iter().position(|n| hits(n, cop)) else {
                    continue;
                };
                if d.mode.ends_with("next") {
                    if d.standalone
                        && attachments
                            .as_ref()
                            .is_some_and(|a| a.contains_key(&d.line))
                    {
                        first.get_or_insert((index, 0, arg));
                    }
                    // Statement ranges are finite. Temporary enables reopen
                    // with the same origin, even below EOF; no bounds needed.
                    continue;
                }
                first.get_or_insert((index, 0, arg));
                if d.standalone && !d.embedded {
                    open = d.disabled().then_some(d);
                }
            }
            if let Some(d) = open {
                candidates.push((first, cop, d));
            }
        }
        candidates.sort_by_key(|&(first, _, _)| first);
        let mut reported = HashSet::new();
        for (_, cop, d) in candidates {
            if !reported.insert(d.range.start) {
                continue;
            }
            let department = cop.split('/').next().expect("qualified cop");
            let (target, kind) = if d.names.iter().any(|n| n == department) {
                (department, "department")
            } else {
                (cop, "cop")
            };
            let closer = if d.mode == "push" { "pop" } else { "enable" };
            let (line, col) = file.line_col(d.range.start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Re-enable {target} {kind} with `# rubocop:{closer}` after disabling it."
                ),
                line,
                col,
                end_line: file.line_of(d.range.end.saturating_sub(1).max(d.range.start)),
                correction: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/missing_cop_enable_directive_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/MissingCopEnableDirective")
            .collect()
    }

    #[test]
    fn push_origins_and_restoration() {
        for (source, line, target, closer) in [
            (
                "# rubocop:push -Layout/SpaceAroundOperators\nx = 0\n",
                1,
                "Layout/SpaceAroundOperators cop",
                "pop",
            ),
            (
                "# rubocop:push\n# rubocop:disable Layout/SpaceAroundOperators\nx = 0\n",
                2,
                "Layout/SpaceAroundOperators cop",
                "enable",
            ),
            (
                "# rubocop:disable Layout/SpaceAroundOperators\n# rubocop:push +Layout/SpaceAroundOperators\nx = 0\n# rubocop:pop\n",
                1,
                "Layout/SpaceAroundOperators cop",
                "enable",
            ),
            (
                "# rubocop:push -Layout\nx = 0\n",
                1,
                "Layout/LineLength cop",
                "pop",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, 1), "{source}");
            assert_eq!(
                found[0].message,
                format!("Re-enable {target} with `# rubocop:{closer}` after disabling it.")
            );
        }
    }

    #[test]
    fn self_closing_next_and_matched_pushes_need_no_enable() {
        for prefix in ["rubocop", "rustocop"] {
            for source in [
                format!(
                    "# {prefix}:push\n# {prefix}:disable Layout/LineLength\nx\n# {prefix}:pop\ny\n"
                ),
                format!("# {prefix}:push -Layout/LineLength\nx\n# {prefix}:pop\n"),
                format!(
                    "# {prefix}:push -Layout/LineLength\n# {prefix}:push +Layout/LineLength\nx\n# {prefix}:pop\n# {prefix}:pop\n"
                ),
                format!("# {prefix}:disable-next Layout/LineLength\nx = [\n  1,\n  2\n]\n"),
                format!("# {prefix}:next -Layout/LineLength\nx = <<~TEXT\n  hello\nTEXT\n"),
                format!("# {prefix}:todo-next Layout/LineLength\n\nx\n"),
                format!("x # {prefix}:disable-next Layout/LineLength\n"),
                format!("# {prefix}:disable-next Layout/LineLength\n"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }

    #[test]
    fn next_enables_preserve_original_opening_comment() {
        for prefix in ["rubocop", "rustocop"] {
            for mode in ["enable-next Layout/LineLength", "next +Layout/LineLength"] {
                let open = format!(
                    "# {prefix}:disable Layout/LineLength\nx\n# {prefix}:{mode}\ny = [\n  1\n]\nz\n"
                );
                let found = offenses(&open);
                assert_eq!(found.len(), 1, "{open}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert!(
                    offenses(&format!("{open}# {prefix}:enable Layout/LineLength\n")).is_empty()
                );
            }
        }
    }

    #[test]
    fn departments_aliases_lists_and_last_disable_attribution() {
        for (source, line, target) in [
            ("# rubocop:disable Layout\nx\n", 1, "Layout department"),
            ("# rubocop:todo LineLength\nx\n", 1, "Layout/LineLength cop"),
            (
                "# rubocop:disable Layout/SpaceAroundOperators, Layout/LineLength\nx\n",
                1,
                "Layout/SpaceAroundOperators cop",
            ),
            (
                "# rubocop:disable Layout/LineLength\n# rubocop:disable Layout/LineLength\nx\n",
                2,
                "Layout/LineLength cop",
            ),
            (
                "# rubocop:disable Layout/LineLength, Layout\nx\n",
                1,
                "Layout department",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, 1));
            assert_eq!(
                found[0].message,
                format!("Re-enable {target} with `# rubocop:enable` after disabling it.")
            );
        }
    }

    #[test]
    fn finite_ranges_and_non_directive_lookalikes() {
        for source in [
            "# rubocop:disable Layout/LineLength\nx\n# rubocop:enable Layout/LineLength\n",
            "# rubocop:disable Layout\nx\n# rubocop:enable all\n",
            "x # rubocop:disable Layout/LineLength\ny\n",
            "# note # rubocop:disable Layout/LineLength\nx\n",
            "# # rubocop:disable Layout/LineLength\nx\n",
            "# rubocop:disable Unknown/Cop, Wrong/LineLength, Lint/Syntax\nx\n",
            "\"# rubocop:disable Layout/LineLength\"\n",
            "x = <<~TEXT\n# rubocop:disable Layout/LineLength\nTEXT\n",
            "__END__\n# rubocop:disable Layout/LineLength\n",
            "=begin\n# rubocop:disable Layout/LineLength\n=end\nx\n",
            "# rubocop:disable Style/FrozenStringLiteralComment\nx\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn reports_the_opening_comment_with_exact_message() {
        let found = offenses("  # rubocop:disable Layout/SpaceAroundOperators -- reason\nx = 0\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 3, 1));
        assert_eq!(
            found[0].message,
            "Re-enable Layout/SpaceAroundOperators cop with `# rubocop:enable` after disabling it."
        );
        assert!(found[0].correction.is_none());
    }
}
