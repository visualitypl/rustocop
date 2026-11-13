//! `Style/DisableCopsWithinSourceCodeDirective`: opt-in ban on source directives.
use super::correction::{combine, contains_magic_comment_key};
use crate::{
    cops::{Cop, NodeContext},
    directives::{
        comment_attachments,
        comments::{BAN, DirectiveComment, hits, registered_names, signed_hits},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct DisableCopsWithinSourceCodeDirective;
impl Cop for DisableCopsWithinSourceCodeDirective {
    fn name(&self) -> &'static str {
        "Style/DisableCopsWithinSourceCodeDirective"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_program_node().is_none() {
            return;
        }
        let records = file.directive_records();
        let attachments = cx
            .corrections_requested()
            .then(|| comment_attachments(file));
        let mut protected = std::collections::HashSet::new();
        if let Some(attachments) = &attachments {
            // Every push saves every cop, not just its named targets. Keep
            // boundaries around changes to unrelated cops, even nested or
            // unclosed pushes. Prefix counts avoid rescanning deep interiors.
            let mut changes = vec![0usize];
            for d in records {
                changes.push(changes.last().unwrap() + usize::from(affects_other_cops(d)));
            }
            for (i, d) in records.iter().enumerate() {
                if d.mode == "push" && changes[d.pop.unwrap_or(records.len())] > changes[i + 1] {
                    protected.insert(d.line);
                }
            }
            let mut first_next = std::collections::HashMap::new();
            for d in records {
                if d.standalone
                    && matches!(
                        d.mode,
                        "disable-next" | "todo-next" | "enable-next" | "next"
                    )
                    && affects_other_cops(d)
                    && let Some(&code) = attachments.get(&d.line)
                {
                    first_next.entry(code).or_insert(d.line);
                }
            }
            for d in records {
                if d.standalone
                    && let Some(code) = attachments.get(&d.line)
                    && first_next.get(code).is_some_and(|&first| first < d.line)
                {
                    protected.insert(d.line);
                }
            }
        }
        let mut corrected_until = 0;
        for d in records {
            if !d.has_cops() {
                continue;
            }
            let (line, col) = file.line_col(d.range.start);
            // Atomic push/pop spans overlap nested directives. The existing
            // first-wins engine would defer those inner edits anyway; avoid
            // copying their overlapping interiors (quadratic on deep stacks).
            let correction = (cx.corrections_requested() && d.range.start >= corrected_until)
                .then(|| removal(file, d, records, &protected))
                .flatten();
            if let Some(correction) = &correction {
                corrected_until = correction.range.end;
            }
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "RuboCop disable/enable directives are not permitted.".into(),
                line,
                col,
                end_line: file.line_of(d.range.end.saturating_sub(1).max(d.range.start)),
                correction,
            });
        }
    }
}
/// Removing an active selector would re-enable unrelated offenses (or extend
/// an earlier disable when removing an enable). Keep the offense but withhold
/// that edit, even when the target happens to be clean or unselected today.
fn removal(
    file: &SourceFile,
    d: &DirectiveComment,
    records: &[DirectiveComment],
    protected: &std::collections::HashSet<u32>,
) -> Option<Correction> {
    if affects_other_cops(d) {
        return None;
    }
    let mut edits = Vec::new();
    for record in std::iter::once(d).chain(d.pop.map(|pop| &records[pop])) {
        // Even an unknown-cop comment can carry another directive's attachment.
        // Removing either boundary of an atomic pair must preserve that chain.
        if protected.contains(&record.line) {
            return None;
        }
        let bytes = &file.bytes[record.range.clone()];
        // Preserve physical lines, source headers and arbitrary legacy bytes.
        if bytes.contains(&b'\n') || bytes.starts_with(b"#!") || contains_magic_comment_key(bytes) {
            return None;
        }
        // Prism includes CR in an inline comment's location, but it belongs
        // to the physical CRLF terminator, not the comment being removed.
        let mut range = record.range.clone();
        if bytes.last() == Some(&b'\r') && file.bytes.get(range.end) == Some(&b'\n') {
            range.end -= 1;
        }
        edits.push(Correction {
            range,
            replacement: String::new(),
        });
    }
    combine(file, edits)
}

fn affects_other_cops(d: &DirectiveComment) -> bool {
    let signed = matches!(d.mode, "push" | "next");
    let selector_hits = if signed { signed_hits } else { hits };
    let names: Vec<_> = if signed {
        d.signed.iter().map(|(_, n)| n.as_str()).collect()
    } else {
        d.names.iter().map(String::as_str).collect()
    };
    names.iter().any(|name| {
        registered_names()
            .iter()
            .any(|cop| *cop != BAN && selector_hits(name, cop))
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/disable_cops_within_source_code_directive_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn canonical_push_pop_and_signed_next_removals_are_atomic() {
        assert_correction(
            &DisableCopsWithinSourceCodeDirective,
            "# rubocop:push -Migration/Nonexistent\n# rubocop:push\ndef foo\nend\n# rubocop:pop\n# rubocop:pop\n",
            "\n# rubocop:push\ndef foo\nend\n# rubocop:pop\n\n",
        );
        assert_correction(
            &DisableCopsWithinSourceCodeDirective,
            // Both selectors must remain unregistered: Style/For, the
            // Metrics complexity cops and
            // Migration/DepartmentName now exist.
            "# rustocop:next -Migration/Nonexistent +Metrics/Future\ndef foo\nend\n",
            "\ndef foo\nend\n",
        );
        assert_correction(
            &DisableCopsWithinSourceCodeDirective,
            "# rustocop:disable-next Migration/Nonexistent -- legacy\ndef foo\nend\n",
            "\ndef foo\nend\n",
        );
        // The landed Migration/DepartmentName is an active selector.
        for source in [
            "# rubocop:push -Migration/DepartmentName\n# rubocop:push\ndef foo\nend\n# rubocop:pop\n# rubocop:pop\n",
            "# rustocop:next -Migration/DepartmentName +Metrics/Future\ndef foo\nend\n",
            "# rustocop:disable-next Migration/DepartmentName -- legacy\ndef foo\nend\n",
        ] {
            let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn self_disabling_cannot_bypass_explicit_opt_in() {
        for prefix in ["rubocop", "rustocop"] {
            for target in [BAN, "DisableCopsWithinSourceCodeDirective", "Style", "all"] {
                let source = format!(
                    "# {prefix}:disable {target}\ndef foo # {prefix}:disable Migration/Nonexistent\nend\n"
                );
                let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &source);
                assert_eq!(found.len(), 2, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 1));
                assert_eq!((found[1].line, found[1].col), (2, 9));
                if target == BAN || target == "DisableCopsWithinSourceCodeDirective" {
                    assert_correction(
                        &DisableCopsWithinSourceCodeDirective,
                        &source,
                        "\ndef foo \nend\n",
                    );
                } else {
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn preserves_active_suppressions_even_if_target_is_clean_or_unselected() {
        for target in [
            "Lint/Debugger",
            "Debugger",
            "Lint",
            "Style",
            "Style/StringLiterals",
        ] {
            for mode in [
                "disable",
                "todo",
                "enable",
                "disable-next",
                "todo-next",
                "enable-next",
                "push",
                "next",
            ] {
                let signed = if matches!(mode, "push" | "next") {
                    "-"
                } else {
                    ""
                };
                let source = format!("# rubocop:{mode} {signed}{target}\nfoo\n# rubocop:pop\n");
                let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn literal_signed_all_is_removable_but_ordinary_all_is_protected() {
        for prefix in ["rubocop", "rustocop"] {
            for sign in ["+", "-"] {
                assert_correction(
                    &DisableCopsWithinSourceCodeDirective,
                    &format!("# {prefix}:push {sign}all\nwork\n# {prefix}:pop\n"),
                    "\nwork\n\n",
                );
                assert_correction(
                    &DisableCopsWithinSourceCodeDirective,
                    &format!("# {prefix}:next {sign}all\nwork\n"),
                    "\nwork\n",
                );
                for mode in ["push", "next"] {
                    let source = format!(
                        "# {prefix}:{mode} {sign}all {sign}Lint/Debugger\nwork\n# {prefix}:pop\n"
                    );
                    let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert!(found[0].correction.is_none(), "{source}");
                }
            }
            for mode in [
                "disable",
                "todo",
                "enable",
                "disable-next",
                "todo-next",
                "enable-next",
            ] {
                let source = format!("# {prefix}:{mode} all\nwork\n");
                let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn comments_without_captured_cops_and_literal_mentions_are_ignored() {
        for source in [
            "# rubocop:disable\n",
            "# rustocop:push\nfoo\n# rustocop:pop\n",
            "# mention rubocop:disable Lint/Debugger\n",
            "## rubocop:disable Lint/Debugger\n",
            "# # rustocop:enable all\n",
            "#\u{b}# rustocop:enable all\n",
            "# rubocop:disabled all\n",
            "# rustocop:disableé Lint/Debugger\n",
            "# rubocop:todo\u{301} all\n",
            "# rubocop:push Metrics/AbcSize\n",
            "# rubocop:next Metrics/AbcSize\n",
            "x = '# rubocop:disable Lint/Debugger'\n",
            "x = <<~H\n# rubocop:disable all\nH\n",
            "__END__\n# rubocop:disable all\n",
        ] {
            assert!(
                correction_offenses(&DisableCopsWithinSourceCodeDirective, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn embedded_directives_are_real_but_plain_mentions_are_not() {
        let source = "# example # rubocop:disable Migration/Nonexistent -- legacy\n";
        let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, source);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_correction(&DisableCopsWithinSourceCodeDirective, source, "\n");
        assert_correction(
            &DisableCopsWithinSourceCodeDirective,
            "#\u{b}rustocop\u{b}:\u{b}todo\u{b}Migration/Nonexistent\n",
            "\n",
        );
    }
    #[test]
    fn unsafe_header_and_multiline_comment_removals_are_withheld() {
        for source in [
            "# rubocop:disable Metrics/AbcSize -- coding: ASCII-8BIT\np __LINE__\n",
            "# rubocop:disable Metrics/AbcSize -- frozen_string_literal: true\n",
            "=begin\n# rubocop:disable Metrics/AbcSize\n=end\np __LINE__\n",
        ] {
            let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn nested_atomic_corrections_copy_only_disjoint_source_spans() {
        let source = format!(
            "{}work\n{}",
            "# rubocop:push -Metrics/AbcSize\n".repeat(1000),
            "# rubocop:pop\n".repeat(1000)
        );
        let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &source);
        assert_eq!(found.len(), 1000);
        let copied: usize = found
            .iter()
            .filter_map(|o| o.correction.as_ref())
            .map(|c| c.range.len())
            .sum();
        assert!(copied <= source.len(), "{copied} > {}", source.len());
    }
    #[test]
    fn canonical_inline_removals_keep_whitespace() {
        for prefix in ["rubocop", "rustocop"] {
            for mode in [
                "disable",
                "enable",
                "todo",
                "disable-next",
                "todo-next",
                "enable-next",
            ] {
                let s = format!("def foo # {prefix}:{mode} Migration/Nonexistent -- legacy\nend\n");
                let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col), (1, 9));
                assert_eq!(
                    found[0].message,
                    "RuboCop disable/enable directives are not permitted."
                );
                assert_correction(&DisableCopsWithinSourceCodeDirective, &s, "def foo \nend\n");
                // A landed cop's directive (the Metrics complexity cops,
                // Migration/DepartmentName) is an
                // active selector: reported, not removed.
                for cop in ["Metrics/CyclomaticComplexity", "Migration/DepartmentName"] {
                    let s = format!("def foo # {prefix}:{mode} {cop} -- legacy\nend\n");
                    let found = correction_offenses(&DisableCopsWithinSourceCodeDirective, &s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert!(found[0].correction.is_none(), "{s}");
                }
            }
        }
    }
}
