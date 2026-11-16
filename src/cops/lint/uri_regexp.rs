//! `Lint/UriRegexp` — obsolete global URI.regexp calls, at fixed Ruby 3.4.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct UriRegexp;
impl Cop for UriRegexp {
    fn name(&self) -> &'static str {
        "Lint/UriRegexp"
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
        // Prism fuses the getter send into compound writes; RuboCop visits it.
        macro_rules! getter {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                if call.is_safe_navigation() {
                    return;
                }
                let Some(selector) = call.message_loc() else {
                    return;
                };
                let end = selector.end_offset();
                (call.read_name(), call.receiver(), selector, end, None)
            }};
        }
        let (name, receiver, selector, end, first_argument) = match node {
            Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
            Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
            Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
            _ => {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if call.is_safe_navigation() || call.name().as_slice() != b"regexp" {
                    return;
                }
                let Some(selector) = call.message_loc() else {
                    return;
                };
                (
                    call.name(),
                    call.receiver(),
                    selector,
                    ast::source_end(node),
                    ast::call_arguments(&call).into_iter().next(),
                )
            }
        };
        if name.as_slice() != b"regexp" {
            return;
        }
        let Some(receiver) = receiver.filter(|r| ast::is_global_const(r, &["URI"])) else {
            return;
        };
        // Upstream copies RAW receiver/first-argument source; it neither
        // normalizes spelling nor retains later arguments in its suggestion.
        // A literal block is not part of RuboCop's send node source.
        let current = String::from_utf8_lossy(&file.bytes[node.location().start_offset()..end]);
        let argument = first_argument
            .map(|arg| format!("({})", String::from_utf8_lossy(arg.location().as_slice())))
            .unwrap_or_default();
        let preferred = format!(
            "{}::RFC2396_PARSER.make_regexp{argument}",
            String::from_utf8_lossy(receiver.location().as_slice())
        );
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(selector.end_offset() - 1),
            message: format!(
                "`{current}` is obsolete and should not be used. Instead, use `{preferred}`."
            ),
            // RuboCop replaces the call. A replacement is UTF-8 text: other
            // bytes cannot be carried over.
            correction: (cx.corrections_requested()
                && std::str::from_utf8(node.location().as_slice()).is_ok())
            .then(|| crate::offense::Correction {
                range: node.location().start_offset()..ast::source_end(node),
                replacement: preferred.clone(),
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's `spec/rubocop/cop/lint/uri_regexp_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UriRegexp")
            .collect()
    }

    #[test]
    fn pinned_examples_at_fixed_ruby34_have_exact_selector_and_message() {
        for receiver in ["URI", "::URI"] {
            for argument in [
                "",
                "('http://example.com')",
                "(['http', 'https'])",
                "(%w[http https])",
                "(%i[http https])",
            ] {
                let call = format!("{receiver}.regexp{argument}");
                let source = format!("# before\n  {call}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (2, receiver.len() as u32 + 4, 2)
                );
                assert_eq!(
                    found[0].message,
                    format!(
                        "`{call}` is obsolete and should not be used. Instead, use `{receiver}::RFC2396_PARSER.make_regexp{argument}`."
                    )
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn closed_stdlib_method_reference_has_a_real_canonical_edit() {
        for receiver in ["URI", "::URI"] {
            for suffix in ["", "()"] {
                let before = format!("require 'uri'\n[defined?({receiver}.regexp{suffix})]\n");
                let after =
                    format!("require 'uri'\n[defined?({receiver}::RFC2396_PARSER.make_regexp)]\n");
                crate::cops::testing::assert_correction(&super::UriRegexp, &before, &after);
            }
        }
    }

    #[test]
    fn unrelated_receivers_methods_and_safe_navigation_are_clean() {
        for source in [
            "regexp('http://example.com')",
            "m.regexp('http://example.com')",
            "Other::URI.regexp",
            "(URI).regexp",
            "URI&.regexp",
            "uri.regexp",
            "URI.regexp = value",
            "def URI.regexp; end",
            "URI::RFC2396_PARSER.make_regexp",
            "# URI.regexp\n'URI.regexp'\n:regexp",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
