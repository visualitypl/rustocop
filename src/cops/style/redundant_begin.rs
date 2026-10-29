//! `Style/RedundantBegin` — redundant explicit exception/grouping wrappers.
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

#[cfg(test)]
thread_local! {
    static SEARCH_NODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct RedundantBegin {
    max: Option<usize>,
}
impl Default for RedundantBegin {
    fn default() -> Self {
        Self { max: Some(120) }
    }
}
impl Cop for RedundantBegin {
    fn name(&self) -> &'static str {
        "Style/RedundantBegin"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/BlockDelimiters"]
    }
    fn with_line_length_limit(&self, max: Option<usize>) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self { max }))
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(begin) = node.as_begin_node() else {
            return;
        };
        let Some(keyword) = begin.begin_keyword_loc() else {
            return;
        };
        let mut ancestors = cx.ancestors();
        let parent = semantic_parent(ancestors.by_ref());
        let forced = forced_context(node, parent, semantic_parent(ancestors));
        if !forced && (!self.generic(node, parent, file) || self.has_nested_candidate(node, file)) {
            return;
        }
        let (line, col) = file.line_col(keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Redundant `begin` block detected.".into(),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, parent, forced, cx, file))
                .flatten(),
        });
    }
}

/// Singleton statement lists are transparent; a multi-statement list is the
/// parser-gem's synthetic begin, not the surrounding method/branch. A singleton
/// Program is the one context where a kwbegin has no semantic parent.
fn semantic_parent<'a, 'pr>(
    ancestors: impl Iterator<Item = &'a Node<'pr>>,
) -> Option<&'a Node<'pr>> {
    ancestors
        .filter(|n| {
            !n.as_statements_node().is_some_and(|s| s.body().len() == 1)
                && n.as_arguments_node().is_none()
        })
        .find(|n| n.as_program_node().is_none())
}

fn forced_context(node: &Node<'_>, parent: Option<&Node<'_>>, owner: Option<&Node<'_>>) -> bool {
    let Some(p) = parent else { return false };
    let begin = node.as_begin_node().unwrap();
    let protected = begin.rescue_clause().is_some() || begin.ensure_clause().is_some();
    let start = node.location().start_offset();
    if let Some(def) = p.as_def_node() {
        return def.equal_loc().is_none()
            && def
                .body()
                .is_some_and(|b| b.location().start_offset() == start);
    }
    if let Some(block) = p.as_block_node() {
        return block.opening_loc().as_slice() == b"do";
    }
    if protected {
        return false;
    }
    if let Some(n) = p.as_if_node() {
        return n.end_keyword_loc().is_some() && start > n.predicate().location().end_offset();
    }
    if let Some(n) = p.as_unless_node() {
        return n.end_keyword_loc().is_some() && start > n.predicate().location().end_offset();
    }
    if let Some(n) = p.as_while_node() {
        return n.closing_loc().is_some()
            && !n.is_begin_modifier()
            && start > n.predicate().location().end_offset();
    }
    if let Some(n) = p.as_until_node() {
        return n.closing_loc().is_some()
            && !n.is_begin_modifier()
            && start > n.predicate().location().end_offset();
    }
    matches!(
        p,
        Node::ElseNode { .. } | Node::WhenNode { .. } | Node::InNode { .. }
    ) && match p {
        Node::WhenNode { .. } => p
            .as_when_node()
            .unwrap()
            .statements()
            .is_some_and(|s| s.location().start_offset() == start),
        Node::InNode { .. } => p
            .as_in_node()
            .unwrap()
            .statements()
            .is_some_and(|s| s.location().start_offset() == start),
        // Rescue else clauses are not conditional branches: they retain the
        // generic exemptions for empty and multi-expression begin wrappers.
        _ => owner.is_some_and(|n| {
            matches!(
                n,
                Node::IfNode { .. }
                    | Node::UnlessNode { .. }
                    | Node::CaseNode { .. }
                    | Node::CaseMatchNode { .. }
            )
        }),
    }
}

impl RedundantBegin {
    fn generic(&self, node: &Node<'_>, parent: Option<&Node<'_>>, file: &SourceFile) -> bool {
        let Some(b) = node.as_begin_node() else {
            return false;
        };
        if b.begin_keyword_loc().is_none()
            || b.rescue_clause().is_some()
            || b.ensure_clause().is_some()
        {
            return false;
        }
        let Some(statements) = b.statements() else {
            return false;
        };
        if statements.body().len() > 1 && parent.is_some() {
            return false;
        }
        let Some(p) = parent else { return true };
        if p.as_call_node().is_some()
            || call_write_operand(p, node)
            || matches!(p, Node::AndNode { .. } | Node::OrNode { .. })
        {
            return false;
        }
        if p.as_while_node().is_some_and(|n| n.is_begin_modifier())
            || p.as_until_node().is_some_and(|n| n.is_begin_modifier())
        {
            return false;
        }
        if assignment(p)
            && let Some(max) = self.max
        {
            let first = statements.body().first().unwrap();
            if file.line_of(first.location().start_offset()) == file.last_line_of(first.location())
            {
                let length = file.line_col(first.location().end_offset()).1
                    - file.line_col(first.location().start_offset()).1;
                if (file.line_col(node.location().start_offset()).1 - 1 + length) as usize
                    + usize::from(low_precedence(&first)) * 2
                    > max
                {
                    return false;
                }
            }
        }
        true
    }
    fn has_nested_candidate(&self, node: &Node<'_>, file: &SourceFile) -> bool {
        struct Search<'a, 'pr> {
            cop: &'a RedundantBegin,
            file: &'a SourceFile,
            stack: Vec<Node<'pr>>,
            found: bool,
        }
        impl<'pr> NodeVisitor<'pr> for Search<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                SEARCH_NODES.set(SEARCH_NODES.get() + 1);
                if !self.stack.is_empty()
                    && self
                        .cop
                        .generic(&node, semantic_parent(self.stack.iter().rev()), self.file)
                {
                    self.found = true;
                }
                self.stack.push(node);
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        // Each subtree is visited at most MAX_AST_DEPTH times; no source scans
        // or reparses. Match upstream's deepest generic kwbegin, without hiding
        // independent contextual (method/block/branch) findings.
        let mut search = Search {
            cop: self,
            file,
            stack: Vec::new(),
            found: false,
        };
        walk(&mut search, node);
        search.found
    }
}

fn correction(
    node: &Node<'_>,
    parent: Option<&Node<'_>>,
    forced: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    use crate::offense::Correction;
    let b = node.as_begin_node()?;
    let begin = b.begin_keyword_loc()?;
    let end = b.end_keyword_loc()?;
    let root = cx.ancestors().next_back()?;
    let end_line = file.line_of(end.start_offset());
    let range = begin.start_offset()..end.end_offset();
    if file.contains_heredoc(root, node)
        || super::correction::touches_directive(file, range.clone())
    {
        return None;
    }
    // Token deletion can activate a trailing encoding/frozen header. Ordinary
    // comments remain byte-for-byte in place, including their physical lines.
    for token in [&begin, &end] {
        let line_end = file.line_start(file.line_of(token.end_offset()) + 1);
        let comments = file.comments();
        let index = comments.partition_point(|c| c.end <= token.end_offset());
        if let Some(comment) = comments.get(index).filter(|c| c.start < line_end) {
            // A long shared trailing comment must not be rescanned once per
            // wrapper on a dense line. Keep that individual edit unmodified.
            if comment.len() > 4096 {
                return None;
            }
            #[cfg(test)]
            REWRITE_BYTES.set(REWRITE_BYTES.get() + comment.len());
            if !super::correction::comment_relocation_safe(&file.bytes[comment.clone()]) {
                return None;
            }
        }
    }
    #[cfg(test)]
    REWRITE_BYTES.set(REWRITE_BYTES.get() + range.len());
    if parent.is_some_and(|p| {
        assignment(p)
            || matches!(
                p,
                Node::ReturnNode { .. } | Node::NextNode { .. } | Node::BreakNode { .. }
            )
    }) {
        let statements = b.statements()?;
        if statements.body().len() != 1 {
            return None;
        }
        let first = statements.body().first()?;
        // Upstream hoists the first expression onto the begin's line. Dynamic
        // execution/backtrace locations (and later lines) must not move. Only
        // already-single-line wrappers with whitespace-only seams are eligible.
        if file.line_of(begin.start_offset()) != end_line
            || !file.bytes[begin.end_offset()..first.location().start_offset()]
                .iter()
                .all(u8::is_ascii_whitespace)
            || !file.bytes[first.location().end_offset()..end.start_offset()]
                .iter()
                .all(u8::is_ascii_whitespace)
        {
            return None;
        }
        let text = std::str::from_utf8(first.location().as_slice()).ok()?;
        let text = if low_precedence(&first) {
            format!("({text})")
        } else {
            text.to_owned()
        };
        let correction = super::correction::combine(
            file,
            vec![
                Correction {
                    range: begin.start_offset()..first.location().end_offset(),
                    replacement: text,
                },
                Correction {
                    range: end.start_offset()..end.end_offset(),
                    replacement: String::new(),
                },
            ],
        )?;
        return super::correction::preserves_data_positions(file, cx, &correction)
            .then_some(correction);
    }
    // Removing an expression wrapper is only proven at statement boundaries.
    // Value/condition/collection contexts keep the finding but not a guess at
    // precedence. Endless definitions and multiline modifier guards require
    // moving executable expressions; preserve their backtrace lines instead.
    if let Some(p) = parent {
        if matches!(p, Node::IfNode { .. } | Node::UnlessNode { .. }) && !forced {
            let keyword = if let Some(n) = p.as_if_node() {
                n.if_keyword_loc()?
            } else {
                p.as_unless_node()?.keyword_loc()
            };
            // Only a simple inline modifier body is proven outside branches;
            // predicate wrappers and nested modifier precedence stay unedited.
            let first = b.statements()?.body().first()?;
            if node.location().start_offset() >= keyword.start_offset() || low_precedence(&first) {
                return None;
            }
        }
        if matches!(
            p,
            Node::WhenNode { .. } | Node::InNode { .. } | Node::DefNode { .. }
        ) && !forced
        {
            return None;
        }
        let body = match p {
            Node::ClassNode { .. } => p.as_class_node()?.body(),
            Node::SingletonClassNode { .. } => p.as_singleton_class_node()?.body(),
            _ => None,
        };
        if matches!(p, Node::ClassNode { .. } | Node::SingletonClassNode { .. })
            && !body.is_some_and(|body| {
                body.location().start_offset() == node.location().start_offset()
            })
        {
            return None;
        }
        if (p
            .as_if_node()
            .is_some_and(|n| n.end_keyword_loc().is_none())
            || p.as_unless_node()
                .is_some_and(|n| n.end_keyword_loc().is_none()))
            && file.line_of(begin.start_offset()) != end_line
        {
            return None;
        }
        if !forced
            && !matches!(
                p,
                Node::StatementsNode { .. }
                    | Node::BeginNode { .. }
                    | Node::BlockNode { .. }
                    | Node::LambdaNode { .. }
                    | Node::ClassNode { .. }
                    | Node::ModuleNode { .. }
                    | Node::SingletonClassNode { .. }
                    | Node::IfNode { .. }
                    | Node::UnlessNode { .. }
                    | Node::ElseNode { .. }
                    | Node::WhenNode { .. }
                    | Node::InNode { .. }
                    | Node::DefNode { .. }
            )
        {
            return None;
        }
    }
    let correction = super::correction::combine(
        file,
        vec![
            Correction {
                range: begin.start_offset()..begin.end_offset(),
                replacement: String::new(),
            },
            Correction {
                range: end.start_offset()..end.end_offset(),
                replacement: String::new(),
            },
        ],
    )?;
    super::correction::preserves_data_positions(file, cx, &correction).then_some(correction)
}

/// Prism stores a compound setter's receiver/index expressions directly under
/// the write. RuboCop gives those operands a synthetic send parent instead.
fn call_write_operand(parent: &Node<'_>, node: &Node<'_>) -> bool {
    let value = match parent {
        Node::CallAndWriteNode { .. } => parent.as_call_and_write_node().unwrap().value(),
        Node::CallOrWriteNode { .. } => parent.as_call_or_write_node().unwrap().value(),
        Node::CallOperatorWriteNode { .. } => parent.as_call_operator_write_node().unwrap().value(),
        Node::IndexAndWriteNode { .. } => parent.as_index_and_write_node().unwrap().value(),
        Node::IndexOrWriteNode { .. } => parent.as_index_or_write_node().unwrap().value(),
        Node::IndexOperatorWriteNode { .. } => {
            parent.as_index_operator_write_node().unwrap().value()
        }
        _ => return false,
    };
    node.location().end_offset() <= value.location().start_offset()
}

fn assignment(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::LocalVariableWriteNode { .. }
            | Node::LocalVariableAndWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::LocalVariableOperatorWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::InstanceVariableAndWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::InstanceVariableOperatorWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::ClassVariableOperatorWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::GlobalVariableOperatorWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantAndWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantOperatorWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::CallOperatorWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::MultiWriteNode { .. }
    )
}
fn low_precedence(n: &Node<'_>) -> bool {
    n.as_and_node()
        .is_some_and(|n| n.operator_loc().as_slice() == b"and")
        || n.as_or_node()
            .is_some_and(|n| n.operator_loc().as_slice() == b"or")
        || n.as_call_node()
            .is_some_and(|n| n.message_loc().is_some_and(|l| l.as_slice() == b"not"))
        || n.as_if_node()
            .is_some_and(|n| n.end_keyword_loc().is_none())
        || n.as_unless_node()
            .is_some_and(|n| n.end_keyword_loc().is_none())
        || n.as_while_node().is_some_and(|n| n.closing_loc().is_none())
        || n.as_until_node().is_some_and(|n| n.closing_loc().is_none())
}

#[cfg(test)]
mod tests {
    //! Pinned examples ported from RuboCop's style/redundant_begin_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    #[test]
    fn canonical_token_removals_and_single_line_values() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("begin\n  work\nend\n", "\n  work\n\n"),
            (
                "def f; begin; x; y; rescue; z end; end\n",
                "def f; ; x; y; rescue; z ; end\n",
            ),
            ("x = begin foo end\n", "x = foo \n"),
            ("x = begin foo if c end\n", "x = (foo if c) \n"),
            (
                "def f; return begin not a end; end\n",
                "def f; return (not a) ; end\n",
            ),
        ] {
            assert_correction(&RedundantBegin::default(), before, after);
        }
    }

    #[test]
    fn every_pinned_correction_is_canonical_deferred_or_individually_held() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_begin_pinned.json"
        ))
        .unwrap();
        let mut totals = [0; 3];
        for case in cases {
            let Some(after) = case["correction"].as_str() else {
                continue;
            };
            let before = case["source"].as_str().unwrap();
            let line = case["spec_line"].as_u64().unwrap();
            let cop = RedundantBegin {
                max: Some(case["max"].as_u64().unwrap() as usize),
            };
            if matches!(
                line,
                211 | 232
                    | 250
                    | 283
                    | 298
                    | 845
                    | 869
                    | 999
                    | 1037
                    | 1051
                    | 1065
                    | 1079
                    | 1097
                    | 1116
            ) {
                let found = correction_offenses(&cop, before);
                assert!(!found.is_empty());
                assert!(
                    found.iter().all(|o| o.correction.is_none()),
                    "spec {line}: {before}"
                );
                totals[0] += 1;
            } else if matches!(line, 886 | 923 | 961) {
                let first_pass = before.replacen("  begin\n", "  \n", 1).replacen(
                    "  end\nend\n",
                    "  \nend\n",
                    1,
                );
                assert_correction(&cop, before, &first_pass);
                assert_correction(&cop, &first_pass, after);
                assert!(correction_offenses(&cop, after).is_empty());
                totals[1] += 1;
            } else {
                assert_correction(&cop, before, after);
                assert!(correction_offenses(&cop, after).is_empty(), "{after}");
                totals[2] += 1;
            }
        }
        assert_eq!(totals, [15, 3, 20]);
    }

    #[test]
    fn required_rescue_scopes_and_synthetic_call_operands_are_not_redundant() {
        for source in [
            "def f; before; begin; work; rescue; nil; end; end",
            "def f; begin; work; rescue; nil; end; after; end",
            "proc { begin; work; rescue; nil; end }",
            "-> do; begin; work; ensure; cleanup; end; end",
            "begin; work; end while condition",
            "begin foo end.value ||= x",
            "obj[begin foo end] += x",
            "foo(begin work end)",
            "true && begin work end",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&RedundantBegin::default(), source);
            assert!(found.is_empty(), "{source}: {found:?}");
        }
    }

    #[test]
    fn rescue_else_keeps_empty_and_multi_expression_exemptions() {
        for header in ["def f", "def self.f", "proc do", "begin"] {
            for body in [
                "begin; end",
                "begin; 1; 2; end",
                "begin\nend",
                "begin\n1\n2\nend",
            ] {
                for tail in ["end", "ensure; cleanup; end"] {
                    let source = format!("{header}; 1; rescue; 0; else; {body}; {tail}");
                    let found: Vec<_> = crate::pipeline::lint_str(&source)
                        .into_iter()
                        .filter(|o| o.cop == "Style/RedundantBegin")
                        .collect();
                    assert!(found.is_empty(), "{source}: {found:?}");
                }
            }
        }
    }

    #[test]
    fn else_detection_distinguishes_rescue_from_conditional_and_case_owners() {
        for source in [
            "def f; 1; rescue; 0; else; begin; 1; end; end",
            "if c; 0; else; begin; end; end",
            "if c; 0; elsif d; 1; else; begin; 1; 2; end; end",
            "unless c; 0; else; begin; 1; 2; end; end",
            "case x; when 0; 0; else; begin; 1; 2; end; end",
            "case x; in 0; 0; else; begin; end; end",
            "def f; 1; rescue; 0; else; if c; 0; else; begin; 1; 2; end; end; end",
        ] {
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/RedundantBegin")
                .collect();
            assert_eq!(found.len(), 1, "{source}: {found:?}");
            assert_eq!(found[0].line, 1);
            assert_eq!(found[0].col, source.find("begin").unwrap() as u32 + 1);
            assert_eq!(found[0].message, "Redundant `begin` block detected.");
        }
    }

    #[test]
    fn uncertain_value_binding_headers_heredocs_and_line_moves_keep_findings() {
        for source in [
            "x = begin\n raise 'bad'\nend\n",
            "def f = begin\n work\nend\n",
            "begin\n work\nend if condition\n",
            "[begin work end]",
            "case; when begin work if condition end; fallback; end",
            "class C < begin superclass if condition end; end",
            "class << begin object if condition end; end",
            "if begin work if condition end; fallback; end",
            "begin work if condition end unless other",
            "begin # frozen_string_literal: true\n 'x'\nend\n",
            "begin #!/usr/bin/ruby\n 1\nend\n",
            "begin\n x = <<~TXT\n payload\n TXT\nend\n",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&RedundantBegin::default(), source);
            assert!(!found.is_empty(), "{source}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{source}: {found:?}"
            );
        }
    }

    #[test]
    fn bounded_lazy_main_source_work_for_many_wrappers() {
        let source = "begin; 1; end\n".repeat(400);
        SEARCH_NODES.set(0);
        REWRITE_BYTES.set(0);
        let found = crate::pipeline::lint_str(&source);
        assert!(SEARCH_NODES.get() <= 4 * 400);
        assert_eq!(REWRITE_BYTES.get(), 0);
        assert_eq!(
            found
                .iter()
                .filter(|o| o.cop == "Style/RedundantBegin")
                .count(),
            400
        );
        let found = crate::cops::testing::correction_offenses(&RedundantBegin::default(), &source);
        assert_eq!(found.len(), 400);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert!(SEARCH_NODES.get() <= 8 * 400);
        assert!(REWRITE_BYTES.get() <= source.len());
        let dense = format!(
            "{}# {}\n",
            "begin; 1; end; ".repeat(400),
            "comment".repeat(10000)
        );
        REWRITE_BYTES.set(0);
        let found = crate::cops::testing::correction_offenses(&RedundantBegin::default(), &dense);
        assert_eq!(found.len(), 400);
        assert!(found.iter().all(|o| o.correction.is_none()));
        assert_eq!(REWRITE_BYTES.get(), 0);
    }

    #[test]
    fn pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_begin_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 72);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let cop = RedundantBegin {
                max: Some(case["max"].as_u64().unwrap() as usize),
            };
            let found: Vec<Offense> = if cop.max == Some(120) {
                crate::pipeline::lint_str(source)
                    .into_iter()
                    .filter(|o| o.cop == cop.name())
                    .collect()
            } else {
                crate::cops::testing::correction_offenses(&cop, source)
            };
            let expected: Vec<Vec<u32>> =
                serde_json::from_value(case["positions"].clone()).unwrap();
            assert_eq!(
                found
                    .iter()
                    .map(|o| vec![o.line, o.col])
                    .collect::<Vec<_>>(),
                expected,
                "spec {}: {source}",
                case["spec_line"]
            );
            let messages: Vec<String> = serde_json::from_value(case["messages"].clone()).unwrap();
            assert_eq!(
                found.iter().map(|o| o.message.clone()).collect::<Vec<_>>(),
                messages
            );
        }
    }
}
