//! `Style/WhenThen` — use `then` instead of a same-line when separator.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct WhenThen;
impl Cop for WhenThen {
    fn name(&self) -> &'static str {
        "Style/WhenThen"
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
        let Some(n) = node.as_when_node() else { return };
        let Some(body) = n.statements() else { return };
        if n.then_keyword_loc().is_some()
            || file.line_of(n.keyword_loc().start_offset()) != file.last_line_of(body.location())
        {
            return;
        }
        let conditions = n.conditions();
        let Some(last) = conditions.iter().last() else {
            return;
        };
        let gap = last.location().end_offset()..body.location().start_offset();
        // The parser owns the conditions/body boundary; a semicolon in a
        // literal or body statement must never be mistaken for this token.
        let Some(relative) = file.bytes[gap.clone()]
            .iter()
            // Ruby also treats vertical tabs as whitespace, unlike Rust's
            // is_ascii_whitespace predicate.
            .position(|b| !matches!(b, b' ' | b'\t'..=b'\r'))
        else {
            return;
        };
        let at = gap.start + relative;
        if file.bytes[at] != b';' {
            return;
        }
        let expression = conditions
            .iter()
            .map(|n| String::from_utf8_lossy(n.location().as_slice()).into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        let (line, col) = file.line_col(at);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!(
                "Do not use `when {expression};`. Use `when {expression} then` instead."
            ),
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
#[cfg(test)]
mod tests {
    //! RuboCop when_then_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/when_then_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 4);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/WhenThen")
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
                assert_correction(&WhenThen, source, after);
            }
        }
    }
    #[test]
    fn ruby_whitespace_before_separator_is_preserved() {
        for whitespace in [" ", "\t", "\x0b", "\x0c", "\r"] {
            let source = format!("case x\nwhen true{whitespace}; y\nend\n");
            let found: Vec<_> = crate::pipeline::lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Style/WhenThen")
                .collect();
            assert_eq!(found.len(), 1, "whitespace: {whitespace:?}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 11, 2));
            assert_eq!(
                found[0].message,
                "Do not use `when true;`. Use `when true then` instead."
            );
            assert_correction(
                &WhenThen,
                &source,
                &format!("case x\nwhen true{whitespace} then y\nend\n"),
            );
            assert!(
                correction_offenses(
                    &WhenThen,
                    &format!("case x\nwhen true{whitespace}then y\nend\n"),
                )
                .is_empty()
            );
        }
    }
    #[test]
    fn tokens_literals_comments_crlf_and_line_values_survive() {
        assert_correction(
            &WhenThen,
            "\u{feff}# coding: utf-8\r\ncase x\r\nwhen \"é;\", :x; work; other # keep\r\nwhen __LINE__; :line\r\nend\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\ncase x\r\nwhen \"é;\", :x then work; other # keep\r\nwhen __LINE__ then :line\r\nend\r\n__END__\r\npayload\x00\r\n",
        );
        assert_correction(
            &WhenThen,
            "case; when x; y; end",
            "case; when x then y; end",
        );
        let found = correction_offenses(
            &WhenThen,
            "case x\nwhen :a; work # rubocop:disable Lint/Debugger\nend",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn multiline_bodies_conditions_and_empty_branches_are_not_single_line_when() {
        for s in [
            "case x; when a; end",
            "case x\nwhen a;\n b\nend",
            "case x\nwhen a; b\n c\nend",
            "case x\nwhen a,\n b; c\nend",
            "case x\nin Integer; 1\nend",
            "'when a; b'",
        ] {
            assert!(correction_offenses(&WhenThen, s).is_empty(), "{s}");
        }
    }
}
