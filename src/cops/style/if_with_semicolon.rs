//! `Style/IfWithSemicolon` — semicolon-delimited conditional headers.
use crate::{
    cops::{
        Cop, NodeContext, ast, variables,
        walk::{NodeVisitor, walk},
    },
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
                .then(|| correction(node, &parts, cx, file))
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
const REWRITE_BUDGET: usize = 16 * 1024;
fn text<'pr>(node: &Node<'pr>) -> Option<&'pr str> {
    std::str::from_utf8(node.location().as_slice()).ok()
}
fn correction(
    node: &Node<'_>,
    p: &Parts<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    if range.len() > REWRITE_BUDGET || super::correction::touches_directive(file, range.clone()) {
        return None;
    }
    let lines = file.line_start(file.line_of(range.start))
        ..file.line_start(file.line_of(range.end.saturating_sub(1)) + 1);
    if super::correction::contains_comment(file, lines) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, range.start) {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    let newline_only = newline(p.body.as_ref())
        || newline(p.other_body().as_ref())
        || special(p.body.as_ref())
        || special(p.other_body().as_ref())
        || nested_else(p);
    let end = file.line_start(file.line_of(p.semi) + 1);
    let nl = if file.bytes.get(end.saturating_sub(2)..end) == Some(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let edit = if newline_only {
        Correction {
            range: p.semi..p.semi + 1,
            replacement: nl.to_owned(),
        }
    } else {
        struct Hazards {
            reorder: bool,
            unsafe_edit: bool,
        }
        impl<'pr> NodeVisitor<'pr> for Hazards {
            fn enter(&mut self, n: Node<'pr>) {
                self.unsafe_edit |= matches!(
                    n,
                    Node::BreakNode { .. }
                        | Node::NextNode { .. }
                        | Node::RetryNode { .. }
                        | Node::RedoNode { .. }
                        | Node::DefNode { .. }
                        | Node::ClassNode { .. }
                        | Node::ModuleNode { .. }
                        | Node::SingletonClassNode { .. }
                ) || (self.reorder
                    && (variables::is_local_assignment(&n)
                        || matches!(
                            n,
                            Node::MatchWriteNode { .. }
                                | Node::MatchPredicateNode { .. }
                                | Node::MatchRequiredNode { .. }
                        )))
                    || n.as_call_node()
                        .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()));
            }
        }
        let mut hazards = Hazards {
            reorder: p.unless,
            unsafe_edit: false,
        };
        walk(&mut hazards, node);
        if hazards.unsafe_edit {
            return None;
        }
        let replacement = if p.other.as_ref().is_some_and(|n| n.as_if_node().is_some()) {
            elsif_replacement(p, nl)?
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
        if text(node)?.contains("__LINE__") && text(node)?.contains('\n') {
            return None;
        }
        Correction { range, replacement }
    };
    (super::correction::preserves_line_values(file, &edit)
        && super::correction::preserves_data_positions(file, cx, &edit))
    .then_some(edit)
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
        || n.as_call_node().is_some_and(|c| c.equal_loc().is_some())
}
fn command(call: &ruby_prism::CallNode<'_>) -> bool {
    call.opening_loc().is_none()
        && (call.arguments().is_some()
            || call
                .block()
                .is_some_and(|n| n.as_block_argument_node().is_some()))
        && !matches!(
            call.name().as_slice(),
            b"+" | b"-" | b"*" | b"/" | b"%" | b"**" | b"[]" | b"[]="
        )
        && call.equal_loc().is_none()
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
    ) || n
        .as_and_node()
        .is_some_and(|n| n.operator_loc().as_slice() == b"and")
        || n.as_or_node()
            .is_some_and(|n| n.operator_loc().as_slice() == b"or")
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
        let start = first.location().start_offset() - n.location().start_offset();
        return Some(format!(
            "{}({})",
            source[..start].trim_end(),
            &source[start..]
        ));
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
    if parent.as_and_node().is_some()
        || parent.as_or_node().is_some()
        || parent.as_range_node().is_some()
    {
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
fn elsif_replacement(p: &Parts<'_>, nl: &str) -> Option<String> {
    let body = |b: Option<&StatementsNode<'_>>| -> Option<String> {
        Some(
            b.map(|s| std::str::from_utf8(s.location().as_slice()).map(str::to_owned))
                .transpose()
                .ok()?
                .unwrap_or_default(),
        )
    };
    let mut result = format!(
        "if {}{nl}  {}{nl}",
        text(&p.predicate)?,
        body(p.body.as_ref())?
    );
    let mut next = p.other.as_ref().and_then(|n| n.as_if_node());
    while let Some(n) = next {
        result.push_str(&format!(
            "elsif {}{nl}  {}{nl}",
            text(&n.predicate())?,
            body(n.statements().as_ref())?
        ));
        let other = n.subsequent();
        if let Some(e) = other.as_ref().and_then(|n| n.as_else_node()) {
            result.push_str(&format!("else{nl}  {}{nl}", body(e.statements().as_ref())?));
        }
        next = other.and_then(|n| n.as_if_node());
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
    fn escaped_header_continuation_and_nested_else_association() {
        let source = "if ready \\\n; work end\n";
        let found = crate::pipeline::lint_str_with(&[&IfWithSemicolon], source);
        assert!(found.is_empty());
        let source = "if outer; yes else unless inner; no else other end end\n";
        let found = crate::cops::testing::correction_offenses(&IfWithSemicolon, source);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "Do not use `if outer;` - use `if/else` instead."
        );
        crate::cops::testing::assert_correction(
            &IfWithSemicolon,
            source,
            "if outer\n yes else unless inner; no else other end end\n",
        );
    }
    #[test]
    fn range_endpoints_keep_ternary_grouping() {
        for operator in ["..", "..."] {
            for condition in ["true", "false"] {
                let before = format!("if {condition}; 2 else 3 end");
                let after = format!("({condition} ? 2 : 3)");
                for (before, after) in [
                    (
                        format!("{before}{operator}5"),
                        format!("{after}{operator}5"),
                    ),
                    (
                        format!("1{operator}{before}"),
                        format!("1{operator}{after}"),
                    ),
                    (format!("{operator}{before}"), format!("{operator}{after}")),
                    (format!("{before}{operator}"), format!("{after}{operator}")),
                ] {
                    crate::cops::testing::assert_correction(
                        &IfWithSemicolon,
                        &format!("p({before})\n"),
                        &format!("p({after})\n"),
                    );
                }
            }
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
    fn crlf_elsif_copies_multiline_literal_bytes_without_transcoding_newlines() {
        crate::cops::testing::assert_correction(
            &IfWithSemicolon,
            "if a; \"x\r\ny\" elsif b; \"z\" end\r\n",
            "if a\r\n  \"x\r\ny\"\r\nelsif b\r\n  \"z\"\r\nend\r\n",
        );
    }
    #[test]
    fn block_pass_only_commands_keep_argument_binding() {
        // Opaque block dispatch retains the shared source-observer barrier.
        let found = crate::cops::testing::correction_offenses(
            &IfWithSemicolon,
            "block = nil; other = nil\nif cond; foo &block else bar &other end\n",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        crate::cops::testing::assert_correction(
            &IfWithSemicolon,
            "if cond; foo &:to_s else bar &:to_i end\n",
            "cond ? foo(&:to_s) : bar(&:to_i)\n",
        );
        crate::cops::testing::assert_correction(
            &IfWithSemicolon,
            "if foo &:to_s; run end\n",
            "foo(&:to_s) ? run : nil\n",
        );
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
    fn individually_withholds_ambiguous_and_source_sensitive_edits() {
        for s in [
            "unless ready; x = 1 else x end\n",
            "unless ready; /(?<x>a)/ =~ value else x end\n",
            "loop { if ready; break value end }\n",
            "loop { if ready; next value end }\n",
            "if ready; work # keep association\nelse\n more\nend\n",
            "if ready; work; more end\np __LINE__\n",
            "if ready; DATA.pos else 0 end\n__END__\npayload\n",
            "if ready; <<~TEXT else 0 end\nbody\nTEXT\n",
            "if ready; work end # rubocop:disable Style/StringLiterals\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&IfWithSemicolon, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn safe_source_envelopes_keep_literals_lines_and_payload() {
        crate::cops::testing::assert_correction(
            &IfWithSemicolon,
            "\u{feff}# encoding: UTF-8\r\nif ready; \"é\\nvalue\" else :none end\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nready ? \"é\\nvalue\" : :none\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
        crate::cops::testing::assert_correction(
            &IfWithSemicolon,
            "if ready; work; more end\r\n",
            "if ready\r\n work; more end\r\n",
        );
    }
    #[test]
    fn source_work_is_lazy_and_bounded() {
        let s = "if ready; work else other end\n".repeat(3000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&IfWithSemicolon], &s).len(),
            3000
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        assert!(
            crate::cops::testing::correction_offenses(&IfWithSemicolon, &s)
                .iter()
                .all(|o| o.correction.is_some())
        );
        REWRITE_BYTES.with(|n| assert!(n.get() <= s.len()));
        let s = format!("if ready; {} end\n", "work;".repeat(5000));
        REWRITE_BYTES.with(|n| n.set(0));
        assert!(
            crate::cops::testing::correction_offenses(&IfWithSemicolon, &s)[0]
                .correction
                .is_none()
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
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
