//! `Style/ImplicitRuntimeError` — fixed pinned exception policy.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ImplicitRuntimeError;
impl Cop for ImplicitRuntimeError {
    fn name(&self) -> &'static str {
        "Style/ImplicitRuntimeError"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.receiver().is_some() || !matches!(call.name().as_slice(), b"raise" | b"fail") {
            return;
        }
        let args = ast::call_arguments(&call);
        let [message] = args.as_slice() else { return };
        if !matches!(
            message,
            Node::StringNode { .. }
                | Node::InterpolatedStringNode { .. }
                | Node::SourceFileNode { .. }
        ) {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(), severity: self.severity(), line, col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: format!("Use `{}` with an explicit exception class and message, rather than just a message.", String::from_utf8_lossy(call.name().as_slice())),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/implicit_runtime_error_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn strings_interpolation_positions_and_detection_only() {
        for keyword in ["raise", "fail"] {
            for message in ["'x'", "\"#{value}\"", "%q(x)", "\"a\" \\\n  \"b\""] {
                let source = format!("def f\n  {keyword}({message})\nend");
                let found = correction_offenses(&ImplicitRuntimeError, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert_eq!(
                    found[0].message,
                    format!(
                        "Use `{keyword}` with an explicit exception class and message, rather than just a message."
                    )
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn source_file_messages_positions_and_detection_only() {
        let found = correction_offenses(&ImplicitRuntimeError, "raise __FILE__\nfail __FILE__\n");
        assert_eq!(found.len(), 2);
        for (offense, (line, keyword)) in found.iter().zip([(1, "raise"), (2, "fail")]) {
            assert_eq!(
                (offense.line, offense.col, offense.end_line),
                (line, 1, line)
            );
            assert_eq!(
                offense.message,
                format!(
                    "Use `{keyword}` with an explicit exception class and message, rather than just a message."
                )
            );
            assert!(offense.correction.is_none());
        }
    }
    #[test]
    fn no_offense_receiver_arity_variable_and_literal_lookalikes() {
        for source in [
            "Kernel.raise 'x'",
            "::Kernel.fail 'x'",
            "self.raise('x')",
            "obj&.fail('x')",
            "raise RuntimeError, 'x'",
            "raise 'x', cause: e",
            "raise msg",
            "raise :message",
            "raise __LINE__",
            "fail __ENCODING__",
            "Kernel.raise __FILE__",
            "fail RuntimeError, __FILE__",
            "raise `command`",
            "raise",
            "fail",
            "raise = 'x'; raise",
            "# raise 'x'\n'fail'",
            "raise 'x', &b",
        ] {
            assert!(
                correction_offenses(&ImplicitRuntimeError, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn all_applicable_pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/implicit_runtime_error_pinned.json"
        ))
        .unwrap();
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found = correction_offenses(&ImplicitRuntimeError, source);
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
