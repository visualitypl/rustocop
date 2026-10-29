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
        let Some((body, opening, closing, lambda)) = candidate(node) else {
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
                .then(|| correction(node, &body, (opening, closing), lambda, flow, cx, file))
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

fn correction(
    node: &Node<'_>,
    body: &Node<'_>,
    delimiters: (Location<'_>, Location<'_>),
    lambda: bool,
    flow: Flow,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let (opening, closing) = delimiters;
    let root = cx.ancestors().next_back()?;
    // break in a proc can raise LocalJumpError or escape an invoking method.
    // Method-form lambda return/break also require real core lambda lookup;
    // literal arrows are intrinsic, including in deferred/nested scopes.
    if (!lambda && flow == Flow::Break)
        || (node.as_lambda_node().is_none()
            && matches!(flow, Flow::Return | Flow::Break)
            && !file
                .lambda_facts(root)
                .proven
                .contains(&node.location().start_offset()))
        || file.class_method_bindings(root).observes_sources
    {
        return None;
    }
    let mut range = body.location().start_offset()..body.location().end_offset();
    if super::correction::contains_comment(file, range.clone()) {
        return None;
    }
    // Never consume line separators or anything in headers, params, comments,
    // heredoc values or other statements. For multiline bodies erase code but
    // retain CR/LF, so __LINE__ and later source locations stay fixed.
    while range.start > opening.end_offset() && matches!(file.bytes[range.start - 1], b' ' | b'\t')
    {
        range.start -= 1;
    }
    while range.end < closing.start_offset() && matches!(file.bytes[range.end], b' ' | b'\t') {
        range.end += 1;
    }
    let bytes = &file.bytes[range.clone()];
    let replacement = if bytes.contains(&b'\n') || bytes.contains(&b'\r') {
        bytes
            .iter()
            .filter(|b| matches!(b, b'\n' | b'\r'))
            .map(|b| char::from(*b))
            .collect()
    } else if (range.start == opening.end_offset() && opening.as_slice() == b"do")
        || file.bytes.get(range.end) == Some(&b'#')
    {
        " ".into()
    } else {
        String::new()
    };
    let edit = Correction { range, replacement };
    super::correction::preserves_data_positions(file, cx, &edit).then_some(edit)
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/nil_lambda_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
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
    #[test]
    fn receiver_and_argument_lambda_sends_keep_detection_but_not_unproven_jumps() {
        // RuboCop AST BlockNode#lambda? is name-only (unlike Node#lambda?).
        for head in ["obj.lambda", "Kernel.lambda", "obj&.lambda(1)", "lambda(1)"] {
            for body in ["return nil", "break nil"] {
                let source = format!("{head} {{ {body} }}");
                let f = correction_offenses(&NilLambda, &source);
                assert_eq!(f.len(), 1, "{source}");
                assert_eq!((f[0].line, f[0].col), (1, 1));
                assert_eq!(
                    f[0].message,
                    "Use an empty lambda instead of always returning nil."
                );
                assert!(f[0].correction.is_none(), "{source}");
            }
            for body in ["nil", "next nil"] {
                assert_correction(
                    &NilLambda,
                    &format!("{head} {{ {body} }}"),
                    &format!("{head} {{}}"),
                );
            }
        }
    }
    #[test]
    fn canonical_horizontal_corrections_and_multiline_line_preserving_variants() {
        for (head, body) in [
            ("->", "nil"),
            ("->", "return nil"),
            ("->", "break nil"),
            ("->", "next nil"),
            ("lambda", "nil"),
            ("lambda", "return nil"),
            ("lambda", "break nil"),
            ("lambda", "next nil"),
            ("proc", "nil"),
            ("proc", "next nil"),
            ("Proc.new", "nil"),
            ("Proc.new", "next nil"),
        ] {
            assert_correction(
                &NilLambda,
                &format!("{head} {{ {body} }}\n"),
                &format!("{head} {{}}\n"),
            );
            assert_correction(
                &NilLambda,
                &format!("{head} do\n  {body}\nend\n"),
                &format!("{head} do\n\nend\n"),
            );
        }
        for head in ["lambda", "proc", "Proc.new"] {
            assert_correction(
                &NilLambda,
                &format!("fn = {head} do |x|\n       nil\n     end\n"),
                &format!("fn = {head} do |x|\n\n     end\n"),
            );
        }
    }
    #[test]
    fn all_21_pinned_correction_pairs_are_accounted_for() {
        // Exact source shapes in the pinned spec: 7 canonical horizontal
        // pairs, 12 deliberate line-preserving variants, 2 unsafe proc breaks.
        let mut counts = [0; 3];
        for head in ["lambda", "proc", "Proc.new"] {
            for body in ["nil", "return nil", "break nil", "next nil"] {
                if head != "lambda" && body == "return nil" {
                    continue;
                }
                let source = format!("{head} do\n  {body}\nend\n");
                if head != "lambda" && body == "break nil" {
                    assert_correction(&NilLambda, &source, &source);
                    counts[2] += 1;
                } else {
                    let upstream = format!("{head} do\nend\n");
                    let safe = format!("{head} do\n\nend\n");
                    assert_eq!(safe.replace("\n\n", "\n"), upstream);
                    assert_correction(&NilLambda, &source, &safe);
                    counts[1] += 1;
                }
            }
            let source = format!("fn = {head} do |x|\n       nil\n     end\n");
            assert_correction(
                &NilLambda,
                &source,
                &format!("fn = {head} do |x|\n\n     end\n"),
            );
            counts[1] += 1;
            assert_correction(
                &NilLambda,
                &format!("{head} {{ nil }}\n"),
                &format!("{head} {{}}\n"),
            );
            counts[0] += 1;
        }
        for body in ["nil", "return nil", "break nil", "next nil"] {
            assert_correction(&NilLambda, &format!("-> {{ {body} }}\n"), "-> {}\n");
            counts[0] += 1;
        }
        assert_correction(&NilLambda, "-> do\n  nil\nend\n", "-> do\n\nend\n");
        counts[1] += 1;
        assert_eq!(counts, [7, 12, 2]);
    }
    #[test]
    fn parameters_nested_scopes_comments_and_heredoc_bytes_stay_in_place() {
        for (before, after) in [
            (
                "->(x, y=1, *rest, k:, **kw, &blk) { nil }\n",
                "->(x, y=1, *rest, k:, **kw, &blk) {}\n",
            ),
            (
                "lambda { |x; shadow| next nil }\n",
                "lambda { |x; shadow|}\n",
            ),
            ("-> { -> { nil } }\n", "-> { -> {} }\n"),
            ("->(x = -> { nil }) { nil }\n", "->(x = -> {}) {}\n"),
            (
                "-> { nil # keep nil comment\n}\n",
                "-> { # keep nil comment\n}\n",
            ),
            (
                "-> do # heading\n  # body\n  nil # trailing\nend\n",
                "-> do # heading\n  # body\n # trailing\nend\n",
            ),
            ("-> do nil end\n", "-> do end\n"),
            ("-> {;nil;}\n", "-> {;;}\n"),
            ("-> { return \\\n nil }\n", "-> {\n}\n"),
            (
                "f = ->(x = <<~TEXT) do\n nil { # payload\nTEXT\n nil\nend\n",
                "f = ->(x = <<~TEXT) do\n nil { # payload\nTEXT\n\nend\n",
            ),
        ] {
            assert_correction(&NilLambda, before, after);
        }
        for s in [
            "-> { return( # keep\n nil) }",
            "-> { return \\\n # keep\n nil }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn unproven_method_control_flow_and_source_observers_withhold_not_hide() {
        for s in [
            "def lambda(&b); proc(&b); end\nlambda { return nil }",
            "unknown; lambda { break nil }",
            "lambda() { return nil }",
            "def f; lambda { return nil }; end",
            "module M; lambda { break nil }; end",
            "f=-> { nil }; p f.source_location",
            "-> { nil }; TracePoint.new {}",
            "-> { nil }; p caller_locations",
            "-> { nil }; p __FILE__",
            "-> { nil }; eval('1')",
            "-> { nil }; p DATA.pos\n__END__\nraw\n",
        ] {
            let f = correction_offenses(&NilLambda, s);
            assert_eq!(f.len(), 1, "{s}");
            assert!(f[0].correction.is_none(), "{s}");
        }
        // Removing nil/next does not change callee lookup or jump semantics,
        // even when a custom method accepts the literal block.
        assert_correction(
            &NilLambda,
            "def lambda; yield; end\nlambda { next nil }\n",
            "def lambda; yield; end\nlambda {}\n",
        );
        assert_correction(
            &NilLambda,
            "unknown; -> { return nil }\n",
            "unknown; -> {}\n",
        );
        assert_correction(
            &NilLambda,
            "proc { break nil }; -> { nil }\n",
            "proc { break nil }; -> {}\n",
        );
    }
    #[test]
    fn proc_break_reports_but_never_removes_its_nonlocal_jump() {
        for s in ["proc { break nil }", "Proc.new do\n break nil\nend"] {
            let f = correction_offenses(&NilLambda, s);
            assert_eq!(f.len(), 1);
            assert!(f[0].correction.is_none());
        }
    }
}
