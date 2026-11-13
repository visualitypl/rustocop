//! `Style/InPatternThen` — prefer `then` to a same-line pattern separator.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{InNode, Node};
pub struct InPatternThen;
impl Cop for InPatternThen {
    fn name(&self) -> &'static str {
        "Style/InPatternThen"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_in_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(n) = node.as_in_node() else { return };
        let Some(body) = n.statements() else { return };
        if n.then_loc().is_some()
            || file.line_of(n.in_loc().start_offset()) != file.last_line_of(body.location())
        {
            return;
        }
        // Prism owns the boundary, including any guard. Only its first
        // non-whitespace token can be the separator, never a literal's `;`.
        let gap = n.pattern().location().end_offset()..body.location().start_offset();
        let Some(relative) = file.bytes[gap.clone()]
            .iter()
            .position(|b| !super::is_ruby_whitespace(*b))
        else {
            return;
        };
        let at = gap.start + relative;
        if file.bytes[at] != b';' {
            return;
        }
        let expression = pattern_source(pattern(&n));
        let (line, col) = file.line_col(at);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!("Do not use `in {expression};`. Use `in {expression} then` instead."),
            correction: cx
                .corrections_requested()
                .then(|| {
                    super::negated_if::finish(
                        Correction {
                            range: at..at + 1,
                            replacement: " then".into(),
                        },
                        cx,
                        file,
                    )
                })
                .flatten(),
        });
    }
}

/// RuboCop keeps guards separate; Prism wraps the pattern in a modifier node.
/// Use the wrapper for token boundaries but the actual pattern for policy.
pub(super) fn pattern<'pr>(n: &InNode<'pr>) -> Node<'pr> {
    let p = n.pattern();
    if let Some(guard) = p.as_if_node() {
        return guard.statements().unwrap().body().first().unwrap();
    }
    if let Some(guard) = p.as_unless_node() {
        return guard.statements().unwrap().body().first().unwrap();
    }
    p
}

fn pattern_source(mut pattern: Node<'_>) -> String {
    // Upstream normalizes only the left spine of a top-level alternative.
    // Iteration avoids adding recursion for long pattern lists.
    let mut parts = Vec::new();
    while let Some(alt) = pattern.as_alternation_pattern_node() {
        parts.push(String::from_utf8_lossy(alt.right().location().as_slice()).into_owned());
        pattern = alt.left();
    }
    parts.push(String::from_utf8_lossy(pattern.location().as_slice()).into_owned());
    parts.reverse();
    parts.join(" | ")
}

#[cfg(test)]
mod tests {
    //! Ported RuboCop in_pattern_then_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};

    #[test]
    fn both_pending_rules_are_version_gated_before_ruby27() {
        use crate::cops::{Registry, policy::RubyVersion, select_active_for_version};
        let registry = Registry::new();
        for cop in ["Style/InPatternThen", "Style/MultilineInPatternThen"] {
            for (version, count) in [
                (RubyVersion(2, 6), 0),
                (RubyVersion(2, 7), 1),
                (RubyVersion(3, 4), 1),
            ] {
                assert_eq!(
                    select_active_for_version(
                        &registry,
                        &Default::default(),
                        &[cop.into()],
                        &[],
                        version
                    )
                    .len(),
                    count
                );
            }
        }
    }

    #[test]
    fn guards_pins_and_alternative_messages_use_the_pattern_not_the_guard() {
        for (pattern, guard, message_pattern) in [
            ("[x, ^pin]", " if x > 0", "[x, ^pin]"),
            ("{key: x}", " unless x.nil?", "{key: x}"),
            ("0|1|2|3", "", "0 | 1 | 2 | 3"),
            ("Integer => x", " if x.positive?", "Integer => x"),
        ] {
            let source = format!("pin = 2\ncase value\nin {pattern}{guard}; work\nend\n");
            let found = correction_offenses(&InPatternThen, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (3, (4 + pattern.len() + guard.len()) as u32, 3)
            );
            assert_eq!(
                found[0].message,
                format!(
                    "Do not use `in {message_pattern};`. Use `in {message_pattern} then` instead."
                )
            );
            assert_correction(
                &InPatternThen,
                &source,
                &source.replace("; work", " then work"),
            );
        }
    }

    #[test]
    fn ignores_multiline_empty_and_non_branch_lookalikes() {
        for source in [
            "case x\nin a;\n work\nend",
            "case x\nin a; work\n other\nend",
            "case x\nin [a,\n b]; work\nend",
            "case x\nin a; end",
            "case x\nin a\nend",
            "case x\nin a then work; other\nend",
            "case x; when a; work; end",
            "x in [a]; work",
            "x => [a]; work",
            "for x in values; work; end",
            "# in a; work\n'in a; work'",
        ] {
            assert!(
                correction_offenses(&InPatternThen, source).is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn only_the_separator_changes_in_comments_literals_and_source_envelopes() {
        let source = "\u{feff}# coding: utf-8\r\ncase value\r\nin [\"é;\", x]; [x, __LINE__] # keep\r\nend\r\n__END__\r\npayload\x00\r\n";
        assert_correction(&InPatternThen, source, &source.replace("; [x", " then [x"));
        for whitespace in [" ", "\t", "\x0b", "\x0c", "\r"] {
            let source = format!("case value\nin x{whitespace}; work\nend");
            assert_correction(
                &InPatternThen,
                &source,
                &source.replace("; work", " then work"),
            );
        }
    }

    #[test]
    fn selector_replacement_safety_guards_keep_findings() {
        for source in [
            "case value\nin x; DATA.pos\nend\n__END__\npayload\n",
            "case value\nin x; work # rubocop:disable Lint/Debugger\nend\n",
            "case value\nin x; work\nend\np File.read(__FILE__)\n",
        ] {
            let found = correction_offenses(&InPatternThen, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn wide_disjoint_patterns_use_bounded_source_work_and_lazy_corrections() {
        let source = "case value\nin [x, y]; work(x, y)\nend\n".repeat(6000);
        let found = crate::pipeline::lint_str_with(&[&InPatternThen], &source);
        assert_eq!(found.len(), 6000);
        assert!(found.iter().all(|o| o.correction.is_none()));
        let found = correction_offenses(&InPatternThen, &source);
        assert_eq!(
            found.iter().filter(|o| o.correction.is_some()).count(),
            6000
        );
        let result = crate::corrections::apply(source.as_bytes(), &found).unwrap();
        assert_eq!(
            result.bytes,
            source.replace("; work", " then work").as_bytes()
        );
    }
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/in_pattern_then_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 7);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/InPatternThen")
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "{c}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{c}"
            );
            if let Some(after) = c["correction"].as_str() {
                assert_correction(&InPatternThen, source, after);
            }
        }
    }
}
