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
            correction: cx
                .corrections_requested()
                .then(|| closed_method_reference(node, cx, file, &preferred))
                .flatten(),
        });
    }
}

/// Only a closed top-level `require 'uri'; [defined?(URI.regexp)]` (also
/// rooted/empty-parens) establishes BOTH native methods without invoking them.
/// Ruby 3.4's ordinary call warns twice under -w; the replacement warns zero
/// times. Never suppress those findings or repair ordinary calls. `defined?`
/// preserves the consumed "method" string, with no argument/block evaluation,
/// Regexp construction, user binding, namespace mutation or delayed execution.
/// Requiring the actual stdlib excludes an unresolved/shadowed URI or parser;
/// arbitrary project code/loads/mutations are not admitted. The same constant-time
/// parent/sibling proof excludes source/runtime observers; shared main-parse
/// guards remain authoritative. Exact call spelling avoids discarded comments.
fn closed_method_reference(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    preferred: &str,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some()
        || !matches!(
            node.location().as_slice(),
            b"URI.regexp" | b"URI.regexp()" | b"::URI.regexp" | b"::URI.regexp()"
        )
    {
        return None;
    }
    let mut ancestors = cx.ancestors();
    ancestors.next()?.as_defined_node()?;
    let array = ancestors.next()?.as_array_node()?;
    if array.elements().len() != 1 {
        return None;
    }
    let statements = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || statements.body().len() != 2 {
        return None;
    }
    let body = statements.body();
    let mut nodes = body.iter();
    let require = nodes.next()?.as_call_node()?;
    if !matches!(
        require.location().as_slice(),
        b"require 'uri'" | b"require \"uri\""
    ) || nodes.next()?.location().start_offset() != array.location().start_offset()
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset()..node.location().end_offset(),
        replacement: preferred.into(),
    })
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
    fn all_pinned_specs_canonical_branches_source_and_control_rows() {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/uri_regexp.json")).unwrap();
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            let found: Vec<_> = offenses(source)
                .into_iter()
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            assert_eq!(found, expected, "{row}");
            let found = crate::cops::testing::correction_offenses(&super::UriRegexp, source);
            assert_eq!(
                crate::corrections::apply(source.as_bytes(), &found)
                    .unwrap()
                    .bytes,
                row["project_after"].as_str().unwrap().as_bytes(),
                "{row}"
            );
        }
    }

    #[test]
    fn bounded_large_sibling_set_retains_all_uncertain_findings() {
        let source = "URI.regexp\n".repeat(6000);
        let found = crate::cops::testing::correction_offenses(&super::UriRegexp, &source);
        assert_eq!(found.len(), 6000);
        for (i, offense) in found.iter().enumerate() {
            assert_eq!(
                (offense.line, offense.col, offense.end_line),
                (i as u32 + 1, 5, i as u32 + 1)
            );
            assert!(offense.correction.is_none());
        }
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
    fn uncertain_bindings_calls_arguments_comments_and_observers_keep_findings() {
        for source in [
            "URI.regexp",
            "require 'uri'\n[URI.regexp]",
            "[defined?(URI.regexp)]",
            "require 'other'\n[defined?(URI.regexp)]",
            "require 'uri'\ndefined?(URI.regexp)",
            "require 'uri'\n[defined?(URI.regexp('http'))]",
            "require 'uri'\n[defined?(URI.regexp { work })]",
            "require 'uri'\n[defined?(URI.regexp(&block))]",
            "require 'uri'\n[defined?(URI.regexp( # keep\n))]",
            "require 'uri'\n[defined?(URI.regexp), 1]",
            "require 'uri'\n[defined?(URI.regexp)]\nGC.stat",
            "require 'uri'\n[defined?(URI.regexp)]\n__END__\ndata\n",
            "module URI; end\n[defined?(URI.regexp)]",
            "require 'uri'\nURI = Other\n[defined?(URI.regexp)]",
            "def f; require 'uri'; [defined?(URI.regexp)]; end",
        ] {
            let found = crate::cops::testing::correction_offenses(&super::UriRegexp, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
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
