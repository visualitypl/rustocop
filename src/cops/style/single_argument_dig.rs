//! `Style/SingleArgumentDig` — prefer indexing for ordinary one-key dig calls.
use super::dig_chain::literal_block;
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SingleArgumentDig {
    dig_chain_enabled: bool,
}
impl Default for SingleArgumentDig {
    fn default() -> Self {
        Self {
            dig_chain_enabled: true,
        }
    }
}
impl Cop for SingleArgumentDig {
    fn name(&self) -> &'static str {
        "Style/SingleArgumentDig"
    }
    fn with_dig_chain(&self, enabled: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            dig_chain_enabled: enabled,
        }))
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
        if call.name().as_slice() != b"dig" || call.is_safe_navigation() {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let args = ast::call_arguments(&call);
        let [argument] = args.as_slice() else { return };
        if matches!(
            argument,
            Node::SplatNode { .. }
                | Node::ForwardingArgumentsNode { .. }
                | Node::BlockArgumentNode { .. }
                | Node::HashNode { .. }
                | Node::KeywordHashNode { .. }
        ) {
            return;
        }
        // ArgumentsNode is transparent in RuboCop's AST. Literal blocks wrap
        // sends there, so a send with its own block has no send parent.
        let parent = cx.ancestors().find(|n| n.as_arguments_node().is_none());
        if self.dig_chain_enabled {
            let facts = file.dig_calls(cx.ancestors().next_back().unwrap_or(node));
            if facts.dig_expression(&receiver)
                || (!literal_block(&call)
                    && parent
                        .and_then(Node::as_call_node)
                        .is_some_and(|parent| facts.is_dig(&parent)))
            {
                return;
            }
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let original = single_line(&file.bytes[start..end]);
        let receiver = single_line(receiver.location().as_slice());
        let argument = single_line(argument.location().as_slice());
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{receiver}[{argument}]` instead of `{original}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `single_argument_dig_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SingleArgumentDig")
            .collect()
    }

    #[test]
    fn all_pinned_examples_match_exact_positions_messages_and_emit_no_corrections() {
        use crate::cops::{Cop, testing::correction_offenses};
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/single_argument_dig_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 15);
        for case in cases.as_array().unwrap() {
            let enabled = case["toml"]["Style/DigChain"]["enabled"]
                .as_bool()
                .unwrap_or(true);
            let cop = super::SingleArgumentDig::default()
                .with_dig_chain(enabled)
                .unwrap();
            let found = correction_offenses(cop.as_ref(), case["source"].as_str().unwrap());
            let mut actual: Vec<_> = found
                .iter()
                .map(|o| (o.line, o.col, o.message.as_str()))
                .collect();
            let mut expected: Vec<_> = case["positions"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["messages"].as_array().unwrap())
                .map(|(p, m)| {
                    (
                        p[0].as_u64().unwrap() as u32,
                        p[1].as_u64().unwrap() as u32,
                        m.as_str().unwrap(),
                    )
                })
                .collect();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected, "{case}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn reports_literal_and_dynamic_receivers_with_exact_message() {
        for (call, replacement) in [
            ("{ key: 'value' }.dig(:key)", "{ key: 'value' }[:key]"),
            ("data.dig(var)", "data[var]"),
            ("[1, 2, 3].dig(0)", "[1, 2, 3][0]"),
        ] {
            let found = offenses(&format!("def f\n  {call}\nend\n"));
            assert_eq!(found.len(), 1, "{call}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("Use `{replacement}` instead of `{call}`.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn ignores_safe_navigation_forwarding_hashes_and_wrong_arity() {
        for source in [
            "data&.dig(:key)",
            "data.dig(a, b)",
            "data.dig",
            "dig(:key)",
            "data.dig(*keys)",
            "data.dig(&block)",
            "data.dig(key: 1)",
            "data.dig({key: 1})",
            "data.dig(key, &block)",
            "def f(...); data.dig(...); end",
            "def f(&); data.dig(&); end",
            "def f(*); data.dig(*); end",
            "def f(**); data.dig(**); end",
            "data.digger(:key)",
            "def dig(key); key; end",
            "dig = :key; dig",
            "# data.dig(:key)\n'data.dig(:key)'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn wide_argument_parents_do_not_rescan_their_arguments_for_every_child() {
        use crate::cops::{ast::CALL_ARGUMENT_VISITS, testing::correction_offenses};
        let source = format!(
            "x.dig({})",
            std::iter::repeat_n("x.dig(:key)", 1000)
                .collect::<Vec<_>>()
                .join(", ")
        );
        CALL_ARGUMENT_VISITS.set(0);
        assert!(correction_offenses(&super::SingleArgumentDig::default(), &source).is_empty());
        assert!(
            CALL_ARGUMENT_VISITS.get() < 10_000,
            "{} argument visits",
            CALL_ARGUMENT_VISITS.get()
        );
    }

    #[test]
    fn delegates_direct_chains_but_not_interrupted_chains() {
        for source in ["data.dig(a).dig(b)", "data.dig(a).dig(b) { false }"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("x.dig(y.dig(:a)) { :v }");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "Use `x[y.dig(:a)]` instead of `x.dig(y.dig(:a))`."
        );
        let found = offenses("data.dig(var1)[0].dig(var2)");
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|o| (o.line, o.col) == (1, 1)));
        assert!(
            found
                .iter()
                .any(|o| o.message == "Use `data[var1]` instead of `data.dig(var1)`.")
        );
        assert!(found.iter().any(|o| o.message
            == "Use `data.dig(var1)[0][var2]` instead of `data.dig(var1)[0].dig(var2)`."));
    }
}
