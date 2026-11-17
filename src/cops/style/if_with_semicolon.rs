//! `Style/IfWithSemicolon` — semicolon-delimited conditional headers.
use crate::{
    cops::{Cop, NodeContext, ast, variables},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};
pub struct IfWithSemicolon;
impl Cop for IfWithSemicolon {
    fn name(&self) -> &'static str {
        "Style/IfWithSemicolon"
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
        let Some(parts) = candidate(node, file) else {
            return;
        };
        if conditional_parent(cx.ancestors())
            || cx.ancestors().enumerate().any(|(i, n)| {
                candidate(n, file).is_some() && !conditional_parent(cx.ancestors().skip(i + 1))
            })
        {
            return;
        }
        let advice = if newline(parts.body.as_ref()) || newline(parts.other_body().as_ref()) {
            "a newline"
        } else if special(parts.body.as_ref())
            || special(parts.other_body().as_ref())
            || nested_else(&parts)
            || parts
                .other
                .as_ref()
                .is_some_and(|n| n.as_if_node().is_some() || n.as_unless_node().is_some())
        {
            "`if/else`"
        } else {
            "a ternary operator"
        };
        let loc = node.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Do not use `{} {};` - use {advice} instead.",
                if parts.unless { "unless" } else { "if" },
                String::from_utf8_lossy(parts.predicate.location().as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(loc),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, &parts, cx))
                .flatten(),
        });
    }
}
struct Parts<'pr> {
    predicate: Node<'pr>,
    body: Option<StatementsNode<'pr>>,
    other: Option<Node<'pr>>,
    semi: usize,
    unless: bool,
}
impl<'pr> Parts<'pr> {
    fn other_body(&self) -> Option<StatementsNode<'pr>> {
        self.other
            .as_ref()
            .and_then(|n| n.as_else_node())
            .and_then(|n| n.statements())
    }
}
fn candidate<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Parts<'pr>> {
    let (predicate, body, other, unless) = if let Some(n) = node.as_if_node() {
        if n.end_keyword_loc().is_none()
            || n.if_keyword_loc()?.as_slice() != b"if"
            || n.then_keyword_loc().is_some()
        {
            return None;
        }
        (n.predicate(), n.statements(), n.subsequent(), false)
    } else {
        let n = node.as_unless_node()?;
        if n.end_keyword_loc().is_none() || n.then_keyword_loc().is_some() {
            return None;
        }
        (
            n.predicate(),
            n.statements(),
            n.else_clause().map(|n| n.as_node()),
            true,
        )
    };
    let mut semi = predicate.location().end_offset();
    while matches!(file.bytes.get(semi), Some(b' ' | b'\t' | b'\x0c' | b'\x0b')) {
        semi += 1;
    }
    (file.bytes.get(semi) == Some(&b';')).then_some(Parts {
        predicate,
        body,
        other,
        semi,
        unless,
    })
}
fn conditional_parent<'a, 'pr: 'a>(mut parents: impl Iterator<Item = &'a Node<'pr>>) -> bool {
    parents
        .find(|n| {
            n.as_else_node().is_none()
                && !n.as_statements_node().is_some_and(|s| s.body().len() <= 1)
        })
        .is_some_and(|n| n.as_if_node().is_some() || n.as_unless_node().is_some())
}
fn newline(body: Option<&StatementsNode<'_>>) -> bool {
    body.is_some_and(|s| {
        s.body().len() > 1
            || s.body().iter().any(|n| {
                n.as_parentheses_node().is_some()
                    || n.as_return_node().is_some_and(|n| n.arguments().is_some())
            })
    })
}
fn special(body: Option<&StatementsNode<'_>>) -> bool {
    body.is_some_and(|s| {
        s.body().iter().any(|n| {
            n.as_multi_write_node().is_some()
                || n.as_lambda_node().is_some()
                || n.as_call_node()
                    .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()))
                || n.as_super_node()
                    .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()))
                || n.as_forwarding_super_node()
                    .is_some_and(|c| c.block().is_some())
        })
    })
}
fn nested_else(p: &Parts<'_>) -> bool {
    p.other_body().is_some_and(|s| {
        s.body().len() == 1
            && s.body()
                .iter()
                .any(|n| n.as_if_node().is_some() || n.as_unless_node().is_some())
    })
}
fn text<'pr>(node: &Node<'pr>) -> Option<&'pr str> {
    std::str::from_utf8(node.location().as_slice()).ok()
}
/// RuboCop's `autocorrect`.
fn correction(node: &Node<'_>, p: &Parts<'_>, cx: &NodeContext<'_, '_>) -> Option<Correction> {
    let loc = node.location();
    let newline_only = newline(p.body.as_ref())
        || newline(p.other_body().as_ref())
        || special(p.body.as_ref())
        || special(p.other_body().as_ref());
    if newline_only {
        return Some(Correction {
            range: p.semi..p.semi + 1,
            replacement: "\n".into(),
        });
    }
    let replacement = if else_is_conditional(node) {
        elsif_replacement(node)?
    } else {
        let condition = if assignment(&p.predicate) {
            format!("({})", text(&p.predicate)?)
        } else {
            operand(&p.predicate)?
        };
        let mut yes = branch(p.body.as_ref())?;
        let mut no = branch(p.other_body().as_ref())?;
        if p.unless {
            std::mem::swap(&mut yes, &mut no);
        }
        let ternary = format!("{condition} ? {yes} : {no}");
        if operand_parent(node, cx) {
            format!("({ternary})")
        } else {
            ternary
        }
    };
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement,
    })
}
fn branch(body: Option<&StatementsNode<'_>>) -> Option<String> {
    match body.and_then(|s| s.body().iter().next()) {
        Some(n) => operand(&n),
        None => Some("nil".to_owned()),
    }
}
fn assignment(n: &Node<'_>) -> bool {
    variables::is_local_assignment(n)
        || matches!(
            n,
            Node::InstanceVariableWriteNode { .. }
                | Node::ClassVariableWriteNode { .. }
                | Node::GlobalVariableWriteNode { .. }
                | Node::ConstantWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
                | Node::InstanceVariableOrWriteNode { .. }
                | Node::InstanceVariableAndWriteNode { .. }
                | Node::InstanceVariableOperatorWriteNode { .. }
                | Node::ClassVariableOrWriteNode { .. }
                | Node::ClassVariableAndWriteNode { .. }
                | Node::ClassVariableOperatorWriteNode { .. }
                | Node::GlobalVariableOrWriteNode { .. }
                | Node::GlobalVariableAndWriteNode { .. }
                | Node::GlobalVariableOperatorWriteNode { .. }
                | Node::ConstantOrWriteNode { .. }
                | Node::ConstantAndWriteNode { .. }
                | Node::ConstantOperatorWriteNode { .. }
                | Node::ConstantPathOrWriteNode { .. }
                | Node::ConstantPathAndWriteNode { .. }
                | Node::ConstantPathOperatorWriteNode { .. }
                | Node::CallOrWriteNode { .. }
                | Node::CallAndWriteNode { .. }
                | Node::CallOperatorWriteNode { .. }
                | Node::IndexOrWriteNode { .. }
                | Node::IndexAndWriteNode { .. }
                | Node::IndexOperatorWriteNode { .. }
                | Node::MultiWriteNode { .. }
        )
}
/// RuboCop's `require_argument_parentheses?`.
fn command(call: &ruby_prism::CallNode<'_>) -> bool {
    !call.block().is_some_and(|b| b.as_block_node().is_some())
        && call.opening_loc().is_none()
        && (call.arguments().is_some()
            || call
                .block()
                .is_some_and(|n| n.as_block_argument_node().is_some()))
        && !matches!(
            call.name().as_slice(),
            b"+" | b"-" | b"*" | b"/" | b"%" | b"**" | b"[]" | b"[]="
        )
}
fn lower(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::RescueModifierNode { .. }
            | Node::WhileNode { .. }
            | Node::UntilNode { .. }
            | Node::MatchPredicateNode { .. }
            | Node::MatchRequiredNode { .. }
    ) || n.as_and_node().is_some()
        || n.as_or_node().is_some()
        || n.as_call_node().is_some_and(|c| {
            c.name().as_slice() == b"!"
                && (c.message_loc().is_some_and(|l| l.as_slice() == b"not")
                    || c.receiver().is_some_and(|r| {
                        r.as_call_node().is_some_and(|c| command(&c)) || lower(&r)
                    }))
        })
        || n.as_yield_node()
            .is_some_and(|n| n.arguments().is_some() && n.lparen_loc().is_none())
        || n.as_super_node()
            .is_some_and(|n| n.arguments().is_some() && n.lparen_loc().is_none())
        || n.as_defined_node()
            .is_some_and(|n| n.lparen_loc().is_none())
}
fn operand(n: &Node<'_>) -> Option<String> {
    let source = text(n)?;
    if lower(n) {
        return Some(format!("({source})"));
    }
    if let Some(c) = n.as_call_node()
        && command(&c)
    {
        let first = ast::call_arguments(&c).into_iter().next()?;
        let base = n.location().start_offset();
        let start = first.location().start_offset() - base;
        let selector = c.message_loc()?.end_offset() - base;
        return Some(format!("{}({})", &source[..selector], &source[start..]));
    }
    Some(source.to_owned())
}
fn operand_parent(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut parents = cx.ancestors();
    // Prism arguments are transparent, but ordinary argument calls do not
    // require wrapping. Operator arguments do.
    let Some(mut parent) = parents.next() else {
        return false;
    };
    if parent.as_arguments_node().is_some() {
        let Some(n) = parents.next() else {
            return false;
        };
        parent = n;
    }
    if parent.as_and_node().is_some() || parent.as_or_node().is_some() {
        return true;
    }
    parent.as_call_node().is_some_and(|c| {
        if c.message_loc().is_some_and(|l| l.as_slice() == b"not") {
            return false;
        }
        c.receiver()
            .is_some_and(|r| r.location().start_offset() == node.location().start_offset())
            || (ast::is_operator_method(c.name().as_slice())
                && !matches!(c.name().as_slice(), b"[]" | b"[]="))
    })
}
/// A conditional's parts as rubocop-ast sees them: the else branch is
/// another conditional when it is one (an `elsif`, or an else body that is a
/// lone `if`/`unless`).
enum Else<'pr> {
    Conditional(Node<'pr>),
    Body(StatementsNode<'pr>),
}
fn parts<'pr>(
    n: &Node<'pr>,
) -> Option<(Node<'pr>, Option<StatementsNode<'pr>>, Option<Else<'pr>>)> {
    let lone = |body: StatementsNode<'pr>| {
        let mut nodes = body.body().iter();
        match (nodes.next(), nodes.next()) {
            (Some(n), None) if n.as_if_node().is_some() || n.as_unless_node().is_some() => {
                Else::Conditional(n)
            }
            _ => Else::Body(body),
        }
    };
    if let Some(c) = n.as_if_node() {
        let other = c.subsequent().and_then(|o| {
            if o.as_if_node().is_some() {
                Some(Else::Conditional(o))
            } else {
                o.as_else_node()?.statements().map(lone)
            }
        });
        return Some((c.predicate(), c.statements(), other));
    }
    let c = n.as_unless_node()?;
    let other = c.else_clause().and_then(|e| e.statements()).map(lone);
    Some((c.predicate(), c.statements(), other))
}
fn else_is_conditional(node: &Node<'_>) -> bool {
    matches!(parts(node), Some((_, _, Some(Else::Conditional(_)))))
}
/// RuboCop's `correct_elsif` and `build_else_branch`.
fn elsif_replacement(node: &Node<'_>) -> Option<String> {
    let body = |b: Option<&StatementsNode<'_>>| -> Option<String> {
        b.map_or(Some(String::new()), |s| {
            std::str::from_utf8(s.location().as_slice())
                .ok()
                .map(str::to_owned)
        })
    };
    let (predicate, statements, mut other) = parts(node)?;
    let mut result = format!(
        "if {}\n  {}\n",
        text(&predicate)?,
        body(statements.as_ref())?
    );
    while let Some(Else::Conditional(n)) = other {
        let (predicate, statements, next) = parts(&n)?;
        result.push_str(&format!(
            "elsif {}\n  {}\n",
            text(&predicate)?,
            body(statements.as_ref())?
        ));
        if let Some(Else::Body(b)) = &next {
            result.push_str(&format!("else\n  {}\n", body(Some(b))?));
        }
        other = next;
    }
    result.push_str("end");
    Some(result)
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! RuboCop spec examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn lookalikes_and_direct_conditional_children_are_ignored() {
        for s in [
            "work if ready\n",
            "work unless ready\n",
            "if ready then work end\n",
            "if ready\n work; more\nend\n",
            "if outer\n if inner; work end\nend\n",
            "# if ready; work end\n'if ready; work end'\n",
        ] {
            assert!(
                crate::pipeline::lint_str_with(&[&IfWithSemicolon], s).is_empty(),
                "{s}"
            );
        }
    }
    #[test]
    fn literal_super_blocks_recommend_if_else_and_only_replace_the_semicolon() {
        for keyword in ["if", "unless"] {
            for call in [
                "super { work }",
                "super() { work }",
                "super(value) { work }",
                "super do work end",
                "super() do work end",
            ] {
                for (yes, no) in [(call, "other"), ("other", call)] {
                    let source = format!("def f; {keyword} ready; {yes} else {no} end; end\n");
                    let found =
                        crate::cops::testing::correction_offenses(&IfWithSemicolon, &source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (1, 8));
                    assert_eq!(
                        found[0].message,
                        format!("Do not use `{keyword} ready;` - use `if/else` instead."),
                        "{source}"
                    );
                    crate::cops::testing::assert_correction(
                        &IfWithSemicolon,
                        &source,
                        &format!("def f; {keyword} ready\n {yes} else {no} end; end\n"),
                    );
                }
            }
        }
    }
    #[test]
    fn complex_assignment_conditions_keep_the_assigned_value() {
        for condition in [
            "@value ||= false",
            "$value &&= true",
            "Box::VALUE = false",
            "object[0] ||= false",
            "object.value += 1",
        ] {
            let before = format!("if {condition}; :yes else :no end\n");
            let after = format!("({condition}) ? :yes : :no\n");
            crate::cops::testing::assert_correction(&IfWithSemicolon, &before, &after);
        }
    }
    #[test]
    fn all_pinned_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_with_semicolon_pinned.json"
        ))
        .unwrap();
        assert_eq!(
            cases.iter().filter(|c| c["correction"].is_string()).count(),
            44
        );
        for c in cases {
            if let Some(after) = c["correction"].as_str() {
                let source = c["source"].as_str().unwrap();
                let (source, after) = if source.contains("yield baz") {
                    (
                        format!("def fixture\n{source}end\n"),
                        format!("def fixture\n{after}end\n"),
                    )
                } else {
                    (source.to_owned(), after.to_owned())
                };
                crate::cops::testing::assert_correction(&IfWithSemicolon, &source, &after);
            }
        }
    }
    #[test]
    fn all_pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_with_semicolon_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 45);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            // The pin puts `yield` at top level; Prism correctly rejects it.
            let wrapped = source.contains("yield baz");
            let source = if wrapped {
                format!("def fixture\n{source}end\n")
            } else {
                source.to_owned()
            };
            let found = crate::pipeline::lint_str_with(&[&IfWithSemicolon], &source);
            assert_eq!(
                serde_json::json!(
                    found
                        .iter()
                        .map(|o| (o.line - u32::from(wrapped), o.col))
                        .collect::<Vec<_>>()
                ),
                c["positions"],
                "spec {}",
                c["spec_line"]
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "spec {}",
                c["spec_line"]
            );
        }
    }
}
