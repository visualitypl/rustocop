//! `Style/YodaCondition` — forbid literal/constant-first comparisons.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct YodaCondition;
impl Cop for YodaCondition {
    fn name(&self) -> &'static str {
        "Style/YodaCondition"
    }
    fn unsafe_fixes(&self) -> bool {
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
        if call.is_safe_navigation()
            || !matches!(
                call.name().as_slice(),
                b"==" | b"!=" | b"<" | b"<=" | b">" | b">="
            )
        {
            return;
        }
        let Some(lhs) = call.receiver() else { return };
        let args = ast::call_arguments(&call);
        let Some(rhs) = args.first() else { return };
        if matches!(call.name().as_slice(), b"==" | b"!=")
            && lhs.location().as_slice() == b"__FILE__"
            && rhs
                .as_global_variable_read_node()
                .is_some_and(|n| matches!(n.name().as_slice(), b"$0" | b"$PROGRAM_NAME"))
        {
            return;
        }
        if matches!(
            lhs,
            Node::InterpolatedStringNode { .. } | Node::InterpolatedRegularExpressionNode { .. }
        ) || !constant_portion(&lhs)
            || constant_portion(rhs)
        {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        // move_receiver_to_back: `rhs op' lhs`, with the comparison reversed.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let operator = match call.name().as_slice() {
                    b"<" => ">",
                    b"<=" => ">=",
                    b">" => "<",
                    b">=" => "<=",
                    other => std::str::from_utf8(other).ok()?,
                };
                let at = rhs.location().end_offset();
                Correction::from_edits(
                    &file.bytes,
                    vec![
                        (start..rhs.location().start_offset(), String::new()),
                        (
                            at..at,
                            format!(
                                " {operator} {}",
                                String::from_utf8_lossy(lhs.location().as_slice())
                            ),
                        ),
                    ],
                )
            })
            .flatten();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!(
                "Reverse the order of the operands `{}`.",
                String::from_utf8_lossy(&file.bytes[start..end])
            ),
            correction,
        });
    }
}
fn constant_portion(node: &Node<'_>) -> bool {
    if ast::is_const(node) {
        return true;
    }
    let mut literal = RecursiveLiteral(true);
    crate::cops::walk::walk(&mut literal, node);
    literal.0
}

// RuboCop's recursive_literal?, not constant folding or purity inference.
// Constants count only as entire operands, not inside literal collections.
struct RecursiveLiteral(bool);
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for RecursiveLiteral {
    fn enter(&mut self, node: Node<'pr>) {
        if !self.0 {
            return;
        }
        self.0 = if let Some(call) = node.as_call_node() {
            ast::negated_numeric_literal(&node).is_some()
                || (!call.is_safe_navigation()
                    && call.receiver().is_some()
                    && call.block().is_none()
                    && matches!(
                        call.name().as_slice(),
                        b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<" | b"*" | b"!" | b"<=>"
                    ))
        } else {
            ast::is_literal(&node)
                || matches!(
                    node,
                    Node::AndNode { .. }
                        | Node::OrNode { .. }
                        | Node::ParenthesesNode { .. }
                        | Node::StatementsNode { .. }
                        | Node::EmbeddedStatementsNode { .. }
                        | Node::AssocNode { .. }
                        | Node::ArgumentsNode { .. }
                )
        };
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/yoda_condition_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/YodaCondition")
            .collect()
    }
    #[test]
    fn flags_literal_and_constant_left_operands() {
        for expr in [
            "\"foo\" == bar",
            "nil == bar",
            "false == active?",
            "15 != @foo",
            "42 < bar",
            "FOO < bar",
            "::Foo::BAR >= baz",
            "[-1, 2] == foo",
            "{a: 1} == foo",
            "(1..2) == foo",
            "-1.0r <= x",
            "__LINE__ == x",
            "__ENCODING__ == x",
        ] {
            let s = format!("# comparison\n  {expr}\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, 3));
            assert_eq!(
                o[0].message,
                format!("Reverse the order of the operands `{expr}`.")
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn accepts_dynamic_and_symmetric_lookalikes() {
        for s in [
            "b.value == 2",
            "b&.value == 2",
            "@value == 2",
            "@@value == 2",
            "b=1; b == 2",
            "$var == 5",
            "foo == \"bar\"",
            "foo == BAR",
            "\"#{interpolation}\" == foo",
            "/#{interpolation}/ == foo",
            "foo[0] > \"bar\" || baz != \"baz\"",
            "(first_line - second_line) > 0",
            "5 == 6",
            "[1,2,3] <=> [4,5,6]",
            "[x] == [x].do_something",
            "{a: x} == foo",
            "def foo(*); [*] == [*].do_something; end",
            "!true",
            "not true",
            "0 <=> val",
            "\"foo\" === bar",
            "__FILE__ == $0",
            "__FILE__ != $PROGRAM_NAME",
            "foo.==",
            "1&.==(x)",
            "[CONST] == x",
            "begin; 1; end == foo",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn checks_nested_comparisons_independently() {
        let o = offenses("2 == foo(1 == bar)\n");
        assert_eq!(
            o.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 1), (1, 10)]
        );
    }
}
