//! Parenthesize direct command-call children of parenthesized invocations.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct NestedParenthesizedCalls;
const ALLOWED: &[&[u8]] = &[
    b"be",
    b"be_a",
    b"be_an",
    b"be_between",
    b"be_falsey",
    b"be_kind_of",
    b"be_instance_of",
    b"be_truthy",
    b"be_within",
    b"eq",
    b"eql",
    b"end_with",
    b"include",
    b"match",
    b"raise_error",
    b"respond_to",
    b"start_with",
];
impl Cop for NestedParenthesizedCalls {
    fn name(&self) -> &'static str {
        "Style/NestedParenthesizedCalls"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let args = ast::call_arguments(&call);
        if args.is_empty()
            // A literal block wraps its send in RuboCop's AST, so this is
            // not a direct call child of the outer invocation.
            || call.block().is_some_and(|n| n.as_block_node().is_some())
            || call.opening_loc().is_some()
            || call.equal_loc().is_some()
            || ast::is_operator_method(call.name().as_slice())
        {
            return;
        }
        let Some(parent) = cx
            .ancestors()
            .find(|n| n.as_arguments_node().is_none())
            .and_then(|n| n.as_call_node())
        else {
            return;
        };
        if !parent
            .opening_loc()
            .is_some_and(|loc| loc.as_slice() == b"(")
        {
            return;
        }
        if args.len() == 1
            && ast::call_arguments(&parent).len() == 1
            && ALLOWED.contains(&call.name().as_slice())
        {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Add parentheses to nested method call `{}`.",
                single_line(&file.bytes[start..end])
            ),
            line,
            col,
            end_line: file.line_of(end - 1),
            correction: cx
                .corrections_requested()
                .then(|| super::method_call_with_args_parentheses::correction(node, end, file))
                .flatten()
                .filter(|edit| super::correction::preserves_data_positions(file, cx, edit)),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's corresponding Style specs (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/NestedParenthesizedCalls")
            .collect()
    }
    #[test]
    fn data_byte_guard_keeps_nested_call_findings_and_allowlist() {
        use crate::cops::testing::correction_offenses;
        let source = "outer(inner 1)\np ::DATA.method(:tell).call\n__END__\nraw\n";
        let found = correction_offenses(&NestedParenthesizedCalls, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 7, 1));
        assert_eq!(
            found[0].message,
            "Add parentheses to nested method call `inner 1`."
        );
        assert!(found[0].correction.is_none());
        // `DATA` with a data section is a listed observer:
        // even the equal-length edit is held.
        crate::cops::testing::assert_listed_hold(
            &NestedParenthesizedCalls,
            "outer(inner  1)\np DATA.pos\n__END__\nraw\n",
        );
        assert!(offenses("outer(eq 1)\np DATA.pos\n__END__\nraw\n").is_empty());
    }

    #[test]
    fn pinned_offense_and_canonical_correction() {
        let found = offenses("puts(compute something)\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 6, 1));
        assert_eq!(
            found[0].message,
            "Add parentheses to nested method call `compute something`."
        );
        assert_correction(
            &NestedParenthesizedCalls,
            "puts(compute something)\n",
            "puts(compute(something))\n",
        );
    }
    #[test]
    fn pinned_single_multiple_and_navigation_argument_corrections() {
        for (before, after, source) in [
            (
                "puts(receiver&.compute something)",
                "puts(receiver&.compute(something))",
                "receiver&.compute something",
            ),
            (
                "puts(compute first, second)",
                "puts(compute(first, second))",
                "compute first, second",
            ),
            (
                "puts(nex \\\n       5)",
                "puts(nex(\\\n       5))",
                "nex \\\n       5",
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (1, 6));
            assert_eq!(
                found[0].message,
                format!(
                    "Add parentheses to nested method call `{}`.",
                    single_line(source.as_bytes())
                )
            );
            assert_correction(&NestedParenthesizedCalls, before, after);
        }
    }

    #[test]
    fn fixed_allowlist_requires_one_argument_on_both_calls() {
        for name in ALLOWED {
            let name = std::str::from_utf8(name).unwrap();
            assert!(offenses(&format!("expect(obj).to({name} true)")).is_empty());
            assert_correction(
                &NestedParenthesizedCalls,
                &format!("outer({name} a, b)"),
                &format!("outer({name}(a, b))"),
            );
            assert!(offenses(&format!("outer({name}(a), b)")).is_empty());
        }
    }

    #[test]
    fn block_forwarding_heredocs_and_nested_commands_preserve_association() {
        for (before, after) in [
            ("outer(inner a { body })", "outer(inner(a { body }))"),
            (
                "def f(&block); outer(inner &block); end",
                "def f(&block); outer(inner(&block)); end",
            ),
            (
                "outer(inner <<~DOC)\ntext\nDOC\n",
                "outer(inner(<<~DOC))\ntext\nDOC\n",
            ),
            (
                "outer(inner a, <<~DOC,\ntext\nDOC\n  __LINE__)",
                "outer(inner(a, <<~DOC,\ntext\nDOC\n  __LINE__))",
            ),
            ("outer(inner command a, b)", "outer(inner(command a, b))"),
        ] {
            assert_correction(&NestedParenthesizedCalls, before, after);
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "puts 1, 2",
            "puts(compute(something))",
            "puts(compute)",
            "method(obj[1])",
            "method(block_taker { other 1 })",
            "expect(obj).to(be true)",
            "expect(obj.attr = 1).to eq 1",
            "outer(inner a do; body; end)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
