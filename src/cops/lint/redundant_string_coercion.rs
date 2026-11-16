//! `Lint/RedundantStringCoercion` — redundant explicit string conversions.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RedundantStringCoercion;
impl Cop for RedundantStringCoercion {
    fn name(&self) -> &'static str {
        "Lint/RedundantStringCoercion"
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
        if call.name().as_slice() != b"to_s" || call.arguments().is_some() || call.block().is_some()
        {
            return;
        }
        let Some(context) = context(node, cx) else {
            return;
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(selector.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: format!(
                "{} in {context}.",
                if call.receiver().is_some() {
                    "Redundant use of `Object#to_s`"
                } else {
                    "Use `self` instead of `Object#to_s`"
                }
            ),
            // RuboCop replaces the call with its receiver, or `self`.
            correction: cx
                .corrections_requested()
                .then(|| {
                    let replacement = match call.receiver() {
                        Some(receiver) => std::str::from_utf8(receiver.location().as_slice())
                            .ok()?
                            .into(),
                        None => "self".into(),
                    };
                    Some(crate::offense::Correction {
                        range: node.location().start_offset()..crate::cops::ast::source_end(node),
                        replacement,
                    })
                })
                .flatten(),
        });
    }
}
// Prism's Statements/Arguments are transparent in RuboCop, but explicit
// parentheses and literal blocks are not. Only the final immediate expression
// of an interpolation, or an immediate argument of a receiverless output send.
fn context(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> Option<&'static str> {
    let mut ancestors = cx.ancestors();
    let parent = ancestors.next()?;
    if let Some(statements) = parent.as_statements_node() {
        // A StatementsNode ends at its last direct child (including heredoc
        // call locations). Compare endpoints rather than rescanning siblings.
        if statements.location().end_offset() != node.location().end_offset() {
            return None;
        }
        ancestors.next()?.as_embedded_statements_node()?;
        if matches!(
            ancestors.next()?,
            Node::InterpolatedStringNode { .. }
                | Node::InterpolatedSymbolNode { .. }
                | Node::InterpolatedRegularExpressionNode { .. }
                | Node::InterpolatedXStringNode { .. }
        ) {
            return Some("interpolation");
        }
    } else if parent.as_arguments_node().is_some() {
        let outer = ancestors.next()?.as_call_node()?;
        if outer.receiver().is_none() {
            return match outer.name().as_slice() {
                b"print" => Some("`print`"),
                b"puts" => Some("`puts`"),
                b"warn" => Some("`warn`"),
                _ => None,
            };
        }
    }
    None
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_string_coercion_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantStringCoercion")
            .collect()
    }
    #[test]
    fn pinned_interpolations_and_implicit_receiver() {
        for (prefix, col) in [
            ("\"this is the ", 23),
            ("/regexp ", 18),
            (":\"symbol ", 19),
            ("`backticks ", 21),
        ] {
            let close = if prefix.starts_with('/') {
                "/"
            } else if prefix.starts_with('`') {
                "`"
            } else {
                "\""
            };
            for (operator, extra) in [(".", 0), ("&.", 1)] {
                let source = format!("{prefix}#{{result{operator}to_s}}{close}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, col + extra, 1)
                );
                assert_eq!(
                    found[0].message,
                    "Redundant use of `Object#to_s` in interpolation."
                );
            }
        }
        for (source, col, implicit) in [
            ("\"this is the #{top; result.to_s}\"", 28, false),
            ("\"#{to_s}\"", 4, true),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(
                found[0].message,
                if implicit {
                    "Use `self` instead of `Object#to_s` in interpolation."
                } else {
                    "Redundant use of `Object#to_s` in interpolation."
                }
            );
        }
    }
    #[test]
    fn pinned_output_arguments() {
        for method in ["print", "puts", "warn"] {
            for (args, cols, implicit) in [
                (
                    "first.to_s, second.to_s",
                    vec![method.len() as u32 + 8, method.len() as u32 + 21],
                    false,
                ),
                (
                    "to_s, to_s",
                    vec![method.len() as u32 + 2, method.len() as u32 + 8],
                    true,
                ),
            ] {
                let found = offenses(&format!("{method} {args}"));
                assert_eq!(found.iter().map(|o| o.col).collect::<Vec<_>>(), cols);
                for o in found {
                    assert_eq!(
                        o.message,
                        format!(
                            "{} in `{method}`.",
                            if implicit {
                                "Use `self` instead of `Object#to_s`"
                            } else {
                                "Redundant use of `Object#to_s`"
                            }
                        )
                    );
                }
            }
        }
    }
    #[test]
    fn original_byte_reference_matrix() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_string_coercion.json"
        ))
        .unwrap();
        let mut mismatches = Vec::new();
        for row in rows {
            if row.get("parser_rejection").is_some() {
                continue;
            }
            let bytes: Vec<u8> = serde_json::from_value(row["bytes"].clone()).unwrap();
            let registry = crate::cops::Registry::new();
            let config = crate::config::ResolvedConfig::default();
            let active = crate::cops::select_active(
                &registry,
                &config,
                &["Lint/RedundantStringCoercion".into()],
                &[],
            );
            let report = crate::pipeline::lint_one(
                std::path::Path::new("example.rb"),
                std::path::Path::new("example.rb"),
                Some(&bytes),
                &active,
                &config,
            );
            assert!(
                report.errors.is_empty() && report.internal_errors.is_empty(),
                "{row}\n{report:?}"
            );
            let mut found: Vec<_> = report
                .result
                .offenses
                .into_iter()
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            found.sort();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            if found != expected {
                mismatches.push(format!(
                    "{:?}: {found:?} != {expected:?}",
                    String::from_utf8_lossy(&bytes)
                ));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
    #[test]
    fn canonical_replacement_only_in_a_closed_inactive_envelope() {
        for (before, after) in [
            ("\"#{alpha.to_s}\"", "\"#{alpha}\""),
            ("/\u{23}{alpha&.to_s}/", "/\u{23}{alpha}/"),
            ("puts to_s", "puts self"),
        ] {
            crate::cops::testing::assert_correction(
                &super::RedundantStringCoercion,
                &format!("if false\n  {before}\nend\n"),
                &format!("if false\n  {after}\nend\n"),
            );
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes_and_parent_boundaries() {
        for source in [
            "\"this is a #{result.to_s(8)}\"",
            "\"this is the #{result}\"",
            "\"this is #{} silly\"",
            "print obj.do_something",
            "puts obj.do_something",
            "p first.to_s, second.to_s",
            "obj.print first.to_s, second.to_s",
            "puts obj.to_s(argument)",
            "\"#{x.to_s; other}\"",
            "\"#{(x.to_s)}\"",
            "\"#{x.to_s { 1 }}\"",
            "puts (x.to_s)",
            "puts [x.to_s]",
            "puts x.to_s(&block)",
            "\"#{x.to_s(&block)}\"",
            "\"#{x.to_s.inspect}\"",
            "'#{x.to_s}'",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
