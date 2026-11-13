//! `Lint/UnreachablePatternBranch` — branches after an unguarded catch-all.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct UnreachablePatternBranch;
impl Cop for UnreachablePatternBranch {
    fn name(&self) -> &'static str {
        "Lint/UnreachablePatternBranch"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_case_match_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(case) = node.as_case_match_node() else {
            return;
        };
        let mut catch_all = false;
        for branch in case.conditions().iter() {
            let branch = branch.as_in_node().unwrap();
            if catch_all {
                self.report(
                    branch.location(),
                    "Unreachable `in` pattern branch detected.",
                    file,
                    out,
                );
            } else {
                // Guards are modifier If/Unless wrappers in Prism, never captures.
                catch_all = catch_all_pattern(&branch.pattern());
            }
        }
        if catch_all && let Some(other) = case.else_clause() {
            self.report(
                other.else_keyword_loc(),
                "Unreachable `else` branch detected.",
                file,
                out,
            );
        }
    }
}

impl UnreachablePatternBranch {
    fn report(
        &self,
        loc: ruby_prism::Location<'_>,
        message: &str,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(loc.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: message.into(),
            correction: None,
        });
    }
}
fn catch_all_pattern(node: &Node<'_>) -> bool {
    if node.as_local_variable_target_node().is_some() {
        true
    } else if let Some(n) = node.as_capture_pattern_node() {
        catch_all_pattern(&n.value())
    } else if let Some(n) = node.as_alternation_pattern_node() {
        catch_all_pattern(&n.left()) || catch_all_pattern(&n.right())
    } else if let Some(n) = node.as_parentheses_node() {
        n.body().is_some_and(|body| {
            if let Some(s) = body.as_statements_node() {
                s.body()
                    .first()
                    .is_some_and(|first| catch_all_pattern(&first))
            } else {
                catch_all_pattern(&body)
            }
        })
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/unreachable_pattern_branch_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        assert!(
            ruby_prism::parse(s.as_bytes()).errors().next().is_none(),
            "{s}"
        );
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnreachablePatternBranch")
            .collect()
    }
    #[test]
    fn catch_all_variables_aliases_and_alternations() {
        for pattern in [
            "x",
            "_",
            "_ => y",
            "x => y",
            "_ | Integer",
            "Integer | _",
            "(_ | Integer) => y",
            "(((_)))",
        ] {
            let s = format!(
                "case value\nin {pattern}\n handle_other\nin Integer\n handle_integer\nin String\n handle_string\nelse\nend\n"
            );
            let found = offenses(&s);
            assert_eq!(found.len(), 3, "{s}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 1, 5));
            assert_eq!(
                found[0].message,
                "Unreachable `in` pattern branch detected."
            );
            assert_eq!((found[1].line, found[1].col, found[1].end_line), (6, 1, 7));
            assert_eq!((found[2].line, found[2].col, found[2].end_line), (8, 1, 8));
            assert_eq!(found[2].message, "Unreachable `else` branch detected.");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }
    #[test]
    fn guards_and_specific_patterns_are_not_catch_alls() {
        for pattern in [
            "x if x.positive?",
            "x unless x.nil?",
            "_ if true",
            "_ unless false",
            "Integer => y",
            "Integer | String",
            "[*]",
            "**rest",
            "1",
            "[*, 1, *]",
            "[x]",
            "{x:}",
            "nil",
        ] {
            let s = format!(
                "case value\nin {pattern}\n handle\nin Symbol\n other\nelse\n fallback\nend"
            );
            assert!(offenses(&s).is_empty(), "{s}");
        }
        assert!(offenses("case value; in Integer; foo; in x; bar; end").is_empty());
        assert!(offenses("case value; in x; handle_any; end").is_empty());
    }
    #[test]
    fn guard_does_not_hide_later_unguarded_capture_and_else_is_independent() {
        let found = offenses(
            "case value\nin x if x.positive?\n positive\nin y\n other\nin Integer\n integer\nelse\n fallback\nend",
        );
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(6, 1), (8, 1)]
        );
        assert_eq!(offenses("case value; in x; else; end").len(), 1);
    }
}
