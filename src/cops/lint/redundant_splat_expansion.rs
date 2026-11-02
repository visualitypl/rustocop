//! `Lint/RedundantSplatExpansion` — literal splats with fixed percent-array allowance.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantSplatExpansion;
impl Cop for RedundantSplatExpansion {
    fn name(&self) -> &'static str {
        "Lint/RedundantSplatExpansion"
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
        let Some(splat) = node.as_splat_node() else {
            return;
        };
        let Some(value) = splat.expression() else {
            return;
        };
        let array = value.as_array_node();
        let literal = matches!(
            value,
            Node::StringNode { .. }
                | Node::InterpolatedStringNode { .. }
                | Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::SourceLineNode { .. }
                | Node::SourceFileNode { .. }
        );
        if array.as_ref().is_some_and(|a| a.elements().is_empty()) {
            return;
        }
        let constructor = value.as_call_node().is_some_and(|call| {
            call.name().as_slice() == b"new"
                && !call.is_safe_navigation()
                && !call
                    .block()
                    .and_then(|b| b.as_block_node())
                    .and_then(|b| b.parameters())
                    .is_some_and(|p| {
                        matches!(
                            p,
                            Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. }
                        )
                    })
                && call
                    .receiver()
                    .is_some_and(|r| crate::cops::ast::is_global_const(&r, &["Array"]))
        });
        if !literal && array.is_none() && !constructor {
            return;
        }
        if constructor
            && !constructor_context(
                cx,
                value
                    .as_call_node()
                    .unwrap()
                    .block()
                    .is_some_and(|b| b.as_block_node().is_some()),
            )
        {
            return;
        }
        let argument = cx.parent().is_some_and(|p| p.as_arguments_node().is_some())
            && cx.ancestors().nth(1).is_some_and(|p| {
                matches!(
                    p,
                    Node::CallNode { .. }
                        | Node::IndexAndWriteNode { .. }
                        | Node::IndexOrWriteNode { .. }
                        | Node::IndexOperatorWriteNode { .. }
                        | Node::IndexTargetNode { .. }
                )
            });
        let in_array = cx
            .parent()
            .and_then(|p| p.as_array_node())
            .is_some_and(|a| a.opening_loc().is_some());
        if argument
            && array.as_ref().is_some_and(|a| {
                a.opening_loc()
                    .is_some_and(|l| l.as_slice().starts_with(b"%"))
            })
        {
            return;
        }
        let message = if array.is_some() && (argument || in_array) {
            "Pass array contents as separate arguments."
        } else {
            "Replace splat expansion with comma separated values."
        };
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: message.into(),
            correction: cx
                .corrections_requested()
                .then(|| inert_rescue_correction(node, &value, cx, file))
                .flatten(),
        });
    }
}
// A live literal flattening can change dispatch (to_a), allocation, exception
// filter evaluation, keyword arguments and value/control flow. Only a first
// rescue whose protected statements cannot raise or allocate is eligible.
// Trace/source/code-object/heap/position/heredoc barriers remain shared. The
// exact canonical simple-filter spelling also excludes comments, newlines,
// encodings needing replacement conversion, nested splats and grammar changes.
fn inert_rescue_correction(
    node: &Node<'_>,
    value: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    cx.parent()?.as_rescue_node()?;
    let begin = cx.ancestors().nth(1)?.as_begin_node()?;
    if begin.statements().is_some_and(|s| {
        s.body().iter().any(|n| {
            !matches!(
                n,
                Node::NilNode { .. } | Node::TrueNode { .. } | Node::FalseNode { .. }
            )
        })
    }) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.class_method_bindings(root).observes_sources() {
        return None;
    }
    let array = value.as_array_node()?;
    if array.opening_loc()?.as_slice() != b"[" || array.closing_loc()?.as_slice() != b"]" {
        return None;
    }
    let mut elements = Vec::new();
    for element in array.elements().iter() {
        if crate::cops::ast::const_name(&element).is_none()
            && !element.as_call_node().is_some_and(|c| {
                c.receiver().is_none()
                    && c.arguments().is_none()
                    && c.block().is_none()
                    && c.message_loc()
                        .is_some_and(|l| l.as_slice() == c.name().as_slice())
            })
        {
            return None;
        }
        let source = element.location().as_slice();
        if !source.is_ascii() || source.iter().any(|b| matches!(b, b'#' | b'\r' | b'\n')) {
            return None;
        }
        elements.push(std::str::from_utf8(source).ok()?);
    }
    if elements.is_empty() {
        return None;
    }
    let replacement = elements.join(", ");
    if node.location().as_slice() != format!("*[{replacement}]").as_bytes() {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset()..node.location().end_offset(),
        replacement,
    })
}
// Single-statement/argument lists are transparent in the reference AST, but
// multi-statement sequences, groups and literal blocks are real begin/block
// parents. No root rescans or type flow.
fn constructor_context(cx: &NodeContext<'_, '_>, literal_block: bool) -> bool {
    let mut ancestors = cx.ancestors().filter(|p| {
        !p.as_statements_node().is_some_and(|s| s.body().len() <= 1)
            && p.as_arguments_node().is_none()
    });
    let Some(parent) = ancestors.next() else {
        return true;
    };
    if !literal_block
        && parent
            .as_array_node()
            .is_some_and(|a| a.elements().len() > 1)
    {
        return false;
    }
    // Rescue lists have a synthetic array parent; compound index writes
    // have a synthetic send parent beneath their assignment node.
    if matches!(
        parent,
        Node::RescueNode { .. }
            | Node::IndexAndWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::IndexTargetNode { .. }
    ) {
        return false;
    }
    let block = match parent {
        Node::CallNode { .. } => parent.as_call_node().unwrap().block(),
        Node::SuperNode { .. } => parent.as_super_node().unwrap().block(),
        _ => None,
    };
    if block.is_some_and(|b| b.as_block_node().is_some()) {
        return false;
    }
    let grandparent = ancestors.next();
    grandparent.is_none_or(|p| {
        matches!(
            p,
            Node::ProgramNode { .. }
                | Node::LocalVariableWriteNode { .. }
                | Node::InstanceVariableWriteNode { .. }
                | Node::ClassVariableWriteNode { .. }
                | Node::GlobalVariableWriteNode { .. }
                | Node::ConstantWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
        )
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_splat_expansion_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == RedundantSplatExpansion.name())
            .collect()
    }
    #[test]
    fn all_pinned_specs_sources_and_byte_controls_match() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_splat_expansion.json"
        ))
        .unwrap();
        let registry = crate::cops::Registry::new();
        let cop = registry
            .cops()
            .iter()
            .find(|c| c.name() == RedundantSplatExpansion.name())
            .unwrap();
        for row in data["rows"].as_array().unwrap() {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            let file = SourceFile::new("example.rb".into(), bytes);
            let parsed = ruby_prism::parse(&file.bytes);
            if row.get("parser_rejection").is_some() || row.get("native_scope_rejection").is_some()
            {
                assert!(parsed.errors().count() > 0, "{row}");
                continue;
            }
            assert_eq!(parsed.errors().count(), 0, "{row}");
            let mut actual = Vec::new();
            assert!(crate::cops::run_cops(&[cop.as_ref()], &parsed, &file, &mut actual).is_empty());
            actual.sort_by_key(|o| (o.line, o.col));
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            assert_eq!(
                actual
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
        }
    }
    #[test]
    fn canonical_inert_rescue_edits_and_uncertain_holds() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_splat_expansion.json"
        ))
        .unwrap();
        for row in data["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r.get("project_after").is_some())
        {
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            let expected: Vec<u8> = serde_json::from_value(row["project_after"].clone()).unwrap();
            let file = SourceFile::new("example.rb".into(), bytes.clone());
            let parsed = ruby_prism::parse(&file.bytes);
            let mut found = Vec::new();
            assert!(
                crate::cops::run_cops(&[&RedundantSplatExpansion], &parsed, &file, &mut found)
                    .is_empty()
            );
            assert_eq!(
                crate::corrections::apply(&bytes, &found).unwrap().bytes,
                expected,
                "{row}"
            );
        }
        for source in [
            "begin; work; rescue *[First, Second]; end",
            "begin; []; rescue *[First, Second]; end",
            "begin; nil; rescue Error; work; rescue *[First, Second]; end",
            "begin; nil; rescue *[First, # keep\nSecond]; end",
            "begin; nil; rescue *[First, Second]; end\nTracePoint.new(:line) {}",
            "begin; nil; rescue *[First, Second]; end\nGC.stat",
        ] {
            let found = crate::cops::testing::correction_offenses(&RedundantSplatExpansion, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn flags_literals_at_exact_splat_locations() {
        for (source, line, col, message) in [
            (
                "a = *[1, 2, 3]",
                1,
                5,
                "Replace splat expansion with comma separated values.",
            ),
            (
                "array.push(*[1, 2, 3])",
                1,
                12,
                "Pass array contents as separate arguments.",
            ),
            (
                "begin\n foo\nrescue *[First, Second]\n bar\nend",
                3,
                8,
                "Replace splat expansion with comma separated values.",
            ),
            (
                "[1, 2, *%w(c d e)]",
                1,
                8,
                "Pass array contents as separate arguments.",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].message.as_str()),
                (line, col, message)
            );
        }
    }
    #[test]
    fn constructor_splats_preserve_multi_statement_boundaries() {
        for expression in ["f(*Array.new(2))", "[*Array.new(2)]"] {
            for source in [
                format!("nil\n{expression}\n"),
                format!("{expression}\nnil\n"),
                format!("nil\n{expression}\ntrue\n"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
            // A single root expression and an assignment grandparent still qualify.
            for source in [
                format!("{expression}\n"),
                format!("nil\na = {expression}\n"),
            ] {
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].message.as_str()),
                    if source.starts_with("nil") {
                        (
                            2,
                            if expression.starts_with('f') { 7 } else { 6 },
                            "Replace splat expansion with comma separated values.",
                        )
                    } else {
                        (
                            1,
                            if expression.starts_with('f') { 3 } else { 2 },
                            "Replace splat expansion with comma separated values.",
                        )
                    }
                );
            }
        }
    }
    #[test]
    fn constructor_splats_respect_literal_super_blocks() {
        for block in ["{}", "do\nend", "{ _1 }", "do\n _1\nend", "{ it }"] {
            let source = format!("def f\n  result = super(*Array.new(2)) {block}\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for source in [
            "def f\n  result = super(*Array.new(2))\nend\n",
            "def f\n  result = super(*Array.new(2), &block)\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 18));
            assert_eq!(
                found[0].message,
                "Replace splat expansion with comma separated values."
            );
        }
        // The constructor-only guard must not suppress ordinary literal findings.
        let found = offenses("def f\n  result = super(*[1]) {}\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 18));
    }
    #[test]
    fn allows_nonliteral_empty_grouped_and_default_percent_arguments() {
        for source in [
            "a = *b",
            "a = *1..10",
            "array.push(*foo)",
            "foo(*%w[a b])",
            "obj&.foo(*%I[a #{b}])",
            "when_value = [*[]]",
            "foo(*[])",
            "foo(*([1]))",
            "foo(*:a)",
            "foo(*nil)",
            "[1, *[2].map(&:to_s)]",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
