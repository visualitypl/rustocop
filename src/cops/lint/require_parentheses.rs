//! `Lint/RequireParentheses` — ambiguous bare-call boolean binding.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RequireParentheses;
impl Cop for RequireParentheses {
    fn name(&self) -> &'static str {
        "Lint/RequireParentheses"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.closing_loc().is_some_and(|loc| loc.as_slice() == b")") {
            return;
        }
        let args = crate::cops::ast::call_arguments(&call);
        let Some(first) = args.first() else {
            return;
        };
        let end = if let Some(ternary) = first.as_if_node().filter(|n| n.if_keyword_loc().is_none())
        {
            let name = call.name().as_slice();
            if name == b"[]"
                || (name.ends_with(b"=") && !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">="))
                || !boolean(&ternary.predicate())
            {
                return;
            }
            ternary.predicate().location().end_offset()
        } else {
            if !call.name().as_slice().ends_with(b"?") || !boolean(args.last().unwrap()) {
                return;
            }
            crate::cops::ast::source_end(node)
        };
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            message: "Use parentheses in the method call to avoid confusion about precedence."
                .into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
        });
    }
}
fn boolean(node: &Node<'_>) -> bool {
    matches!(node, Node::AndNode { .. } | Node::OrNode { .. })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/require_parentheses_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/RequireParentheses")
            .collect()
    }
    #[test]
    fn pinned_bare_predicates_and_ternaries() {
        for (s, col) in [
            ("if day.is? 'monday' && month == :jan\n foo\nend\n", 4),
            ("day_is? 'tuesday' || true", 1),
            ("wd.include? 'tuesday' && true == true ? a : b", 1),
            ("foo a && b ? c : d", 1),
            ("if day&.is? 'monday' && month == :jan\n foo\nend\n", 4),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(
                found[0].message,
                "Use parentheses in the method call to avoid confusion about precedence."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_no_offense_examples() {
        for s in [
            "if day_is? 'tuesday' + rest\nend",
            "if day.is? 'tuesday' and month == :jan\nend",
            "if day.is? 'tuesday' or month == :jan\nend",
            "if current_level == max + 1\nend",
            "if @debug\nend",
            "if day.is?('tuesday' && true == true)\nend",
            "wd.include?('tuesday' && true == true) ? a : b",
            "weekdays.foo 'tuesday' && true == true",
            "foo && bar ? baz : qux",
            "do_something[foo && bar ? baz : qux]",
            "self.foo = bar && baz ? qux : quux",
            "s.version = @version || '>= 1.8.5'",
            "a[b || c]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
