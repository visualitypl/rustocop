//! `Style/ParenthesesAroundCondition` — fixed safe-assignment and multiline policy.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ParenthesesAroundCondition;
impl Cop for ParenthesesAroundCondition {
    fn name(&self) -> &'static str {
        "Style/ParenthesesAroundCondition"
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
        // Prism's pattern guards are If/Unless nodes directly below InNode;
        // RuboCop uses distinct if_guard/unless_guard nodes, not on_if callbacks.
        if cx
            .parent()
            .is_some_and(|parent| parent.as_in_node().is_some())
        {
            return;
        }
        let (condition, keyword, is_loop) = if let Some(n) = node.as_if_node() {
            let Some(k) = n.if_keyword_loc() else { return };
            (n.predicate(), k.as_slice(), false)
        } else if let Some(n) = node.as_unless_node() {
            (n.predicate(), b"unless".as_slice(), false)
        } else if let Some(n) = node.as_while_node() {
            if n.is_begin_modifier() {
                return;
            }
            (n.predicate(), b"while".as_slice(), true)
        } else if let Some(n) = node.as_until_node() {
            if n.is_begin_modifier() {
                return;
            }
            (n.predicate(), b"until".as_slice(), true)
        } else {
            return;
        };
        let Some(p) = condition.as_parentheses_node() else {
            return;
        };
        let Some(body) = p.body().and_then(|b| b.as_statements_node()) else {
            return;
        };
        let Some(first) = body.body().first() else {
            return;
        };
        let loc = condition.location();
        // The pin exempts lexical keyword adjacency (if(...), until(...)).
        if loc
            .start_offset()
            .checked_sub(1)
            .and_then(|i| file.bytes.get(i))
            .is_some_and(u8::is_ascii_lowercase)
            || file
                .bytes
                .get(loc.end_offset())
                .is_some_and(u8::is_ascii_lowercase)
            || modifier(&first)
            || body.body().iter().nth(1).is_some_and(|second| {
                file.bytes[first.location().end_offset()..second.location().start_offset()]
                    .contains(&b';')
            })
            || (body.body().len() == 1 && safe_assignment(&first))
            || (is_loop && keyword_block(&first))
        {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                let range = loc.start_offset()..loc.end_offset();
                // Newline-separated statements need grouping even though the pin
                // reports them. Never turn condition effects into branch effects.
                if range.len() > 16 * 1024
                    || body.body().len() != 1
                    // A post-test loop can escape a modifier guard when unwrapped;
                    // loop headers can also require these delimiters.
                    || post_test_loop(&first)
                    || super::multiline_if_then::ends_with_open_range(&first)
                    || super::correction::contains_comment(file, range.clone())
                {
                    return None;
                }
                let root = cx.ancestors().next_back()?;
                if file.contains_heredoc(root, &condition)
                    || file.heredoc_pending_at(root, range.start)
                {
                    return None;
                }
                #[cfg(test)]
                REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
                let replacement = std::str::from_utf8(first.location().as_slice())
                    .ok()?
                    .to_owned();
                super::negated_if::finish(Correction { range, replacement }, cx, file)
            })
            .flatten();
        let keyword = String::from_utf8_lossy(keyword);
        let article = if keyword == "while" { "a" } else { "an" };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: format!(
                "Don't use parentheses around the condition of {article} `{keyword}`."
            ),
            correction,
        });
    }
}
fn keyword_block(n: &Node<'_>) -> bool {
    if let Some(lambda) = n.as_lambda_node() {
        return lambda.opening_loc().as_slice() == b"do";
    }
    n.as_call_node()
        .and_then(|c| c.block())
        .or_else(|| n.as_super_node().and_then(|c| c.block()))
        .or_else(|| {
            n.as_forwarding_super_node()
                .and_then(|c| c.block())
                .map(|b| b.as_node())
        })
        .and_then(|b| b.as_block_node())
        .is_some_and(|b| b.opening_loc().as_slice() == b"do")
}
fn safe_assignment(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::MultiWriteNode { .. }
    ) || n.as_call_node().is_some_and(|c| c.equal_loc().is_some())
}
fn post_test_loop(n: &Node<'_>) -> bool {
    n.as_while_node().is_some_and(|n| n.is_begin_modifier())
        || n.as_until_node().is_some_and(|n| n.is_begin_modifier())
}
fn modifier(n: &Node<'_>) -> bool {
    n.as_rescue_modifier_node().is_some()
        || n.as_if_node()
            .is_some_and(|n| n.if_keyword_loc().is_some() && n.end_keyword_loc().is_none())
        || n.as_unless_node()
            .is_some_and(|n| n.end_keyword_loc().is_none())
        || n.as_while_node()
            .is_some_and(|n| n.closing_loc().is_none() && !n.is_begin_modifier())
        || n.as_until_node()
            .is_some_and(|n| n.closing_loc().is_none() && !n.is_begin_modifier())
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! RuboCop parentheses_around_condition_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn pattern_guards_are_not_if_callbacks_but_nested_real_conditions_are() {
        for source in [
            "case x; in y if (a or b); end\n",
            "case x; in y unless (a and b); end\n",
            "case x; in y if (ready); work; end\n",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&ParenthesesAroundCondition, source)
                    .is_empty(),
                "{source}"
            );
        }
        let found = crate::pipeline::lint_str_with(
            &[&ParenthesesAroundCondition],
            "case x; in y if (ready); work if (active); end\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 34));
        assert_eq!(
            found[0].message,
            "Don't use parentheses around the condition of an `if`."
        );
    }
    #[test]
    fn required_open_range_delimiters_and_multi_expression_effects_are_not_removed() {
        for source in ["if (1..)\n  work\nend\n", "if (foo\nbar)\nwork\nend\n"] {
            let found =
                crate::cops::testing::correction_offenses(&ParenthesesAroundCondition, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn post_test_loop_conditions_are_reported_without_ambiguous_edits() {
        for keyword in ["while", "until"] {
            for (source, col, outer, article) in [
                (
                    format!("if (begin work end {keyword} ready); action; end\n"),
                    4,
                    "if",
                    "an",
                ),
                (
                    format!("action if (begin work end {keyword} ready)\n"),
                    11,
                    "if",
                    "an",
                ),
                (
                    format!("while (begin work end {keyword} ready); action; end\n"),
                    7,
                    "while",
                    "a",
                ),
            ] {
                let found = crate::pipeline::lint_str_with(&[&ParenthesesAroundCondition], &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, col));
                assert_eq!(
                    found[0].message,
                    format!("Don't use parentheses around the condition of {article} `{outer}`.")
                );
                let found =
                    crate::cops::testing::correction_offenses(&ParenthesesAroundCondition, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }

            let modifier = format!("if (work {keyword} ready); action; end\n");
            assert!(
                crate::pipeline::lint_str_with(&[&ParenthesesAroundCondition], &modifier)
                    .is_empty(),
                "{modifier}"
            );
        }
    }
    #[test]
    fn assignment_variants_nested_guards_and_post_loops() {
        for source in [
            "if (x = 1); end\n",
            "if (@x = 1); end\n",
            "if (obj[0] = 1); end\n",
            "if (obj.x = 1); end\n",
            "if (x,y = values); end\n",
            "begin work end while (ready)\n",
            "if (work rescue fallback); end\n",
            "if (); end\n",
            "if(foo); end\n",
            "while (foo do work end); end\n",
            "def f; while (super do 1 end); work; end; end\n",
            "def f; until (super() do 1 end); work; end; end\n",
            "while (-> do 1 end); work; end\n",
            "if (work if ready); end\n",
            "if (one; two); end\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&ParenthesesAroundCondition], source).is_empty(),
                "{source}"
            );
        }
        for (source, after) in [
            ("if (x ||= 1); end\n", "if x ||= 1; end\n"),
            ("if ((x = 1)); end\n", "if (x = 1); end\n"),
            (
                "if (a); if (b); work; end; end\n",
                "if a; if b; work; end; end\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&ParenthesesAroundCondition, source, after);
        }
    }
    #[test]
    fn ambiguous_comments_heredocs_and_observed_sources_stay_reported() {
        for source in [
            "if (a && # keep\n b); end\n",
            "if (\n __LINE__\n); end\n",
            "if (\n a\n); end\np __LINE__\n",
            "if (DATA.pos); end\n__END__\npayload\n",
            "if (method(:f).source_location); end\n",
            "if (<<~H); end\nbody\nH\n",
            "if (a); end # rubocop:disable Lint/Debugger\n",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&ParenthesesAroundCondition, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn source_envelopes_and_literal_interiors_are_verbatim() {
        crate::cops::testing::assert_correction(
            &ParenthesesAroundCondition,
            "\u{feff}# coding: utf-8\r\nif (\"é\r\n  value\"); end\r\np __LINE__\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\nif \"é\r\n  value\"; end\r\np __LINE__\r\n__END__\r\npayload\x00\r\n",
        );
    }
    #[test]
    fn correction_source_work_is_lazy_and_bounded() {
        let source = "work if (ready)\n".repeat(6000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&ParenthesesAroundCondition], &source).len(),
            6000
        );
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        assert!(
            crate::cops::testing::correction_offenses(&ParenthesesAroundCondition, &source)
                .iter()
                .all(|o| o.correction.is_some())
        );
        assert!(REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let huge = format!("if ([{}]); end\n", "1,".repeat(10000));
        assert!(
            crate::cops::testing::correction_offenses(&ParenthesesAroundCondition, &huge)[0]
                .correction
                .is_none()
        );
    }
    #[test]
    fn pinned_diagnostics_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/parentheses_around_condition_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 26);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == ParenthesesAroundCondition.name())
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
                crate::cops::testing::assert_correction(&ParenthesesAroundCondition, source, after);
            }
        }
    }
}
