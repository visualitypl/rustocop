//! `Style/EmptyElse` — fixed `both`, `AllowComments: false`.

use crate::{
    cops::{Companions, Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

#[derive(Default)]
pub struct EmptyElse {
    missing_else: bool,
}

impl Cop for EmptyElse {
    fn name(&self) -> &'static str {
        "Style/EmptyElse"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            missing_else: companions.missing_else,
        }))
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        // Avoid creating a finding when CLI force-selects the opt-in companion
        // without enabling it in configuration, too.
        &["Style/MissingElse"]
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
        // ElseNode is also used by rescue and pattern matching. Only normal
        // if/unless and traditional case clauses have the implicit nil result.
        if !cx.parent().is_some_and(|p| {
            matches!(
                p,
                Node::IfNode { .. } | Node::UnlessNode { .. } | Node::CaseNode { .. }
            )
        }) {
            return;
        }
        if other.statements().is_some_and(|s| {
            s.body().len() != 1
                || !s
                    .body()
                    .iter()
                    .next()
                    .is_some_and(|n| n.as_nil_node().is_some())
        }) {
            return;
        }
        let (line, col) = file.line_col(other.else_keyword_loc().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Redundant `else`-clause.".into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    if self.missing_else {
                        return None;
                    }
                    let start = other.else_keyword_loc().start_offset();
                    let end = other.end_keyword_loc()?;
                    if super::correction::contains_comment(file, start..end.end_offset()) {
                        return None;
                    }
                    // A heredoc begun in the preceding branch can have its body inside
                    // this lexical interval even though the else AST only contains nil.
                    let root = cx.ancestors().next_back()?;
                    if file.heredoc_pending_at(root, start) {
                        return None;
                    }
                    Some(Correction {
                        range: start..end.start_offset(),
                        replacement: String::new(),
                    })
                })
                .flatten(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! RuboCop empty_else_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/EmptyElse")
            .collect()
    }

    #[test]
    fn detects_empty_and_nil_else_in_ordinary_conditionals() {
        for source in [
            "if a\n  foo\nelse\nend\n",
            "if a\n  foo\nelse\n  nil\nend\n",
            "unless a\n  foo\nelse\n  nil\nend\n",
            "case a\nwhen 1\nelse\n  nil\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
            assert_eq!(found[0].message, "Redundant `else`-clause.");
        }
    }

    #[test]
    fn canonical_removal_keeps_elsif_association_and_indentation() {
        for (before, after) in [
            (
                "if a; foo elsif b; bar else nil end\n",
                "if a; foo elsif b; bar end\n",
            ),
            (
                "x = if a\n  foo\nelsif b\n  bar\nelse\n  nil\nend\n",
                "x = if a\n  foo\nelsif b\n  bar\nend\n",
            ),
            (
                "if a\n  unless b\n    foo\n  else\n    nil\n  end\nend\n",
                "if a\n  unless b\n    foo\n  end\nend\n",
            ),
            (
                "case a\nwhen 1\n  foo\nelse\nend\n",
                "case a\nwhen 1\n  foo\nend\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&super::EmptyElse::default(), before, after);
        }
    }

    #[test]
    fn comments_remain_reported_but_unmodified() {
        for source in [
            "if a; foo; else # TODO\nend\n",
            "if a; foo; else nil # TODO\nend\n",
            "if a; foo; elsif b; bar; else\n# TODO\nend\n",
            "case a; when 1; foo; else\n=begin\nTODO\n=end\nend\n",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&super::EmptyElse::default(), source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn all_applicable_pinned_calls_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/empty_else_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 20);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let cop = super::EmptyElse {
                missing_else: case["missing_else"].as_bool().unwrap(),
            };
            let found = crate::cops::testing::correction_offenses(&cop, source);
            let expected = case["positions"].as_array().unwrap();
            assert_eq!(found.len(), expected.len(), "{case}");
            for (offense, position) in found.iter().zip(expected) {
                assert_eq!(
                    (offense.line, offense.col),
                    (
                        position[0].as_u64().unwrap() as u32,
                        position[1].as_u64().unwrap() as u32
                    ),
                    "{case}"
                );
                assert_eq!(offense.message, "Redundant `else`-clause.");
            }
            if let Some(after) = case["correction"].as_str() {
                crate::cops::testing::assert_correction(&cop, source, after);
            } else {
                assert!(found.iter().all(|o| o.correction.is_none()), "{case}");
            }
        }
    }

    #[test]
    fn pending_heredoc_is_not_deleted_with_the_else_clause() {
        let source = "if a; consume <<~TEXT; else\n  payload\nTEXT\n  nil\nend\n";
        let found = crate::cops::testing::correction_offenses(&super::EmptyElse::default(), source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn many_large_empty_branches_use_constant_size_edits_and_main_parse_indexes() {
        let source = format!(
            "{}{}",
            "# unrelated\n".repeat(2000),
            format!("if a\nelse\n{}end\n", "\n".repeat(2000)).repeat(100)
        );
        // Plain linting never requests the correction-only heredoc/source proofs.
        let plain = offenses(&source);
        assert_eq!(plain.len(), 100);
        assert!(plain.iter().all(|o| o.correction.is_none()));
        let found =
            crate::cops::testing::correction_offenses(&super::EmptyElse::default(), &source);
        assert_eq!(found.len(), 100);
        assert!(found.iter().all(|o| {
            o.correction
                .as_ref()
                .is_some_and(|c| c.replacement.is_empty())
        }));
        // Detection only inspects the direct ElseNode body; removal makes no
        // copies or scans of the large blank intervals, just offset edits.
    }

    #[test]
    fn ignores_side_effects_modifiers_ternaries_rescue_and_pattern_matching() {
        for source in [
            "if a; foo; else bar; nil; end",
            "unless a; foo; else x = nil; end",
            "case a; when 1; foo; else return nil; end",
            "if a; foo; end",
            "foo if a",
            "foo unless a",
            "a ? foo : nil",
            "begin; foo; rescue; bar; else nil; ensure; baz; end",
            "def f; foo; rescue; bar; else nil; end",
            "case a; in 1; foo; else nil; end",
            "if a; foo; else (nil); end",
            "if a; foo; else begin; nil; end; end",
            "if a; foo; else nil; nil; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
