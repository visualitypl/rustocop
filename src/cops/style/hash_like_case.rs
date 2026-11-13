//! `Style/HashLikeCase`: fixed MinBranchesCount=3, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct HashLikeCase;
impl Cop for HashLikeCase {
    fn name(&self) -> &'static str {
        "Style/HashLikeCase"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_case_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(case) = node.as_case_node() else {
            return;
        };
        if case.conditions().len() < 3
            || case
                .else_clause()
                .is_some_and(|e| e.statements().is_some_and(|s| !s.body().is_empty()))
        {
            return;
        }
        let mut condition_type = None;
        let mut body_type = None;
        for branch in case.conditions().iter() {
            let Some(branch) = branch.as_when_node() else {
                return;
            };
            let conditions = branch.conditions();
            if conditions.len() != 1 {
                return;
            }
            let condition = conditions.iter().next().unwrap();
            let kind = if condition.as_string_node().is_some()
                || condition.as_source_file_node().is_some()
            {
                "str"
            } else if condition.as_symbol_node().is_some() {
                "sym"
            } else {
                return;
            };
            if condition_type.replace(kind).is_some_and(|old| old != kind) {
                return;
            }
            let Some(body) = branch.statements() else {
                return;
            };
            if body.body().len() != 1 {
                return;
            }
            let body = body.body().iter().next().unwrap();
            if !ast::is_recursive_basic_literal(&body) {
                return;
            }
            let Some(kind) = literal_type(&body) else {
                return;
            };
            if body_type.replace(kind).is_some_and(|old| old != kind) {
                return;
            }
        }
        let start = case.location().start_offset();
        let end = case.location().end_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Consider replacing `case-when` with a hash lookup.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
// Prism separates pseudo literals and static/dynamic regexps; RuboCop does not.
fn literal_type(node: &Node<'_>) -> Option<&'static str> {
    Some(match node {
        Node::IntegerNode { .. } | Node::SourceLineNode { .. } => "int",
        Node::FloatNode { .. } => "float",
        Node::RationalNode { .. } => "rational",
        Node::ImaginaryNode { .. } => "complex",
        Node::StringNode { .. } | Node::SourceFileNode { .. } => "str",
        Node::SymbolNode { .. } => "sym",
        Node::InterpolatedStringNode { .. } => "dstr",
        Node::InterpolatedSymbolNode { .. } => "dsym",
        Node::RegularExpressionNode { .. } | Node::InterpolatedRegularExpressionNode { .. } => {
            "regexp"
        }
        Node::XStringNode { .. } | Node::InterpolatedXStringNode { .. } => "xstr",
        Node::TrueNode { .. } => "true",
        Node::FalseNode { .. } => "false",
        Node::NilNode { .. } => "nil",
        Node::ArrayNode { .. } => "array",
        Node::HashNode { .. } => "hash",
        Node::ParenthesesNode { .. } => "begin",
        Node::CallNode { .. } => "send",
        Node::AndNode { .. } => "and",
        Node::OrNode { .. } => "or",
        Node::RangeNode { .. } => {
            if node.as_range_node().unwrap().is_exclude_end() {
                "erange"
            } else {
                "irange"
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    //! Adapted from RuboCop hash_like_case_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov); fixed threshold 3.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/HashLikeCase")
            .collect()
    }
    fn case(keys: [&str; 3], bodies: [&str; 3], tail: &str) -> String {
        format!(
            "case x\nwhen {}\n  {}\nwhen {}\n  {}\nwhen {}\n  {}\n{tail}end\n",
            keys[0], bodies[0], keys[1], bodies[1], keys[2], bodies[2]
        )
    }
    #[test]
    fn same_type_literal_branches() {
        for keys in [["'foo'", "'bar'", "'baz'"], [":foo", ":bar", ":baz"]] {
            for bodies in [
                ["'FOO'", "'BAR'", "'BAZ'"],
                ["1", "2", "-3"],
                ["[1]", "[2]", "[]"],
                ["{a: 1}", "{}", "{b: [2]}"],
                ["/a/", "/b/", "/c/"],
                ["(1)", "(2)", "(3)"],
            ] {
                let s = case(keys, bodies, "");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 1, 8));
                assert_eq!(
                    o[0].message,
                    "Consider replacing `case-when` with a hash lookup."
                );
                assert!(o[0].correction.is_none());
            }
        }
    }
    #[test]
    fn mismatched_dynamic_empty_multi_statement_and_else_branches() {
        for s in [
            case(["'foo'", ":bar", "'baz'"], ["1", "2", "3"], ""),
            case(["y", "z", "w"], ["1", "2", "3"], ""),
            case([":a", ":b", ":c"], ["1", "'2'", "3"], ""),
            case([":a", ":b", ":c"], ["y", "z", "w"], ""),
            case([":a", ":b", ":c"], ["1", "2", "3"], "else\n  4\n"),
            case([":a", ":b", ":c"], ["1", "2", "3"], "else\n  nil\n"),
            case([":a, :d", ":b", ":c"], ["1", "2", "3"], ""),
            case([":a", ":b", ":c"], ["1; 2", "2", "3"], ""),
            case([":a", ":b", ":c"], ["", "2", "3"], ""),
            case([":a", ":b", ":c"], ["[effect()]", "[]", "[]"], ""),
            "case x; when :a; 1; when :b; 2; end".into(),
            "case x; in :a; 1; in :b; 2; in :c; 3; end".into(),
        ] {
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
    #[test]
    fn safe_navigation_is_not_a_recursive_literal_at_any_depth() {
        for template in [
            "CALL",
            "[CALL]",
            "[{a: [CALL]}]",
            "{CALL => 1}",
            "{a: CALL}",
            "(CALL)",
            "\"#{CALL}\"",
            "(CALL..3)",
            "(true && CALL)",
            "1.==(CALL)",
            "CALL.==(false)",
        ] {
            let ordinary = template.replace("CALL", "1.==(2)");
            let safe = template.replace("CALL", "1&.==(2)");
            let source = case([":a", ":b", ":c"], [&ordinary; 3], "");
            let o = offenses(&source);
            assert_eq!(o.len(), 1, "{source}");
            assert_eq!((o[0].line, o[0].col), (1, 1));
            assert_eq!(
                o[0].message,
                "Consider replacing `case-when` with a hash lookup."
            );
            let source = case([":a", ":b", ":c"], [&safe, &ordinary, &ordinary], "");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        assert!(
            offenses("case x; when :a; [1&.==(2)]; when :b; [2]; when :c; [3]; end").is_empty()
        );
    }
    #[test]
    fn duplicate_keys_predicateless_and_empty_else_follow_pinned_predicate() {
        for s in [
            "case; when :a; 1; when :a; 2; when :b; 3; end",
            "case x; when :a; 1; when :b; 2; when :c; 3; else; end",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
}
