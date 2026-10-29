//! `Style/MultilineMemoization` — fixed `keyword` wrapping for memoization.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MultilineMemoization;
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
impl Cop for MultilineMemoization {
    fn name(&self) -> &'static str {
        "Style/MultilineMemoization"
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
        let value = match node {
            Node::LocalVariableOrWriteNode { .. } => {
                node.as_local_variable_or_write_node().unwrap().value()
            }
            Node::InstanceVariableOrWriteNode { .. } => {
                node.as_instance_variable_or_write_node().unwrap().value()
            }
            Node::ClassVariableOrWriteNode { .. } => {
                node.as_class_variable_or_write_node().unwrap().value()
            }
            Node::GlobalVariableOrWriteNode { .. } => {
                node.as_global_variable_or_write_node().unwrap().value()
            }
            Node::ConstantOrWriteNode { .. } => node.as_constant_or_write_node().unwrap().value(),
            Node::ConstantPathOrWriteNode { .. } => {
                node.as_constant_path_or_write_node().unwrap().value()
            }
            Node::CallOrWriteNode { .. } => node.as_call_or_write_node().unwrap().value(),
            Node::IndexOrWriteNode { .. } => node.as_index_or_write_node().unwrap().value(),
            _ => return,
        };
        let Some(group) = value.as_parentheses_node() else {
            return;
        };
        if file.line_of(value.location().start_offset()) == file.last_line_of(value.location()) {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Copy only a bounded local span, and never move a pending heredoc's
                // boundary. The shared finish proof protects LINE/DATA/source reads.
                if value.location().end_offset() - value.location().start_offset() > 16 * 1024 {
                    return None;
                }
                let root = cx.ancestors().next_back()?;
                if file.contains_heredoc(root, &value)
                    || file.heredoc_pending_at(root, value.location().start_offset())
                {
                    return None;
                }
                let opening = group.opening_loc();
                let closing = group.closing_loc();
                let newline = if file.bytes.get(
                    file.line_start(file.line_of(opening.start_offset()) + 1)
                        .saturating_sub(2)
                        ..file.line_start(file.line_of(opening.start_offset()) + 1),
                ) == Some(b"\r\n")
                {
                    "\r\n"
                } else {
                    "\n"
                };
                let column = file.line_col(opening.start_offset()).1 as usize - 1;
                let closing_line = &file.bytes[file.line_start(file.line_of(closing.start_offset()))
                    ..file.line_start(file.line_of(closing.start_offset()) + 1)];
                if column > 16 * 1024 || closing_line.len() > 16 * 1024 {
                    return None;
                }
                #[cfg(test)]
                REWRITE_BYTES.with(|n| {
                    n.set(
                        n.get() + value.location().end_offset() - value.location().start_offset()
                            + closing_line.len(),
                    )
                });
                let begin = if matches!(file.bytes.get(opening.end_offset()), Some(b'\n' | b'\r')) {
                    "begin".into()
                } else {
                    format!("begin{newline}{}", " ".repeat(column + 2))
                };
                let end = if closing_line
                    .iter()
                    .any(|b| !b.is_ascii_whitespace() && *b != b')')
                {
                    format!("{newline}{}end", " ".repeat(column))
                } else {
                    "end".into()
                };
                let edit = super::correction::combine(
                    file,
                    vec![
                        Correction {
                            range: opening.start_offset()..opening.end_offset(),
                            replacement: begin,
                        },
                        Correction {
                            range: closing.start_offset()..closing.end_offset(),
                            replacement: end,
                        },
                    ],
                )?;
                super::negated_if::finish(edit, cx, file)
            })
            .flatten();
        super::negated_if::report(
            self,
            node,
            file,
            "Wrap multiline memoization blocks in `begin` and `end`.".into(),
            correction,
            out,
        );
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/multiline_memoization_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn every_pinned_keyword_example_has_exact_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/multiline_memoization_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 9);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == MultilineMemoization.name())
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "{source}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{source}"
            );
        }
    }
    #[test]
    fn pinned_keyword_corrections_and_positions() {
        for (before, after) in [
            (
                "foo ||= (\n  bar\n  baz\n)\n",
                "foo ||= begin\n  bar\n  baz\nend\n",
            ),
            (
                "foo ||=\n  (\n    bar\n    baz\n  )\n",
                "foo ||=\n  begin\n    bar\n    baz\n  end\n",
            ),
            (
                "foo ||= (bar ||\n          baz)\n",
                "foo ||= begin\n          bar ||\n          baz\n        end\n",
            ),
        ] {
            let found: Vec<_> = lint_str(before)
                .into_iter()
                .filter(|o| o.cop == MultilineMemoization.name())
                .collect();
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                "Wrap multiline memoization blocks in `begin` and `end`."
            );
            assert_correction(&MultilineMemoization, before, after);
        }
    }
    #[test]
    fn all_or_write_shapes_preserve_lhs_and_only_change_wrappers() {
        for target in [
            "foo",
            "@foo",
            "@@foo",
            "$foo",
            "CONST",
            "A::CONST",
            "::CONST",
            "object.value",
            "object&.value",
            "object[index]",
            "factory()[index()]",
        ] {
            assert_correction(
                &MultilineMemoization,
                &format!("{target} ||= (\n  work\n)"),
                &format!("{target} ||= begin\n  work\nend"),
            );
        }
    }
    #[test]
    fn nested_parentheses_comments_and_crlf_are_preserved() {
        for (before, after) in [
            (
                "foo ||= (\n  (bar) # keep\n)\n",
                "foo ||= begin\n  (bar) # keep\nend\n",
            ),
            (
                "consume(foo ||= (\n  bar\n))\n",
                "consume(foo ||= begin\n  bar\nend)\n",
            ),
            (
                "foo ||= (\r\n  bar\r\n)\r\n",
                "foo ||= begin\r\n  bar\r\nend\r\n",
            ),
            (
                "foo ||= (# keep\n bar)\n",
                "foo ||= begin\n          # keep\n bar\n        end\n",
            ),
        ] {
            assert_correction(&MultilineMemoization, before, after);
        }
    }
    #[test]
    fn line_data_heredoc_and_directive_hazards_retain_findings_without_edits() {
        for source in [
            "foo ||= (bar ||\n __LINE__)\n",
            "foo ||= (bar ||\n baz)\np __LINE__\n",
            "foo ||= (\n <<~TEXT\n value\nTEXT\n)\n",
            "foo ||= (\n work # rustocop:disable Lint/Debugger\n)\n",
            "foo ||= (\n DATA.pos\n)\n__END__\npayload\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&MultilineMemoization, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &MultilineMemoization,
            "foo ||= (\n __LINE__\n)\np __LINE__\n",
            "foo ||= begin\n __LINE__\nend\np __LINE__\n",
        );
    }
    #[test]
    fn bounded_local_source_work_is_lazy_even_for_wide_lines() {
        let source = "foo ||= (\n  bar\n)\n".repeat(1000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            lint_str(&source)
                .iter()
                .filter(|o| o.cop == MultilineMemoization.name())
                .count(),
            1000
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        let found = crate::cops::testing::correction_offenses(&MultilineMemoization, &source);
        assert_eq!(found.len(), 1000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        REWRITE_BYTES.with(|n| assert!(n.get() <= source.len() * 2));
        for source in [
            format!("foo ||= (\n '{}'\n)", "x".repeat(20_000)),
            format!("{}foo ||= (bar ||\n baz)", " ".repeat(20_000)),
            format!("foo ||= (\n bar\n); #{}", "x".repeat(20_000)),
        ] {
            REWRITE_BYTES.with(|n| n.set(0));
            assert!(
                crate::cops::testing::correction_offenses(&MultilineMemoization, &source)[0]
                    .correction
                    .is_none()
            );
            REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "foo ||= bar",
            "foo ||=\n  bar",
            "foo ||= (bar)",
            "foo ||= bar.each do |b|\n b.baz\n b.bax\nend",
            "foo ||=\n bar.each do |b|\n b.baz\n b.bax\nend",
            "foo ||= if bar\n baz\nelse\n bax\nend",
            "foo ||=\n if bar\n baz\nelse\n bax\nend",
            "foo ||= begin\n bar\n baz\nend",
            "foo ||= begin\n bar\nrescue\n baz\nend",
            "foo ||= begin\n bar\nensure\n baz\nend",
            "foo &&= (\n bar\n baz\n)",
            "foo = (\n bar\n baz\n)",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == MultilineMemoization.name()),
                "{source}"
            );
        }
    }
}
