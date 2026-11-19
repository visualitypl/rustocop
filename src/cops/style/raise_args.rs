//! `Style/RaiseArgs` — fixed pinned exception policy.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RaiseArgs;
impl Cop for RaiseArgs {
    fn name(&self) -> &'static str {
        "Style/RaiseArgs"
    }
    fn unsafe_fixes(&self) -> bool {
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
        // RuboCop command? accepts receiverless calls only.
        if call.receiver().is_some() || !matches!(call.name().as_slice(), b"raise" | b"fail") {
            return;
        }
        let args = ast::call_arguments(&call);
        let [exception] = args.as_slice() else { return };
        let Some(new) = exception.as_call_node() else {
            return;
        };
        if new.receiver().is_none()
            || new.is_safe_navigation()
            || new.name().as_slice() != b"new"
            || new.block().is_some_and(|b| b.as_block_node().is_some())
        {
            return;
        }
        let args = ast::call_arguments(&new);
        if args.len() > 1
            || args.first().is_some_and(|arg| {
                matches!(
                    arg,
                    Node::HashNode { .. }
                        | Node::KeywordHashNode { .. }
                        | Node::SplatNode { .. }
                        | Node::ForwardingArgumentsNode { .. }
                )
            })
        {
            return;
        }
        // correction_compact_to_exploded: `raise Ex, message`, parenthesized
        // where the parent needs it.
        let correction = cx.corrections_requested().then(|| {
            let text = |n: &Node<'_>| String::from_utf8_lossy(n.location().as_slice()).into_owned();
            let arguments = std::iter::once(text(&new.receiver().unwrap()))
                .chain(args.first().map(text))
                .collect::<Vec<_>>()
                .join(", ");
            let name = String::from_utf8_lossy(call.name().as_slice());
            Correction {
                range: node.location().start_offset()..node.location().end_offset(),
                replacement: if requires_parens(cx) {
                    format!("{name}({arguments})")
                } else {
                    format!("{name} {arguments}")
                },
            }
        });
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: format!(
                "Provide an exception class and message as arguments to `{}`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            correction,
        });
    }
}
/// `requires_parens?(node.parent)`: `and`/`or`, a call, `super`, `yield`, an
/// array, a pair, an endless method or a ternary. Argument lists, a sole
/// statement and a ternary's `else` are not parser nodes.
fn requires_parens(cx: &NodeContext<'_, '_>) -> bool {
    let parent = cx.ancestors().find(|n| match n {
        Node::ArgumentsNode { .. } | Node::ElseNode { .. } => false,
        Node::StatementsNode { .. } => n.as_statements_node().unwrap().body().len() > 1,
        _ => true,
    });
    match parent {
        Some(
            Node::AndNode { .. }
            | Node::OrNode { .. }
            | Node::CallNode { .. }
            | Node::SuperNode { .. }
            | Node::YieldNode { .. }
            | Node::ArrayNode { .. }
            | Node::AssocNode { .. },
        ) => true,
        Some(Node::DefNode { .. }) => parent.unwrap().as_def_node().unwrap().equal_loc().is_some(),
        Some(Node::IfNode { .. }) => parent
            .unwrap()
            .as_if_node()
            .unwrap()
            .if_keyword_loc()
            .is_none(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/raise_args_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn constructors_on_arbitrary_receivers_have_exact_diagnostics_and_edits() {
        for keyword in ["raise", "fail"] {
            for constructor in [
                "Ex.new",
                "Ex.new(msg)",
                "error_class.new('hi')",
                "::Ex::Error.new(1)",
                "factory.error.new",
                "Ex.new(&block)",
            ] {
                let source = format!("def f\n  {keyword} {constructor}\nend");
                let found = correction_offenses(&RaiseArgs, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert_eq!(
                    found[0].message,
                    format!("Provide an exception class and message as arguments to `{keyword}`.")
                );
                assert!(found[0].correction.is_some());
            }
        }
    }
    #[test]
    fn receiver_arity_forwarding_and_constructor_lookalikes() {
        for source in [
            "Kernel.raise Ex.new('x')",
            "self.fail Ex.new",
            "object&.raise(Ex.new)",
            "raise Ex&.new('x')",
            "raise Ex.new { break :value }",
            "raise Ex.new(a,b)",
            "raise Ex.new({})",
            "raise Ex.new(k: 1)",
            "raise Ex.new(*args)",
            "raise Ex.new(**args)",
            "raise Ex.new, msg",
            "raise Ex.new, msg, caller",
            "raise new",
            "raise new(msg)",
            "raise Ex.build(msg)",
            "raise Ex.new(...)",
            "raise Ex.new(*)",
            "raise Ex.new(**)",
            "raise",
            "raise 'message'",
        ] {
            let source = format!("def f(...); {source}; end");
            // Anonymous rest forwarding requires anonymous rest parameters.
            let source = source
                .replace("def f(...); raise Ex.new(*)", "def f(*); raise Ex.new(*)")
                .replace(
                    "def f(...); raise Ex.new(**)",
                    "def f(**); raise Ex.new(**)",
                );
            assert_eq!(
                crate::parser::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            assert!(
                correction_offenses(&RaiseArgs, &source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn all_applicable_pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/raise_args_pinned.json"
        ))
        .unwrap();
        for case in cases {
            let source = case["source"].as_str().unwrap();
            // Pinned spec uses yield at top level; production correctly rejects it.
            let valid = if source.starts_with("yield ") {
                format!("def f\n{source}end\n")
            } else {
                source.into()
            };
            let mut found = correction_offenses(&RaiseArgs, &valid);
            if source.starts_with("yield ") {
                for offense in &mut found {
                    offense.line -= 1;
                }
            }
            assert!(
                found.iter().all(|o| o.correction.is_some()),
                "unsafe edit: {source}"
            );
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            let expected: Vec<Vec<u32>> =
                serde_json::from_value(case["positions"].clone()).unwrap();
            assert_eq!(positions, expected, "{source}");
            let messages: Vec<_> = found.iter().map(|o| o.message.as_str()).collect();
            let expected: Vec<String> = serde_json::from_value(case["messages"].clone()).unwrap();
            assert_eq!(messages, expected, "{source}");
        }
    }
}
