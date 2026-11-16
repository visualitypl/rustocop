//! `Style/NilLambda` — omit a lambda/proc's sole nil-returning statement.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct NilLambda;
impl Cop for NilLambda {
    fn name(&self) -> &'static str {
        "Style/NilLambda"
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
        let Some((body, _, _, lambda)) = candidate(node) else {
            return;
        };
        let Some(body) = sole_statement(body) else {
            return;
        };
        let flow = if body.as_nil_node().is_some() {
            Flow::Nil
        } else if body
            .as_return_node()
            .is_some_and(|n| nil_argument(n.arguments()))
        {
            Flow::Return
        } else if body
            .as_next_node()
            .is_some_and(|n| nil_argument(n.arguments()))
        {
            Flow::Next
        } else if body
            .as_break_node()
            .is_some_and(|n| nil_argument(n.arguments()))
        {
            Flow::Break
        } else {
            return;
        };
        if !lambda && flow == Flow::Return {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: format!(
                "Use an empty {} instead of always returning nil.",
                if lambda { "lambda" } else { "proc" }
            ),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, &body, file))
                .flatten(),
        });
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Nil,
    Return,
    Next,
    Break,
}
fn sole_statement(body: Option<Node<'_>>) -> Option<Node<'_>> {
    let statements = body?.as_statements_node()?;
    (statements.body().len() == 1)
        .then(|| statements.body().first())
        .flatten()
}
fn nil_argument(arguments: Option<ruby_prism::ArgumentsNode<'_>>) -> bool {
    arguments.is_some_and(|args| {
        args.arguments().len() == 1
            && args
                .arguments()
                .first()
                .is_some_and(|n| n.as_nil_node().is_some())
    })
}
/// RuboCop BlockNode#lambda? accepts any send named lambda, including receivers
/// and arguments. Its proc? matcher is narrower: bare argumentless proc or
/// argumentless global Proc.new. Detection does not prove runtime core lookup.
fn candidate<'pr>(
    node: &Node<'pr>,
) -> Option<(Option<Node<'pr>>, Location<'pr>, Location<'pr>, bool)> {
    if let Some(n) = node.as_lambda_node() {
        return Some((n.body(), n.opening_loc(), n.closing_loc(), true));
    }
    let call = node.as_call_node()?;
    let lambda = call.name().as_slice() == b"lambda";
    let proc = call.arguments().is_none()
        && !call.is_safe_navigation()
        && ((call.name().as_slice() == b"proc" && call.receiver().is_none())
            || (call.name().as_slice() == b"new"
                && call
                    .receiver()
                    .is_some_and(|r| ast::is_global_const(&r, &["Proc"]))));
    if !lambda && !proc {
        return None;
    }
    let block = call.block()?.as_block_node()?;
    Some((
        block.body(),
        block.opening_loc(),
        block.closing_loc(),
        lambda,
    ))
}

/// RuboCop's corrector: the body with the blanks around it on one line,
/// its whole lines otherwise.
fn correction(node: &Node<'_>, body: &Node<'_>, file: &SourceFile) -> Option<Correction> {
    let bytes = &file.bytes;
    let b = body.location();
    let mut range = b.start_offset()..b.end_offset();
    let n = node.location();
    if file.line_of(n.start_offset()) == file.last_line_of(n) {
        while range.start > 0 && matches!(bytes[range.start - 1], b' ' | b'\t') {
            range.start -= 1;
        }
        while range.start > 0 && bytes[range.start - 1] == b'\n' {
            range.start -= 1;
        }
        while matches!(bytes.get(range.end), Some(b' ' | b'\t')) {
            range.end += 1;
        }
        while bytes.get(range.end) == Some(&b'\n') {
            range.end += 1;
        }
    } else {
        range = file.line_start(file.line_of(range.start))
            ..file.line_start(file.line_of(range.end.saturating_sub(1).max(range.start)) + 1);
    }
    Some(Correction {
        range,
        replacement: String::new(),
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/nil_lambda_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn corrects_as_rubocop() {
        assert_correction(&NilLambda, "-> { nil }\n", "-> {}\n");
        assert_correction(&NilLambda, "lambda do\n  nil\nend\n", "lambda do\nend\n");
    }
    fn offenses(s: &str) -> Vec<Offense> {
        assert_eq!(ruby_prism::parse(s.as_bytes()).errors().count(), 0, "{s}");
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == NilLambda.name())
            .collect()
    }
    #[test]
    fn all_pinned_diagnostics_at_the_lambda_or_proc_not_its_body() {
        for (head, kind) in [
            ("lambda", "lambda"),
            ("->", "lambda"),
            ("proc", "proc"),
            ("Proc.new", "proc"),
        ] {
            for body in ["nil", "return nil", "break nil", "next nil"] {
                if body == "return nil" && kind == "proc" {
                    continue;
                }
                for (source, end_line) in [
                    (format!("  {head} {{ {body} }}\n"), 1),
                    (format!("  {head} do\n    {body}\n  end\n"), 3),
                ] {
                    let f = offenses(&source);
                    assert_eq!(f.len(), 1, "{source}");
                    assert_eq!(
                        (f[0].line, f[0].col, f[0].end_line),
                        (1, 3, end_line),
                        "{source}"
                    );
                    assert_eq!(
                        f[0].message,
                        format!("Use an empty {kind} instead of always returning nil.")
                    );
                    assert!(f[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn pinned_negative_bodies_and_nonlocal_proc_returns() {
        for head in ["lambda", "->", "proc", "Proc.new"] {
            for body in [
                "6",
                "",
                "nil if x",
                "x ? x.method : nil",
                "side_effect; nil",
                "return",
                "next",
                "break",
                "(nil)",
                "begin; nil; end",
                "return nil, nil",
                "next [nil]",
                "nil rescue nil",
            ] {
                let s = format!("{head} {{ {body} }}");
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
        for s in [
            "proc { return nil }",
            "Proc.new { return nil }",
            "::Proc.new { return nil }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn exact_callee_grammar_and_receiver_lookalikes() {
        for s in [
            "proc(1) { nil }",
            "Proc.new(1) { nil }",
            "self.proc { nil }",
            "Other::Proc.new { nil }",
            "Proc&.new { nil }",
            "foo { nil }",
            "def lambda; nil; end",
            "'-> { nil }' # proc { nil }",
            "-> { it }",
            "-> { _1 }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "lambda() { nil }",
            "lambda(1) { nil }",
            "obj.lambda { nil }",
            "Kernel.lambda { nil }",
            "obj&.lambda(1) { nil }",
            "proc() { nil }",
            "::Proc.new() { nil }",
            "Proc::new { nil }",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
}
