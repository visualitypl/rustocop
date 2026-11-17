//! `Style/IfUnlessModifierOfIfUnless` — modifiers on conditional bodies.
use crate::{
    cops::{
        Cop, NodeContext, variables,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub struct IfUnlessModifierOfIfUnless;
impl Cop for IfUnlessModifierOfIfUnless {
    fn name(&self) -> &'static str {
        "Style/IfUnlessModifierOfIfUnless"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/SoleNestedConditional"]
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((keyword, _, _)) = candidate(node) else {
            return;
        };
        let (line, col) = file.line_col(keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Avoid modifier `{}` after another conditional.",
                std::str::from_utf8(keyword.as_slice()).unwrap()
            ),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| correction(node, cx, file))
                .flatten(),
        });
    }
}
fn candidate<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Node<'pr>, Node<'pr>)> {
    let (keyword, predicate, statements) = if let Some(n) = node.as_if_node() {
        if n.end_keyword_loc().is_some() {
            return None;
        }
        (n.if_keyword_loc()?, n.predicate(), n.statements()?)
    } else {
        let n = node.as_unless_node()?;
        if n.end_keyword_loc().is_some() {
            return None;
        }
        (n.keyword_loc(), n.predicate(), n.statements()?)
    };
    if statements.body().len() != 1 {
        return None;
    }
    let body = statements.body().iter().next()?;
    (body.as_if_node().is_some() || body.as_unless_node().is_some())
        .then_some((keyword, predicate, body))
}
const REWRITE_BUDGET: usize = 16 * 1024;
fn correction(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> Option<Correction> {
    // The outermost modifier owns a whole direct modifier chain atomically.
    if cx
        .ancestors()
        .find(|n| n.as_statements_node().is_none())
        .is_some_and(|n| candidate(n).is_some())
    {
        return None;
    }
    let loc = node.location();
    let range = loc.start_offset()..loc.end_offset();
    if range.len() > REWRITE_BUDGET || super::correction::touches_directive(file, range.clone()) {
        return None;
    }
    let prefix = &file.bytes[file.line_start(file.line_of(range.start))..range.start];
    let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    // Do not invent precedence/statement boundaries in larger expressions.
    if prefix.len() > REWRITE_BUDGET
        || (!prefix.iter().all(|b| matches!(b, b' ' | b'\t')) && !prefix.ends_with(b"("))
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, range.start) {
        return None;
    }
    // Comments on any touched line stay reported but unchanged: otherwise a
    // trailing comment could change association or activate a magic header.
    let lines = file.line_start(file.line_of(range.start))
        ..file.line_start(file.line_of(range.end.saturating_sub(1)) + 1);
    if super::correction::contains_comment(file, lines) {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    struct Hazards(bool);
    impl<'pr> NodeVisitor<'pr> for Hazards {
        fn enter(&mut self, n: Node<'pr>) {
            // Reversing lexical order must not change Ruby's local/call
            // classification, named captures, pattern bindings or definitions.
            self.0 |= variables::is_local_assignment(&n)
                || matches!(
                    n,
                    Node::MatchWriteNode { .. }
                        | Node::MatchPredicateNode { .. }
                        | Node::MatchRequiredNode { .. }
                        | Node::DefNode { .. }
                        | Node::ClassNode { .. }
                        | Node::ModuleNode { .. }
                        | Node::SingletonClassNode { .. }
                        | Node::BeginNode { .. }
                        | Node::RescueModifierNode { .. }
                        | Node::PreExecutionNode { .. }
                        | Node::PostExecutionNode { .. }
                );
        }
    }
    let mut hazards = Hazards(false);
    walk(&mut hazards, node);
    if hazards.0 {
        return None;
    }
    let end = file.line_start(file.line_of(range.start) + 1);
    let nl = if file.bytes.get(end.saturating_sub(2)..end) == Some(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut replacement = String::new();
    let mut suffix = String::new();
    let mut current = candidate(node)?;
    loop {
        let (keyword, predicate, body) = current;
        let condition = std::str::from_utf8(predicate.location().as_slice()).ok()?;
        if condition.contains(['\n', '\r']) {
            return None;
        }
        replacement.push_str(std::str::from_utf8(keyword.as_slice()).ok()?);
        replacement.push(' ');
        replacement.push_str(condition);
        replacement.push_str(nl);
        suffix.push_str(nl);
        suffix.push_str("end");
        if let Some(next) = candidate(&body) {
            current = next;
        } else {
            replacement.push_str(std::str::from_utf8(body.location().as_slice()).ok()?);
            break;
        }
    }
    replacement.push_str(&suffix);
    let edit = Correction { range, replacement };
    Some(edit)
}
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/if_unless_modifier_of_if_unless_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn found(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        correction_offenses(&IfUnlessModifierOfIfUnless, source)
    }
    #[test]
    fn normal_nested_guards_and_parenthesized_bodies_are_not_modifiers_of_conditionals() {
        for source in [
            "if outer\n if inner\n work\n end\nend\n",
            "if outer\n work if inner\nend\n",
            "(a ? b : c) if guard\n",
            "begin; if a; b; end; end unless guard\n",
            "work if ready\n",
            "a ? b : (c if ready)\n",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn lexical_declarations_captures_and_ambiguous_control_flow_stay_reported() {
        for source in [
            "if a\n value = 1\nend if value\n",
            "value ? b : c if (value = fetch)\n",
            "if a\n /(?<value>x)/ =~ input\nend if value\n",
            "if a\n [1] => [value]\nend if ready\n",
            "if a\n begin; work; rescue; recover; ensure; cleanup; end\nend if ready\n",
            "if a\n work rescue recover\nend unless ready\n",
            "if a\n def f; work; end\nend if ready\n",
        ] {
            let found = found(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn comments_heredocs_and_multiline_predicates_are_individually_withheld() {
        for source in [
            "a ? b : c if ready # keep comment\n",
            "if a # header\n b\nelse\n c\nend if ready\n",
            "if a\n # branch\n b\nend if ready\n",
            "if a\n <<~TEXT\nbody\nTEXT\nend if ready\n",
            "a ? b : c if (\nready\n)\n",
            "before; a ? b : c if ready\n",
        ] {
            let found = found(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn canonical_expansion_preserves_bom_crlf_literal_and_unread_payload_bytes() {
        assert_correction(
            &IfUnlessModifierOfIfUnless,
            "\u{feff}# encoding: UTF-8\r\na ? \"é\\nvalue\" : :none unless guard\r\n__END__\r\nraw payload\r\n",
            "\u{feff}# encoding: UTF-8\r\nunless guard\r\na ? \"é\\nvalue\" : :none\r\nend\r\n__END__\r\nraw payload\r\n",
        );
        assert_correction(
            &IfUnlessModifierOfIfUnless,
            "(a ? b : c if ready)\n",
            "(if ready\na ? b : c\nend)\n",
        );
        assert_correction(
            &IfUnlessModifierOfIfUnless,
            "begin\n if a\n  work\n elsif b\n  more\n else\n  fallback\n end unless guard\nrescue\n recover\nensure\n cleanup\nend\n",
            "begin\n unless guard\nif a\n  work\n elsif b\n  more\n else\n  fallback\n end\nend\nrescue\n recover\nensure\n cleanup\nend\n",
        );
    }
    #[test]
    fn correction_work_is_lazy_and_bounded() {
        let source = "a ? b : c if ready\n".repeat(2000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&IfUnlessModifierOfIfUnless], &source).len(),
            2000
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        assert!(found(&source).iter().all(|o| o.correction.is_some()));
        REWRITE_BYTES.with(|n| assert!(n.get() <= source.len()));
        REWRITE_BYTES.with(|n| n.set(0));
        let source = format!("if a\n{}end if ready\n", " work\n".repeat(5000));
        assert!(found(&source)[0].correction.is_none());
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
    }
    #[test]
    fn all_pinned_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_unless_modifier_of_if_unless_pinned.json"
        ))
        .unwrap();
        assert_eq!(
            cases.iter().filter(|c| c["correction"].is_string()).count(),
            5
        );
        for c in cases {
            if let Some(after) = c["correction"].as_str() {
                crate::cops::testing::assert_correction(
                    &IfUnlessModifierOfIfUnless,
                    c["source"].as_str().unwrap(),
                    after,
                );
            }
        }
    }
    #[test]
    fn all_pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/if_unless_modifier_of_if_unless_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 7);
        for c in cases {
            let found: Vec<_> = crate::pipeline::lint_str(c["source"].as_str().unwrap())
                .into_iter()
                .filter(|o| o.cop == IfUnlessModifierOfIfUnless.name())
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
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
