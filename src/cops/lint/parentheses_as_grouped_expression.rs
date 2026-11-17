//! `Lint/ParenthesesAsGroupedExpression` — grouped single call arguments.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct ParenthesesAsGroupedExpression;
impl Cop for ParenthesesAsGroupedExpression {
    fn name(&self) -> &'static str {
        "Lint/ParenthesesAsGroupedExpression"
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
        if call.opening_loc().is_some()
            || ast::is_operator_method(call.name().as_slice())
            || call.equal_loc().is_some()
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let [arg] = args.as_slice() else { return };
        // Prism keeps the expression's own parentheses as a distinct node.
        // Chains, compound ranges, hashes and ternaries merely starting with
        // a grouped operand do not own this outer group (nor do bare blocks).
        let Some(group) = arg.as_parentheses_node() else {
            return;
        };
        if invalid_bare_argument(arg) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let start = selector.end_offset();
        let end = group.opening_loc().start_offset();
        if end <= start {
            return;
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        let projected = file.match_pattern_source(arg.location().as_slice());
        let mut argument = String::new();
        for chunk in projected.utf8_chunks() {
            argument.push_str(chunk.valid());
            for byte in chunk.invalid() {
                use std::fmt::Write as _;
                write!(argument, "\\x{byte:02X}").expect("write to String");
            }
        }
        let argument = crate::offense::single_line(argument.as_bytes());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!("`{argument}` interpreted as grouped expression."),
            correction: cx
                .corrections_requested()
                .then(|| correction(start..end))
                .flatten(),
        });
    }
}

// Removing the gap repurposes the group's delimiters as call delimiters.
// A singleton value with clean horizontal margins has identical binding,
// dispatch, evaluation and allocation. Do not repurpose statement groups,
// assignments, control flow, nested commands, comments, continuations or blocks.
// Global source/code-object/heap/DATA/encoding/atomic guards still apply.
/// RuboCop's corrector: remove the space before the parenthesis.
fn correction(gap: std::ops::Range<usize>) -> Option<crate::offense::Correction> {
    Some(crate::offense::Correction {
        range: gap,
        replacement: String::new(),
    })
}

// RuboCop unwraps only singleton `begin` nodes, not keyword begin/end or
// multi-statement groups. These forms cannot appear bare in call delimiters.
fn invalid_bare_argument(node: &Node<'_>) -> bool {
    let mut node = node
        .as_parentheses_node()
        .expect("grouped argument")
        .as_node();
    while let Some(group) = node.as_parentheses_node() {
        let Some(body) = group.body().and_then(|b| b.as_statements_node()) else {
            return false;
        };
        if body.body().len() != 1 {
            return false;
        }
        node = body.body().first().unwrap();
    }
    match node {
        Node::AndNode { .. } => node.as_and_node().unwrap().operator_loc().as_slice() == b"and",
        Node::OrNode { .. } => node.as_or_node().unwrap().operator_loc().as_slice() == b"or",
        Node::RescueModifierNode { .. } => true,
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            // Prism lowers prefix `not` to `!`; ordinary `.not`/`&.not`
            // selectors retain their method name and are valid bare arguments.
            call.name().as_slice() == b"!"
                && call.message_loc().is_some_and(|l| l.as_slice() == b"not")
        }
        Node::IfNode { .. } => node
            .as_if_node()
            .unwrap()
            .if_keyword_loc()
            .is_some_and(|l| l.start_offset() > node.location().start_offset()),
        Node::UnlessNode { .. } => {
            node.as_unless_node().unwrap().keyword_loc().start_offset()
                > node.location().start_offset()
        }
        Node::WhileNode { .. } => {
            node.as_while_node().unwrap().keyword_loc().start_offset()
                > node.location().start_offset()
        }
        Node::UntilNode { .. } => {
            node.as_until_node().unwrap().keyword_loc().start_offset()
                > node.location().start_offset()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/parentheses_as_grouped_expression_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ParenthesesAsGroupedExpression")
            .collect()
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "a.concat (1..1).map { |i| i * 10 }",
            "a.concat (1..1).map { _1 * 10 }",
            "a.concat (1..1).map { it * 10 }",
            "func (x) || y",
            "func (x).func.func.func.func.func",
            "func (x).func&.func",
            "puts (2 + 3) * 4",
            "do_something.eq (foo * bar).to_i",
            "transition (foo - bar) => value",
            "foo (cond) ? 1 : 2",
            "func",
            "puts x",
            "a.b\na.b 1\na.b(1)",
            "a b(c)",
            "def g; a yield(c); end",
            "a super(c)",
            "a defined?(c)",
            "a % (b + c)",
            "a.b = (c == d)",
            "a( (b) )",
            "rand (a - b)..(c - d)",
            "assert_equal (0..1.9), acceleration.domain",
            "foo(\n <<~EOS\nfoo (\n)\nEOS\n)",
            "false? (foo and bar)",
            "false? (foo or bar)",
            "false? (not foo)",
            "false? (foo rescue bar)",
            "false? (foo if bar)",
            "false? ((foo until bar))",
            "false? ((foo while bar))",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn ordinary_not_methods_are_not_keyword_operators() {
        for argument in [
            "obj.not",
            "obj&.not",
            "obj.not(1)",
            "obj&.not(1)",
            "obj::not",
            "(obj.not)",
        ] {
            let source = format!("foo ({argument})");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, 4, 1),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                format!("`({argument})` interpreted as grouped expression.")
            );
        }
        for source in ["foo (not x)", "foo (not(x))", "foo ((not x))"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_grouped_block_arguments() {
        for body in ["|i| i * 10", "_1 * 10", "it * 10"] {
            let source = format!("a.concat ((1..1).map {{ {body} }})");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 9));
            assert_eq!(
                found[0].message,
                format!("`((1..1).map {{ {body} }})` interpreted as grouped expression.")
            );
        }
    }
    #[test]
    fn reports_selector_to_group_gap() {
        for (source, col, argument) in [
            ("a.func (x)", 7, "(x)"),
            ("is? (x)", 4, "(x)"),
            ("a&.func (x)", 8, "(x)"),
            ("rand (1..10)", 5, "(1..10)"),
            ("false? (foo && bar)", 7, "(foo && bar)"),
            ("a.func \t (x)", 7, "(x)"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                format!("`{argument}` interpreted as grouped expression.")
            );
        }
    }
}
