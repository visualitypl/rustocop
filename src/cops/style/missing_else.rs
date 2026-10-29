//! `Style/MissingElse` — opt-in exhaustive ordinary conditionals (fixed `both`).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MissingElse {
    guard_clause: bool,
    unless_else: bool,
}
impl Default for MissingElse {
    fn default() -> Self {
        Self {
            guard_clause: true,
            unless_else: true,
        }
    }
}
impl Cop for MissingElse {
    fn name(&self) -> &'static str {
        "Style/MissingElse"
    }
    // Fixed EmptyElse=both leaves no compatible insertion: retain the finding.
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_missing_else_companions(
        &self,
        guard_clause: bool,
        unless_else: bool,
    ) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            guard_clause,
            unless_else,
        }))
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let kind = if let Some(n) = node.as_if_node() {
            // Ternaries have no keyword; modifier keywords follow the body.
            let Some(keyword) = n.if_keyword_loc() else {
                return;
            };
            if keyword.start_offset() != node.location().start_offset() || n.subsequent().is_some()
            {
                return;
            }
            if self.guard_clause
                && n.statements()
                    .as_ref()
                    .and_then(|s| super::guard_clause::guard_source(s, file))
                    .is_some()
            {
                return;
            }
            "if"
        } else if let Some(n) = node.as_unless_node() {
            if self.unless_else || n.end_keyword_loc().is_none() || n.else_clause().is_some() {
                return;
            }
            if self.guard_clause
                && n.statements()
                    .as_ref()
                    .and_then(|s| super::guard_clause::guard_source(s, file))
                    .is_some()
            {
                return;
            }
            "if"
        } else if let Some(n) = node.as_case_node() {
            if n.else_clause().is_some() {
                return;
            }
            "case"
        } else {
            return;
        };
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: format!("`{kind}` condition requires an `else`-clause."),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! RuboCop missing_else_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn ordinary_if_case_and_last_elsif_require_else() {
        for (source, col, kind) in [
            ("if cond; foo end", 1, "if"),
            ("case v; when a; foo; when b; bar; end", 1, "case"),
            ("if cond_1; 1; elsif cond_2; 3; end", 15, "if"),
            (
                "if cond_1; 1; elsif cond_2; 2; elsif cond_3; 3; end",
                32,
                "if",
            ),
        ] {
            let found = correction_offenses(&MissingElse::default(), source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("`{kind}` condition requires an `else`-clause.")
            );
            // EmptyElse's fixed `both` style has no compatible insertion.
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn all_applicable_pinned_diagnostics_have_no_compatible_fixed_style_fix() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/missing_else_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 13);
        for case in cases {
            let found =
                correction_offenses(&MissingElse::default(), case["source"].as_str().unwrap());
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
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn configured_companions_and_guard_shapes_not_selector_membership() {
        let disabled = MissingElse {
            guard_clause: false,
            unless_else: false,
        };
        for source in ["if a; raise 'boom'; end", "unless a; work; end"] {
            assert_eq!(correction_offenses(&disabled, source).len(), 1, "{source}");
            assert!(
                correction_offenses(&MissingElse::default(), source).is_empty(),
                "{source}"
            );
        }
        for source in [
            "def f; if a; return foo; end; end",
            "loop { if a; break foo; end }",
            "loop { if a; next foo; end }",
            "if a; ok and raise 'boom'; end",
            "if a; ok || fail('boom'); end",
        ] {
            assert!(
                correction_offenses(&MissingElse::default(), source).is_empty(),
                "{source}"
            );
        }
        for source in [
            "if a; work; raise 'boom'; end",
            "if a; object.raise 'boom'; end",
            "if a; raise(\n 'boom'\n); end",
            "if a; raise { work }; end",
            "if a; (raise 'boom'); end",
            "if a; ok && (raise 'boom'); end",
        ] {
            assert_eq!(
                correction_offenses(&MissingElse::default(), source).len(),
                1,
                "{source}"
            );
        }
    }
    #[test]
    fn nested_guards_else_association_rescue_and_branch_effects() {
        for (source, positions) in [
            ("if a\n if b; work; end\nelse\n other\nend\n", vec![(2, 2)]),
            (
                "if a; work; elsif b; other; elsif c; last; end",
                vec![(1, 29)],
            ),
            (
                "if a\n begin\n work\n rescue\n recover\n ensure\n cleanup\n end\nend\n",
                vec![(1, 1)],
            ),
            (
                "case value\nwhen a\n if b; effect; end\nwhen c\n other\nend\n",
                vec![(1, 1), (3, 2)],
            ),
        ] {
            let found = correction_offenses(&MissingElse::default(), source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
        }
    }
    #[test]
    fn accepts_else_pattern_matching_modifiers_unless_and_guards() {
        for source in [
            "if a; foo else end",
            "if a; foo elsif b; bar else nil end",
            "if cond; foo else bar; nil end",
            "unless cond; foo end",
            "case v; when a; foo else end",
            "case pattern; in a; foo; end",
            "foo if cond",
            "foo unless cond",
            "cond ? foo : bar",
            "if a; raise 'boom'; end",
            "def f; if a; return foo; end; end",
        ] {
            assert!(
                correction_offenses(&MissingElse::default(), source).is_empty(),
                "{source}"
            );
        }
    }
}
