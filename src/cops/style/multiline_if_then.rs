//! `Style/MultilineIfThen` — fixed-default multiline conditional style.
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct MultilineIfThen;
impl Cop for MultilineIfThen {
    fn name(&self) -> &'static str {
        "Style/MultilineIfThen"
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
        let (keyword, then, body) = if let Some(n) = node.as_if_node() {
            let (Some(keyword), Some(then)) = (n.if_keyword_loc(), n.then_keyword_loc()) else {
                return;
            };
            (keyword, then, n.statements())
        } else if let Some(n) = node.as_unless_node() {
            let Some(then) = n.then_keyword_loc() else {
                return;
            };
            (n.keyword_loc(), then, n.statements())
        } else {
            return;
        };
        let loc = node.location();
        if then.as_slice() != b"then"
            || then.start_offset() < loc.start_offset()
            || file.line_of(loc.start_offset()) == file.last_line_of(loc)
            || body.is_some_and(|b| {
                file.line_of(b.location().start_offset()) == file.line_of(then.start_offset())
            })
        {
            return;
        }
        let (line, col) = file.line_col(then.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!(
                "Do not use `then` for multi-line `{}`.",
                String::from_utf8_lossy(keyword.as_slice())
            ),
            correction: cx
                .corrections_requested()
                .then(|| {
                    let predicate = node
                        .as_if_node()
                        .map(|n| n.predicate())
                        .or_else(|| node.as_unless_node().map(|n| n.predicate()))?;
                    // `then` terminates an open range: deleting it can consume
                    // the first body expression as the range's right operand.
                    if ends_with_open_range(&predicate) {
                        return None;
                    }
                    remove_then(then, true, cx, file)
                })
                .flatten(),
        });
    }
}

/// Detect an unparenthesized trailing range without scanning unrelated source.
pub(super) fn ends_with_open_range(node: &Node<'_>) -> bool {
    struct OpenRange {
        end: usize,
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for OpenRange {
        fn enter(&mut self, n: Node<'pr>) {
            self.found |= n.location().end_offset() == self.end
                && (n.as_range_node().is_some_and(|r| r.right().is_none())
                    || n.as_flip_flop_node().is_some_and(|r| r.right().is_none()));
        }
    }
    let mut check = OpenRange {
        end: node.location().end_offset(),
        found: false,
    };
    walk(&mut check, node);
    check.found
}

/// Token deletion shared with WhenThen. Only whitespace immediately to its left
/// is touched; keep a separator when code follows a standalone empty `then`.
pub(super) fn remove_then(
    then: Location<'_>,
    newlines: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let mut start = then.start_offset();
    while start > 0
        && then.start_offset() - start < 16 * 1024
        && if newlines {
            super::is_ruby_whitespace(file.bytes[start - 1])
        } else {
            matches!(file.bytes[start - 1], b' ' | b'\t' | b'\x0b' | b'\x0c')
        }
    {
        start -= 1;
    }
    if then.start_offset() - start == 16 * 1024
        // Removing an escaped newline leaves the first body expression on the
        // predicate's logical line. Keep the delimiter rather than change binding.
        || (start > 0
            && file.bytes[start - 1] == b'\\'
            && file.bytes[start..then.start_offset()].contains(&b'\n'))
    {
        return None;
    }
    let end = then.end_offset();
    let suffix = &file.bytes[end..file.line_start(file.line_of(end) + 1)];
    let comments = file.comments();
    let previous_comment = comments.partition_point(|c| c.end <= start);
    let follows_comment = previous_comment > 0 && comments[previous_comment - 1].end == start;
    if file.bytes[start..then.start_offset()].contains(&b'\n')
        && (follows_comment
            || (!suffix.trim_ascii().is_empty() && !suffix.trim_ascii_start().starts_with(b"#")))
    {
        start = file.line_start(file.line_of(then.start_offset()));
    }
    // With an empty inline body, `then` can be the only separator before
    // `end`/`else`/`when`. Upstream's deletion can be invalid Ruby; retain the
    // finding rather than rely on the final parser to reject the whole file.
    if !suffix.trim_ascii().is_empty()
        && !suffix.trim_ascii_start().starts_with(b"#")
        && !suffix.trim_ascii_start().starts_with(b";")
        && start != file.line_start(file.line_of(then.start_offset()))
    {
        return None;
    }
    #[cfg(test)]
    REMOVE_BYTES.with(|n| n.set(n.get() + end - start));
    let edit = Correction {
        range: start..end,
        replacement: String::new(),
    };
    let root = cx.ancestors().next_back()?;
    if file.bytes[edit.range.clone()].contains(&b'\n') && file.heredoc_pending_at(root, start) {
        return None;
    }
    Some(edit)
}
#[cfg(test)]
thread_local! { static REMOVE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's multiline_if_then_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    #[test]
    fn preserves_endless_predicate_delimiters_and_separate_comments() {
        use crate::cops::testing::correction_offenses;
        for keyword in ["if", "unless"] {
            let source = format!("{keyword} 1.. then\n  work\nend\n");
            let found = correction_offenses(&MultilineIfThen, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
            assert_correction(
                &MultilineIfThen,
                &format!("{keyword} cond\nthen end\n"),
                &format!("{keyword} cond\n end\n"),
            );
            assert_correction(
                &MultilineIfThen,
                &format!("{keyword} cond # first\n  then # second\nend\n"),
                &format!("{keyword} cond # first\n # second\nend\n"),
            );
        }
    }
    #[test]
    fn explicit_continuation_retains_then_separator() {
        for keyword in ["if true", "unless false", "if false\n  :unused\nelsif true"] {
            for nl in ["\n", "\r\n"] {
                let source = format!("{keyword} \\\n  then\n -1\nend\n").replace('\n', nl);
                let found = crate::cops::testing::correction_offenses(&MultilineIfThen, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn empty_inline_branch_retains_its_only_separator() {
        let source = "if (ready # condition\n) then end\n";
        let found = crate::cops::testing::correction_offenses(&MultilineIfThen, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn whitespace_work_is_lazy_bounded_and_disjoint() {
        use crate::cops::testing::correction_offenses;
        let source = "if cond then\nend\n".repeat(6000);
        REMOVE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&MultilineIfThen], &source).len(),
            6000
        );
        assert_eq!(REMOVE_BYTES.with(|n| n.get()), 0);
        assert_eq!(
            correction_offenses(&MultilineIfThen, &source)
                .iter()
                .filter(|o| o.correction.is_some())
                .count(),
            6000
        );
        assert!(REMOVE_BYTES.with(|n| n.get()) <= source.len());
        let source = format!("if cond{}then\nend\n", " ".repeat(20000));
        let found = correction_offenses(&MultilineIfThen, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/multiline_if_then_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 14);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/MultilineIfThen")
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
                assert_correction(&MultilineIfThen, source, after);
            }
        }
    }
}
