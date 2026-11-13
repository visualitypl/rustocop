//! `Lint/ElseLayout` — suspicious same-line else bodies.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ElseLayout;
impl Cop for ElseLayout {
    fn name(&self) -> &'static str {
        "Lint/ElseLayout"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_else_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(other) = node.as_else_node() else {
            return;
        };
        if other.else_keyword_loc().as_slice() != b"else" {
            return;
        }
        let Some(parent) = cx.parent() else { return };
        let (keyword, then, end) = if let Some(n) = parent.as_if_node() {
            let Some(keyword) = n.if_keyword_loc() else {
                return;
            };
            (keyword, n.then_keyword_loc(), n.end_keyword_loc())
        } else if let Some(n) = parent.as_unless_node() {
            (n.keyword_loc(), n.then_keyword_loc(), n.end_keyword_loc())
        } else {
            return;
        };
        let Some(statements) = other.statements() else {
            return;
        };
        let Some(mut first) = statements.body().iter().next() else {
            return;
        };
        // RuboCop's implicit begin and explicit parenthesized begin are distinct
        // from kwbegin. Peel exactly one begin, not arbitrary nested grouping.
        let begin = statements.body().len() > 1 || first.as_parentheses_node().is_some();
        if statements.body().len() == 1
            && let Some(group) = first.as_parentheses_node()
        {
            let Some(child) = group
                .body()
                .and_then(|s| s.as_statements_node())
                .and_then(|s| s.body().iter().next())
            else {
                return;
            };
            first = child;
        }
        let end = if keyword.as_slice() == b"elsif" {
            statements.location().end_offset()
        } else {
            end.map_or(parent.location().end_offset(), |l| l.end_offset())
        };
        let single_line =
            file.line_of(keyword.start_offset()) == file.line_of(end.saturating_sub(1));
        // parser_prism inherits the preceding clause's `then` location for an
        // elsif without its own. The upstream recursive check goes only from
        // a real `if`, not from an `elsif`. Use the actual ancestor edges.
        let mut has_then = then.is_some();
        if keyword.as_slice() == b"elsif" && !has_then {
            for ancestor in cx.ancestors().skip(1) {
                let Some(n) = ancestor.as_if_node() else {
                    break;
                };
                if n.then_keyword_loc().is_some() {
                    has_then = true;
                    break;
                }
                if n.if_keyword_loc().is_none_or(|l| l.as_slice() != b"elsif") {
                    break;
                }
            }
        }
        let recursive = keyword.as_slice() == b"elsif"
            && cx
                .ancestors()
                .nth(1)
                .and_then(|a| a.as_if_node())
                .is_some_and(|n| {
                    n.if_keyword_loc().is_some_and(|l| l.as_slice() == b"if")
                        && n.then_keyword_loc().is_none()
                        && file.line_of(n.location().start_offset())
                            != file.line_of(n.location().end_offset().saturating_sub(1))
                });
        if (single_line || has_then && !begin) && !recursive {
            return;
        }
        let start = first.location().start_offset();
        if file.line_of(start) != file.line_of(other.else_keyword_loc().start_offset()) {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(first.location().end_offset().saturating_sub(1)),
            message: "Odd `else` layout detected. Did you mean to use `elsif`?".into(),
            correction: cx
                .corrections_requested()
                .then(|| inert_else(cx, file, node, start))
                .flatten(),
        });
    }
}
// Moving evaluated expressions changes trace/exception locations. Prove only
// an immediate top-level constant conditional with an unexecuted literal else.
// The inherited source/position/code-object/heap guards still filter this edit.
fn inert_else(
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    node: &Node<'_>,
    first: usize,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let parent = ancestors.next()?.as_if_node()?;
    if parent.if_keyword_loc()?.as_slice() != b"if" || parent.predicate().as_true_node().is_none() {
        return None;
    }
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || top.body().len() != 1 {
        return None;
    }
    let inert = |n: Node<'_>| {
        matches!(
            n,
            Node::NilNode { .. } | Node::TrueNode { .. } | Node::FalseNode { .. }
        )
    };
    if parent
        .statements()
        .is_some_and(|s| s.body().iter().any(|n| !inert(n)))
    {
        return None;
    }
    let other = node.as_else_node()?;
    // Even dead literal sequences emit compile-time void-context warnings;
    // moving their first literal changes the warning's physical source line.
    if other.statements()?.body().len() != 1 || other.statements()?.body().iter().any(|n| !inert(n))
    {
        return None;
    }
    // Do not relocate same-line comments, directives or pragmas with a body.
    let comments = file.comments();
    let index = comments.partition_point(|r| r.end <= parent.location().start_offset());
    if comments
        .get(index)
        .is_some_and(|r| r.start < parent.location().end_offset())
    {
        return None;
    }
    let start = other.else_keyword_loc().end_offset();
    let gap = file.bytes.get(start..first)?;
    if gap.is_empty() || !gap.iter().all(|b| matches!(b, b' ' | b'\t')) {
        return None;
    }
    let (_, col) =
        super::array_literal_in_regexp::reference_line_col(parent.location().start_offset(), file);
    let newline = if file.bytes.windows(2).any(|b| b == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    Some(crate::offense::Correction {
        range: start..first,
        replacement: format!("{newline}{}", " ".repeat(col as usize + 1)),
    })
}
#[cfg(test)]
mod tests {
    //! Ported RuboCop lint/else_layout_spec.rb examples (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    #[test]
    fn canonical_unexecuted_else_edits_and_ordinary_holds() {
        for (before, after) in [
            (
                "if true\n  nil\nelse false\nend\n",
                "if true\n  nil\nelse\n  false\nend\n",
            ),
            (
                "if true\n  nil\nelse false; nil\nend\n",
                "if true\n  nil\nelse false; nil\nend\n",
            ),
            (
                "if unknown\n  nil\nelse false\nend\n",
                "if unknown\n  nil\nelse false\nend\n",
            ),
            (
                "if false\n  nil\nelse work\nend\n",
                "if false\n  nil\nelse work\nend\n",
            ),
        ] {
            let offenses = crate::cops::testing::correction_offenses(&super::ElseLayout, before);
            assert_eq!(offenses.len(), 1);
            assert_eq!(
                crate::corrections::apply(before.as_bytes(), &offenses)
                    .unwrap()
                    .bytes,
                after.as_bytes()
            );
        }
    }
    #[test]
    fn pinned_examples_and_source_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/else_attribute_layout.json"
        ))
        .unwrap();
        for row in rows.into_iter().filter(|r| r["cop"] == "Lint/ElseLayout") {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            let Ok(source) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Lint/ElseLayout")
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            assert_eq!(found, expected, "{row}");
        }
    }
}
