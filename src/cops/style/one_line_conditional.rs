//! `Style/OneLineConditional` — prefer ternaries or multiline conditionals.
use crate::{
    cops::{
        Cop, NodeContext, ast, variables,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};
pub struct OneLineConditional;
impl Cop for OneLineConditional {
    fn name(&self) -> &'static str {
        "Style/OneLineConditional"
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
        let Some(p) = candidate(node, file) else {
            return;
        };
        let keyword = if p.unless { "unless" } else { "if" };
        let advice = if p.multiline {
            format!("multi-line `{keyword}`")
        } else {
            "the ternary operator (`?:`)".into()
        };
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Outermost candidates own overlapping edits. Nested findings remain.
                if cx.ancestors().any(|n| candidate(n, file).is_some()) {
                    return None;
                }
                rewrite(node, &p, cx, file)
            })
            .flatten();
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: format!(
                "Favor {advice} over single-line `{keyword}/then/else/end` constructs."
            ),
            correction,
        });
    }
}
struct Parts<'pr> {
    condition: Node<'pr>,
    body: Option<StatementsNode<'pr>>,
    other: Node<'pr>,
    unless: bool,
    multiline: bool,
}
fn begin_body(body: Option<&StatementsNode<'_>>) -> bool {
    body.is_some_and(|s| {
        s.body().len() > 1
            || s.body()
                .first()
                .is_some_and(|n| n.as_parentheses_node().is_some())
    })
}
fn candidate<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Parts<'pr>> {
    let (condition, body, other, unless) = if let Some(n) = node.as_if_node() {
        if n.end_keyword_loc().is_none() || n.if_keyword_loc()?.as_slice() != b"if" {
            return None;
        }
        (n.predicate(), n.statements(), n.subsequent()?, false)
    } else {
        let n = node.as_unless_node()?;
        n.end_keyword_loc()?;
        (
            n.predicate(),
            n.statements(),
            n.else_clause()?.as_node(),
            true,
        )
    };
    let loc = node.location();
    if file.line_of(loc.start_offset()) != file.last_line_of(loc) || begin_body(body.as_ref()) {
        return None;
    }
    let multiline = if other.as_if_node().is_some() {
        true
    } else {
        let statements = other.as_else_node()?.statements()?;
        if statements.body().is_empty() {
            return None;
        }
        statements.body().len() > 1
            || statements.body().first().is_some_and(|n| {
                n.as_parentheses_node()
                    .and_then(|p| p.body())
                    .and_then(|b| b.as_statements_node())
                    .is_some_and(|s| s.body().len() > 1)
            })
    };
    Some(Parts {
        condition,
        body,
        other,
        unless,
        multiline,
    })
}
fn text<'a>(n: &Node<'a>) -> Option<&'a str> {
    std::str::from_utf8(n.location().as_slice()).ok()
}
fn operand(n: &Node<'_>) -> Option<String> {
    let grouping = super::redundant_parentheses::is_assignment(n)
        || matches!(
            n,
            Node::AndNode { .. }
                | Node::OrNode { .. }
                | Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::WhileNode { .. }
                | Node::UntilNode { .. }
                | Node::RescueModifierNode { .. }
                | Node::MatchRequiredNode { .. }
                | Node::MatchPredicateNode { .. }
        )
        || n.as_call_node().is_some_and(|c| {
            c.equal_loc().is_some()
                || (c.opening_loc().is_none()
                    && (c.arguments().is_some() || c.block().is_some())
                    && (c.call_operator_loc().is_some()
                        || !ast::is_operator_method(c.name().as_slice())))
                || c.block().is_some_and(|b| b.as_block_node().is_some())
                || c.message_loc().is_some_and(|l| l.as_slice() == b"not")
        })
        || n.as_defined_node()
            .is_some_and(|n| n.lparen_loc().is_none())
        || n.as_yield_node()
            .is_some_and(|n| n.arguments().is_some() && n.lparen_loc().is_none())
        || n.as_super_node().is_some_and(|n| {
            (n.arguments().is_some() && n.lparen_loc().is_none()) || n.block().is_some()
        })
        || n.as_forwarding_super_node()
            .is_some_and(|n| n.block().is_some())
        || n.as_return_node().is_some_and(|n| n.arguments().is_some())
        || n.as_break_node().is_some_and(|n| n.arguments().is_some())
        || n.as_next_node().is_some_and(|n| n.arguments().is_some());
    Some(if grouping {
        format!("({})", text(n)?)
    } else {
        text(n)?.to_owned()
    })
}
fn branch(body: Option<&StatementsNode<'_>>) -> Option<String> {
    body.and_then(|s| s.body().first())
        .map_or_else(|| Some("nil".into()), |n| operand(&n))
}
fn tight_parent(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let Some(parent) = cx.ancestors().find(|n| n.as_arguments_node().is_none()) else {
        return false;
    };
    let same = |n: Node<'_>| n.location().start_offset() == node.location().start_offset();
    matches!(
        parent,
        Node::AndNode { .. }
            | Node::OrNode { .. }
            | Node::RangeNode { .. }
            | Node::FlipFlopNode { .. }
    ) || parent.as_call_node().is_some_and(|c| {
        c.receiver().is_some_and(same) || ast::is_operator_method(c.name().as_slice())
    }) || parent
        .as_constant_path_node()
        .is_some_and(|c| c.parent().is_some_and(same))
        || parent
            .as_if_node()
            .is_some_and(|n| n.if_keyword_loc().is_none() && same(n.predicate()))
}
fn rewrite(
    node: &Node<'_>,
    p: &Parts<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let range = node.location().start_offset()..node.location().end_offset();
    if range.len() > 16 * 1024 || super::correction::contains_comment(file, range.clone()) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, range.start) {
        return None;
    }
    struct Hazards {
        reorder: bool,
        unsafe_edit: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Hazards {
        fn enter(&mut self, n: Node<'pr>) {
            self.unsafe_edit |= matches!(
                n,
                Node::DefNode { .. }
                    | Node::ClassNode { .. }
                    | Node::ModuleNode { .. }
                    | Node::SingletonClassNode { .. }
                    | Node::RetryNode { .. }
                    | Node::RedoNode { .. }
            ) || self.reorder
                && (variables::is_local_assignment(&n)
                    || matches!(
                        n,
                        Node::MatchWriteNode { .. }
                            | Node::MatchRequiredNode { .. }
                            | Node::MatchPredicateNode { .. }
                    ));
        }
    }
    let mut hazards = Hazards {
        reorder: p.unless && !p.multiline,
        unsafe_edit: false,
    };
    walk(&mut hazards, node);
    if hazards.unsafe_edit {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    let replacement = if p.multiline {
        multiline(node, p, file)?
    } else {
        let mut yes = branch(p.body.as_ref())?;
        let mut no = branch(p.other.as_else_node()?.statements().as_ref())?;
        if p.unless {
            std::mem::swap(&mut yes, &mut no);
        }
        let result = format!("{} ? {yes} : {no}", operand(&p.condition)?);
        if tight_parent(node, cx) {
            format!("({result})")
        } else {
            result
        }
    };
    super::negated_if::finish(Correction { range, replacement }, cx, file)
}
fn multiline(node: &Node<'_>, p: &Parts<'_>, file: &SourceFile) -> Option<String> {
    if super::multiline_if_then::ends_with_open_range(&p.condition) {
        return None;
    }
    let (_, col) = file.line_col(node.location().start_offset());
    if col > 16 * 1024 {
        return None;
    }
    let indent = " ".repeat((col - 1) as usize);
    let end = file.line_start(file.line_of(node.location().start_offset()) + 1);
    let nl = if file.bytes.get(end.saturating_sub(2)..end) == Some(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let body = |s: Option<&StatementsNode<'_>>| -> Option<String> {
        s.map_or_else(
            || Some("nil".into()),
            |s| {
                std::str::from_utf8(s.location().as_slice())
                    .ok()
                    .map(str::to_owned)
            },
        )
    };
    let mut result = format!(
        "{} {}{nl}{indent}  {}{nl}{indent}",
        if p.unless { "unless" } else { "if" },
        text(&p.condition)?,
        body(p.body.as_ref())?
    );
    let mut other = p
        .other
        .as_else_node()
        .map(|n| n.as_node())
        .or_else(|| p.other.as_if_node().map(|n| n.as_node()));
    while let Some(n) = other {
        if result.len() > 16 * 1024 {
            return None;
        }
        if let Some(e) = n.as_else_node() {
            result.push_str(&format!(
                "else{nl}{indent}  {}{nl}{indent}",
                body(e.statements().as_ref())?
            ));
            break;
        }
        let e = n.as_if_node()?;
        if super::multiline_if_then::ends_with_open_range(&e.predicate()) {
            return None;
        }
        result.push_str(&format!(
            "elsif {}{nl}{indent}  {}{nl}{indent}",
            text(&e.predicate())?,
            body(e.statements().as_ref())?
        ));
        other = e.subsequent();
    }
    result.push_str("end");
    (result.len() <= 16 * 1024).then_some(result)
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! RuboCop one_line_conditional_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/one_line_conditional_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 61);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let wrapped = source.contains("yield");
            let source = if wrapped {
                format!("def fixture\n{source}end\n")
            } else {
                source.to_owned()
            };
            let found: Vec<_> = crate::pipeline::lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == OneLineConditional.name())
                .collect();
            assert_eq!(
                serde_json::json!(
                    found
                        .iter()
                        .map(|o| (o.line - u32::from(wrapped), o.col))
                        .collect::<Vec<_>>()
                ),
                c["positions"],
                "{c}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{c}"
            );
        }
    }
    #[test]
    fn pinned_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/one_line_conditional_pinned.json"
        ))
        .unwrap();
        assert_eq!(
            cases.iter().filter(|c| c["correction"].is_string()).count(),
            56
        );
        for c in cases {
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            let before = c["source"].as_str().unwrap();
            let (before, after) = if before.contains("yield") {
                (
                    format!("def fixture\n{before}end\n"),
                    format!("def fixture\n{after}end\n"),
                )
            } else {
                (before.to_owned(), after.to_owned())
            };
            crate::cops::testing::assert_correction(&OneLineConditional, &before, &after);
        }
    }
    #[test]
    fn literal_blocks_and_transfer_arguments_keep_binding() {
        for (before, after) in [
            (
                "if c then foo() do 1 end else 2 end\n",
                "c ? (foo() do 1 end) : 2\n",
            ),
            (
                "def f; if c then super() do 1 end else 2 end; end\n",
                "def f; c ? (super() do 1 end) : 2; end\n",
            ),
            (
                "def f; if c then super do 1 end else 2 end; end\n",
                "def f; c ? (super do 1 end) : 2; end\n",
            ),
            (
                "def f; if c then return 1 else return 2 end; end\n",
                "def f; c ? (return 1) : (return 2); end\n",
            ),
            (
                "loop { if c then next 1 else break 2 end }\n",
                "loop { c ? (next 1) : (break 2) }\n",
            ),
            (
                "loop { if c then break else 7 end }\n",
                "loop { c ? break : 7 }\n",
            ),
            (
                "if c then obj.+ 1 else obj.[] 2 end\n",
                "c ? (obj.+ 1) : (obj.[] 2)\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&OneLineConditional, before, after);
        }
    }
    #[test]
    fn binding_control_flow_and_source_sensitive_edits_stay_reported() {
        for source in [
            "unless c then x = 1 else x end\n",
            "unless c then /(?<x>a)/ =~ value else x end\n",
            "if c then def f; 1 end else 2 end\n",
            "begin; work; rescue; if c then retry else 2 end; end\n",
            "loop { if c then redo else 2 end }\n",
            "if c then <<~H else 2 end\nbody\nH\n",
            "if c then 1 elsif d then 2 end\np __LINE__\n",
            "if c then __LINE__ elsif d then 2 end\n",
            "if c then DATA.pos else 2 end\n__END__\npayload\n",
            "if c then 1 else 2 end # rubocop:disable Lint/Debugger\n",
            "if c then method(:f).source_location else 2 end\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&OneLineConditional, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn precedence_range_endpoints_and_elsif_association() {
        for (before, after) in [
            ("p(1..if c then 2 else 3 end)\n", "p(1..(c ? 2 : 3))\n"),
            ("p(if c then 2 else 3 end..5)\n", "p((c ? 2 : 3)..5)\n"),
            (
                "  if a then 1 elsif b then 2 else 3 end\n",
                "  if a\n    1\n  elsif b\n    2\n  else\n    3\n  end\n",
            ),
            (
                "unless a then 1 else 2; 3 end\n",
                "unless a\n  1\nelse\n  2; 3\nend\n",
            ),
            (
                "if a then begin b rescue c ensure d end else e end\n",
                "a ? begin b rescue c ensure d end : e\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&OneLineConditional, before, after);
        }
    }
    #[test]
    fn literal_envelope_line_values_and_payload_are_preserved() {
        crate::cops::testing::assert_correction(
            &OneLineConditional,
            "\u{feff}# coding: utf-8\r\nif c then \"é\\nvalue\" else /x/ end\r\np __LINE__\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\nc ? \"é\\nvalue\" : /x/\r\np __LINE__\r\n__END__\r\npayload\x00\r\n",
        );
        crate::cops::testing::assert_correction(
            &OneLineConditional,
            "if c then 1 elsif d then 2 else 3 end\r\n",
            "if c\r\n  1\r\nelsif d\r\n  2\r\nelse\r\n  3\r\nend\r\n",
        );
    }
    #[test]
    fn corrections_are_lazy_bounded_and_outermost_owned() {
        let source = "if c then 1 else if d then 2 else 3 end end\n".repeat(3000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&OneLineConditional], &source).len(),
            6000
        );
        assert_eq!(REWRITE_BYTES.with(|n| n.get()), 0);
        assert_eq!(
            crate::cops::testing::correction_offenses(&OneLineConditional, &source)
                .iter()
                .filter(|o| o.correction.is_some())
                .count(),
            3000
        );
        assert!(REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let wide = format!("{}if c then 1 elsif d then 2 end\n", " ".repeat(20000));
        assert!(
            crate::cops::testing::correction_offenses(&OneLineConditional, &wide)[0]
                .correction
                .is_none()
        );
        let huge = format!("if c then 1 else [{}] end\n", "1,".repeat(10000));
        assert!(
            crate::cops::testing::correction_offenses(&OneLineConditional, &huge)[0]
                .correction
                .is_none()
        );
    }
    #[test]
    fn setter_assignment_conditions_keep_grouping() {
        for assignment in ["a[0] = false", "a[0, 1] = nil", "obj.value = true"] {
            crate::cops::testing::assert_correction(
                &OneLineConditional,
                &format!("if {assignment} then :yes else :no end\n"),
                &format!("({assignment}) ? :yes : :no\n"),
            );
        }
    }
    #[test]
    fn canonical_ternary() {
        crate::cops::testing::assert_correction(
            &OneLineConditional,
            "if cond then run else dont end\n",
            "cond ? run : dont\n",
        );
    }
}
