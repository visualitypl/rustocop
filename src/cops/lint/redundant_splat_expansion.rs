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
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_splat_node().is_some()
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
                .then(|| correction(node, &value, constructor, argument, in_array, cx))
                .flatten(),
        });
    }
}
/// RuboCop's `replacement_range_and_content`. Prism puts a rescue's
/// exceptions straight under the rescue, where RuboCop's AST has an
/// unbracketed array.
fn correction(
    node: &Node<'_>,
    value: &Node<'_>,
    constructor: bool,
    argument: bool,
    in_array: bool,
    cx: &NodeContext<'_, '_>,
) -> Option<crate::offense::Correction> {
    let source = |n: &Node<'_>| {
        std::str::from_utf8(n.location().as_slice())
            .ok()
            .map(str::to_owned)
    };
    let parent = cx.parent()?;
    let rescue = parent.as_rescue_node().is_some();
    let whole = node.location().start_offset()..node.location().end_offset();
    let (range, replacement) = if constructor {
        let range = if parent.as_array_node().is_some() {
            parent.location().start_offset()..parent.location().end_offset()
        } else {
            whole
        };
        (range, source(value)?.to_owned())
    } else if let Some(array) = value.as_array_node() {
        if parent.as_when_node().is_some() || argument || in_array || rescue {
            let elements = array
                .elements()
                .iter()
                .map(|e| source(&e))
                .collect::<Option<Vec<_>>>()?;
            let opening = array.opening_loc();
            let opening = opening.as_ref().map_or(b"".as_slice(), |o| o.as_slice());
            let text = if opening.starts_with(b"%w") {
                format!("'{}'", elements.join("', '"))
            } else if opening.starts_with(b"%W") {
                format!("\"{}\"", elements.join("\", \""))
            } else if opening.starts_with(b"%i") {
                format!(":{}", elements.join(", :"))
            } else if opening.starts_with(b"%I") {
                format!(":\"{}\"", elements.join("\", :\""))
            } else {
                elements.join(", ")
            };
            (whole, text)
        } else {
            let operator = node.as_splat_node()?.operator_loc();
            (
                operator.start_offset()..operator.end_offset(),
                String::new(),
            )
        }
    } else {
        let text = source(value)?;
        let unbracketed = rescue
            || parent
                .as_array_node()
                .is_some_and(|a| a.opening_loc().is_none());
        (
            whole,
            if unbracketed {
                format!("[{text}]")
            } else {
                text.to_owned()
            },
        )
    };
    Some(crate::offense::Correction { range, replacement })
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
    #[test]
    fn corrects_as_rubocop() {
        for (before, after) in [
            (
                "begin\n  foo\nrescue *[First, Second]\n  bar\nend\n",
                "begin\n  foo\nrescue First, Second\n  bar\nend\n",
            ),
            ("a = *\"a\"\n", "a = [\"a\"]\n"),
            ("[*%w[a b]]\n", "['a', 'b']\n"),
            ("[*Array.new(3)]\n", "Array.new(3)\n"),
        ] {
            crate::cops::testing::assert_correction(&super::RedundantSplatExpansion, before, after);
        }
    }
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
            let bytes: Vec<u8> = crate::fixture::value(&row["bytes"]);
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
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
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
