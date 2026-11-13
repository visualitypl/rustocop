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
            correction: cx
                .corrections_requested()
                .then(|| inactive_conversion(node, context, cx, file))
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
// Removing explicit to_s can change dispatch (including non-String returns),
// allocation, exceptions, safe-navigation behavior, and overridden output APIs.
// Do not assume builtin methods or infer receiver types. Prove only an immediate
// expression in one closed, top-level `if false`. Exact single-interpolation or
// single-output-argument shapes exclude nested bindings, definitions, heredocs,
// line movement, and deleted comments. The main-parse shared source/encoding/
// runtime/position/code-object/heap guards still apply. No candidate parses.
fn inactive_conversion(
    node: &Node<'_>,
    context: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let call = node.as_call_node()?;
    let replacement = if let Some(receiver) = call.receiver() {
        let bare = receiver.as_call_node()?;
        let name = bare.name().as_slice();
        if bare.receiver().is_some()
            || bare.arguments().is_some()
            || bare.block().is_some()
            || name.is_empty()
            || !name.iter().all(|b| b.is_ascii_alphabetic() || *b == b'_')
            || receiver.location().as_slice() != name
        {
            return None;
        }
        std::str::from_utf8(name).ok()?.to_owned()
    } else {
        "self".to_owned()
    };
    let raw = std::str::from_utf8(node.location().as_slice()).ok()?;
    let expected = if call.receiver().is_some() {
        format!(
            "{replacement}{}to_s",
            if call.is_safe_navigation() { "&." } else { "." }
        )
    } else {
        "to_s".to_owned()
    };
    if raw != expected && raw != format!("{expected}()") {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let expression = if context == "interpolation" {
        let statements = ancestors.next()?.as_statements_node()?;
        if statements.body().len() != 1 {
            return None;
        }
        ancestors.next()?.as_embedded_statements_node()?;
        let literal = ancestors.next()?;
        let interpolation = format!("#{{{raw}}}");
        if ![
            format!("\"{interpolation}\""),
            format!("/{interpolation}/"),
            format!(":\"{interpolation}\""),
            format!("`{interpolation}`"),
        ]
        .iter()
        .any(|s| s.as_bytes() == literal.location().as_slice())
        {
            return None;
        }
        literal
    } else {
        ancestors.next()?.as_arguments_node()?;
        let output = ancestors.next()?;
        let method = context.trim_matches('`');
        if output.location().as_slice() != format!("{method} {raw}").as_bytes() {
            return None;
        }
        output
    };
    let body = ancestors.next()?.as_statements_node()?;
    let conditional = ancestors.next()?.as_if_node()?;
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some()
        || body.body().len() != 1
        || top.body().len() != 1
        || body.location().start_offset() != expression.location().start_offset()
        || conditional.predicate().as_false_node().is_none()
        || conditional.subsequent().is_some()
        || conditional.end_keyword_loc().is_none()
        || conditional.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset()..node.location().end_offset(),
        replacement,
    })
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
