//! `Style/RedundantException` — fixed pinned exception policy.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantException;
impl Cop for RedundantException {
    fn name(&self) -> &'static str {
        "Style/RedundantException"
    }
    fn safe_fixes(&self) -> bool {
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
        if call.receiver().is_some() || !matches!(call.name().as_slice(), b"raise" | b"fail") {
            return;
        }
        let args = ast::call_arguments(&call);
        let (message, compact) = match args.as_slice() {
            [class, message] if ast::is_global_const(class, &["RuntimeError"]) => (message, false),
            [exception] => {
                let Some(new) = exception.as_call_node() else {
                    return;
                };
                if new.name().as_slice() != b"new"
                    || new.is_safe_navigation()
                    || new.block().is_some_and(|b| b.as_block_node().is_some())
                    || !new
                        .receiver()
                        .is_some_and(|r| ast::is_global_const(&r, &["RuntimeError"]))
                {
                    return;
                }
                let args = ast::call_arguments(&new);
                if args.len() != 1 {
                    return;
                }
                // Report while the temporary argument list is alive.
                return report(self, node, &args[0], true, cx, file, out);
            }
            _ => return,
        };
        report(self, node, message, compact, cx, file, out);
    }
}
fn report(
    cop: &RedundantException,
    node: &Node<'_>,
    message: &Node<'_>,
    compact: bool,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    if message.as_nil_node().is_some() {
        return;
    }
    let (line, col) = file.line_col(node.location().start_offset());
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        line,
        col,
        end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
        message: if compact {
            "Redundant `RuntimeError.new` call can be replaced with just the message."
        } else {
            "Redundant `RuntimeError` argument can be removed."
        }
        .into(),
        correction: cx
            .corrections_requested()
            .then(|| correction(node, message, compact))
            .flatten(),
    });
}
/// RuboCop's corrector.
fn correction(
    node: &Node<'_>,
    message: &Node<'_>,
    compact: bool,
) -> Option<crate::offense::Correction> {
    let call = node.as_call_node()?;
    let source = message_source(message)?;
    let args = ast::call_arguments(&call);
    if compact {
        let new = args.first()?.location();
        return Some(crate::offense::Correction {
            range: new.start_offset()..new.end_offset(),
            replacement: source,
        });
    }
    let command = std::str::from_utf8(call.name().as_slice()).ok()?;
    let parenthesized = call.opening_loc().is_some_and(|o| o.as_slice() == b"(");
    let arg = if parenthesized {
        format!("({source})")
    } else {
        format!(" {source}")
    };
    let loc = node.location();
    Some(crate::offense::Correction {
        range: loc.start_offset()..ast::source_end(node),
        replacement: format!("{command}{arg}"),
    })
}

/// RuboCop's `message_source`.
fn message_source(message: &Node<'_>) -> Option<String> {
    let source = std::str::from_utf8(message.location().as_slice()).ok()?;
    let string = matches!(
        message,
        Node::StringNode { .. }
            | Node::InterpolatedStringNode { .. }
            | Node::XStringNode { .. }
            | Node::InterpolatedXStringNode { .. }
            | Node::SourceFileNode { .. }
    );
    Some(if string {
        source.to_owned()
    } else if require_parentheses(message) {
        format!("({source}).to_s")
    } else {
        format!("{source}.to_s")
    })
}

fn require_parentheses(message: &Node<'_>) -> bool {
    if let Some(call) = message.as_call_node()
        && !call.block().is_some_and(|b| b.as_block_node().is_some())
    {
        let name = call.name().as_slice();
        return name != b"[]"
            && (ast::is_operator_method(name)
                || (!ast::call_arguments(&call).is_empty()
                    && !call.opening_loc().is_some_and(|o| o.as_slice() == b"(")));
    }
    !(matches!(
        message,
        Node::IntegerNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::SymbolNode { .. }
            | Node::SourceLineNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
            | Node::ConstantReadNode { .. }
            | Node::ConstantPathNode { .. }
            | Node::SelfNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::ArrayNode { .. }
    ) || ast::is_variable(message))
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/redundant_exception_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;
    #[test]
    fn canonical_string_corrections() {
        for keyword in ["raise", "fail"] {
            for class in ["RuntimeError", "::RuntimeError"] {
                for (before, after) in [
                    (
                        format!("{keyword} {class}, \"message\""),
                        format!("{keyword} \"message\""),
                    ),
                    (
                        format!("{keyword}({class}, \"message\")"),
                        format!("{keyword}(\"message\")"),
                    ),
                    (
                        format!("{keyword} {class}.new \"message\""),
                        format!("{keyword} \"message\""),
                    ),
                    (
                        format!("{keyword} {class}.new(\"message\")"),
                        format!("{keyword} \"message\""),
                    ),
                ] {
                    crate::cops::testing::assert_correction(&RedundantException, &before, &after);
                }
            }
        }
    }
    #[test]
    fn nil_receiver_namespace_backtrace_and_arity_lookalikes() {
        for source in [
            "raise RuntimeError, nil",
            "raise RuntimeError.new(nil)",
            "raise RuntimeError",
            "raise RuntimeError.new",
            "raise RuntimeError, 'x', caller",
            "raise RuntimeError.new('x'), 'other'",
            "raise Foo::RuntimeError, 'x'",
            "raise OtherError, 'x'",
            "Kernel.raise RuntimeError, 'x'",
            "self.fail RuntimeError.new('x')",
            "raise RuntimeError&.new('x')",
            "raise RuntimeError.new('x') { break :result }",
        ] {
            assert!(
                correction_offenses(&RedundantException, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn canonical_edits_preserve_literal_bytes_and_enclosing_expressions() {
        for (before, after) in [
            ("raise RuntimeError, %q(a\\nb)", "raise %q(a\\nb)"),
            (
                "raise(RuntimeError.new(\"\\x82\\n\"))",
                "raise(\"\\x82\\n\")",
            ),
            (
                "x ? raise(RuntimeError, 'bad') : good",
                "x ? raise('bad') : good",
            ),
            (
                "def f = raise(RuntimeError.new('bad'))",
                "def f = raise('bad')",
            ),
            (
                "-> { fail(::RuntimeError.new('bad')) }",
                "-> { fail('bad') }",
            ),
            ("raise RuntimeError, \"a\nb\"", "raise \"a\nb\""),
            (
                "raise RuntimeError.new(\"x\") # keep\r\np __LINE__\r\n__END__\r\npayload",
                "raise \"x\" # keep\r\np __LINE__\r\n__END__\r\npayload",
            ),
        ] {
            crate::cops::testing::assert_correction(&RedundantException, before, after);
        }
    }
    #[test]
    fn all_applicable_pinned_diagnostics() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_exception_pinned.json"
        ))
        .unwrap();
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let found = correction_offenses(&RedundantException, source);
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
