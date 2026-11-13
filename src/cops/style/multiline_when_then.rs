//! `Style/MultilineWhenThen` — fixed-default multiline conditional style.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MultilineWhenThen;
impl Cop for MultilineWhenThen {
    fn name(&self) -> &'static str {
        "Style/MultilineWhenThen"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_when_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(n) = node.as_when_node() else {
            return;
        };
        let Some(then) = n.then_keyword_loc() else {
            return;
        };
        let conditions = n.conditions();
        let Some(first) = conditions.iter().next() else {
            return;
        };
        let last = conditions.iter().last().unwrap();
        if file.line_of(first.location().start_offset()) != file.last_line_of(last.location())
            || n.statements().is_some_and(|b| {
                file.line_of(n.keyword_loc().start_offset())
                    == file.line_of(b.location().start_offset())
            })
        {
            return;
        }
        // A trailing endless range needs `then` as its grammatical delimiter,
        // including a range nested in a splat or other condition expression.
        if super::multiline_if_then::ends_with_open_range(&last) {
            return;
        }
        let (line, col) = file.line_col(then.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Do not use `then` for multiline `when` statement.".into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    // An escaped physical newline is not a statement boundary.
                    // Deleting `then` can bind a unary body to the condition
                    // (e.g. `true -1`), even when the result still parses.
                    let gap = &file.bytes[last.location().end_offset()..then.start_offset()];
                    if gap.windows(2).any(|w| w == b"\\\n")
                        || gap.windows(3).any(|w| w == b"\\\r\n")
                    {
                        return None;
                    }
                    super::multiline_if_then::remove_then(then, false, cx, file)
                })
                .flatten(),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's multiline_when_then_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    #[test]
    fn multiline_conditions_and_trailing_ranges_keep_required_delimiters() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "case x\nwhen first,\n second then\n work\nend\n",
            "case x\nwhen 1.. then\n work\nend\n",
            "case x\nwhen *1.. then\n work\nend\n",
            "case x\nwhen [\n 1\n] then\n work\nend\n",
            "case x\nwhen one then [\n 1\n]\nend\n",
            "case x\nin Integer then\n work\nend\n",
        ] {
            assert!(
                correction_offenses(&MultilineWhenThen, source).is_empty(),
                "{source}"
            );
        }
        assert_correction(
            &MultilineWhenThen,
            "case x\nwhen (1..) then\n work\nend\n",
            "case x\nwhen (1..)\n work\nend\n",
        );
    }
    #[test]
    fn escaped_newlines_keep_condition_and_body_delimited() {
        for nl in ["\n", "\r\n"] {
            for body in ["-1", "+1", "work", "\n -1", ""] {
                let source =
                    format!("case true\nwhen true \\\n  then {body}\nend\n").replace('\n', nl);
                for found in [
                    crate::pipeline::lint_str_with(&[&MultilineWhenThen], &source),
                    crate::cops::testing::correction_offenses(&MultilineWhenThen, &source),
                ] {
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 3));
                    assert_eq!(
                        found[0].message,
                        "Do not use `then` for multiline `when` statement."
                    );
                    assert!(found[0].correction.is_none(), "{source}");
                }
            }
        }
    }
    #[test]
    fn real_newline_and_literal_continuations_still_allow_corrections() {
        for nl in ["\n", "\r\n"] {
            for (before, after) in [
                (
                    "case true\nwhen true\n  then -1\nend\n",
                    "case true\nwhen true\n -1\nend\n",
                ),
                (
                    "case true\nwhen true then\n -1\nend\n",
                    "case true\nwhen true\n -1\nend\n",
                ),
                // A literal backslash/newline escape is not a physical continuation.
                (
                    "case true\nwhen \"a\\n\"\n  then -1\nend\n",
                    "case true\nwhen \"a\\n\"\n -1\nend\n",
                ),
            ] {
                assert_correction(
                    &MultilineWhenThen,
                    &before.replace('\n', nl),
                    &after.replace('\n', nl),
                );
            }
        }
    }
    #[test]
    fn empty_inline_branch_retains_its_only_separator() {
        let source = "case value\nwhen one then end\n";
        let found = crate::cops::testing::correction_offenses(&MultilineWhenThen, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn comments_line_values_and_crlf_preserve_physical_lines() {
        assert_correction(
            &MultilineWhenThen,
            "\u{feff}# coding: utf-8\r\ncase x\r\nwhen \"é\" then # why\r\n p __LINE__\r\nwhen two\r\n  then work # next\r\nend\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\ncase x\r\nwhen \"é\" # why\r\n p __LINE__\r\nwhen two\r\n work # next\r\nend\r\n__END__\r\npayload\x00\r\n",
        );
    }
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/multiline_when_then_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 14);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/MultilineWhenThen")
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                case["positions"],
                "{case}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                case["messages"],
                "{case}"
            );
            if let Some(after) = case["correction"].as_str() {
                assert_correction(&MultilineWhenThen, source, after);
            }
        }
    }
}
