//! `Lint/DeprecatedOpenSSLConstant` — deprecated algorithm constant sends.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

const NO_ARG_ALGORITHM: &[&str] = &["BF", "DES", "IDEA", "RC4"];

pub struct DeprecatedOpenSSLConstant;
impl Cop for DeprecatedOpenSSLConstant {
    fn name(&self) -> &'static str {
        "Lint/DeprecatedOpenSSLConstant"
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
        // Compound writes contain a synthetic zero-argument getter send.
        // Its range ends at the selector, not at the assignment's RHS.
        macro_rules! getter {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                if call.is_safe_navigation()
                    || !matches!(call.read_name().as_slice(), b"new" | b"digest")
                {
                    return;
                }
                let method = call.message_loc().unwrap();
                let end = method.end_offset();
                (call.receiver(), Vec::new(), method, end)
            }};
        }
        let (receiver, arguments, method, end) = match node {
            Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
            Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
            Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
            _ => {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if call.is_safe_navigation()
                    || !matches!(call.name().as_slice(), b"new" | b"digest")
                {
                    return;
                }
                (
                    call.receiver(),
                    ast::call_arguments(&call),
                    call.message_loc().unwrap(),
                    ast::source_end(node),
                )
            }
        };
        let Some(receiver) = receiver else {
            return;
        };
        let Some(algorithm) = receiver.as_constant_path_node() else {
            return;
        };
        let Some(name) = algorithm.name() else { return };
        if name.as_slice() == b"Digest" {
            return;
        }
        let Some(parent) = algorithm.parent() else {
            return;
        };
        let Some(namespace) = parent.as_constant_path_node() else {
            return;
        };
        if !namespace
            .name()
            .is_some_and(|n| matches!(n.as_slice(), b"Cipher" | b"Digest"))
            || !namespace
                .parent()
                .is_some_and(|n| ast::is_global_const(&n, &["OpenSSL"]))
        {
            return;
        }
        if arguments.iter().any(|a| {
            ast::is_variable(a)
                || ast::is_const(a)
                || a.as_call_node()
                    .is_some_and(|c| c.block().is_none_or(|b| b.as_block_node().is_none()))
        }) {
            return;
        }
        let parent_source = text(parent.location().as_slice(), file);
        let name = text(name.as_slice(), file);
        let args = replacement_args(&receiver, &parent_source, &name, &arguments, file);
        let start = node.location().start_offset();
        let original = text(&file.bytes[start..end], file);
        let method = text(method.as_slice(), file);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: format!("Use `{parent_source}.{method}({args})` instead of `{original}`."),
            correction: cx
                .corrections_requested()
                .then(|| closed_literal_digest(node, cx, file))
                .flatten(),
        });
    }
}

/// Only a closed, immediately evaluated two-statement program loading the real
/// stdlib then consuming one SHA256 digest of a plain ASCII message. Native 3.4
/// OpenSSL returns the same binary String without warnings; constructors instead
/// produce distinct subclasses. No user calls, shadowing, redefinitions, delayed
/// execution, argument/block dispatch or observers can intervene in this shape.
/// Exact spelling preserves comments/encoding and excludes heredocs/escapes.
/// Parent/sibling checks are bounded; the existing lazy main-parse source,
/// position, code-object and heap guards remain authoritative for every edit.
fn closed_literal_digest(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let call = node.as_call_node()?;
    if call.name().as_slice() != b"digest" || call.block().is_some() {
        return None;
    }
    let receiver = call.receiver()?;
    let parent = match receiver.location().as_slice() {
        b"OpenSSL::Digest::SHA256" => "OpenSSL::Digest",
        b"::OpenSSL::Digest::SHA256" => "::OpenSSL::Digest",
        _ => return None,
    };
    let arguments = ast::call_arguments(&call);
    let [argument] = arguments.as_slice() else {
        return None;
    };
    argument.as_string_node()?;
    let literal = argument.location().as_slice();
    if literal.len() < 2
        || !matches!(literal[0], b'\'' | b'"')
        || literal.last() != literal.first()
        || literal.iter().any(|b| !b.is_ascii_graphic() && *b != b' ')
        || literal.contains(&b'\\')
    {
        return None;
    }
    if node.location().as_slice()
        != format!("{}::SHA256.digest({})", parent, text(literal, file)).as_bytes()
    {
        return None;
    }
    let mut ancestors = cx.ancestors();
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
        b"require 'openssl'" | b"require \"openssl\""
    ) || nodes.next()?.location().start_offset() != array.location().start_offset()
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset()..node.location().end_offset(),
        replacement: format!("{parent}.digest('SHA256', {})", text(literal, file)),
    })
}

fn text(bytes: &[u8], file: &SourceFile) -> String {
    if file.is_utf8()
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return text.to_owned();
    }
    // Diagnostic-only byte escapes: never reinterpret legacy source as UTF-8
    // (even when its bytes happen to be valid UTF-8), or replace bytes with U+FFFD.
    // The correction path above admits only printable ASCII literals.
    use std::fmt::Write;
    let mut text = String::new();
    let mut remaining = bytes;
    while let Some(&byte) = remaining.first() {
        if byte.is_ascii() {
            text.push(char::from(byte));
            remaining = &remaining[1..];
        } else {
            let width = file.encoded_character_width(remaining).min(remaining.len());
            for byte in &remaining[..width] {
                write!(text, "\\x{byte:02x}").unwrap();
            }
            remaining = &remaining[width..];
        }
    }
    text
}

fn replacement_args(
    receiver: &Node<'_>,
    parent: &str,
    name: &str,
    arguments: &[Node<'_>],
    file: &SourceFile,
) -> String {
    // These are raw source comparisons in the pin, NOT normalized namespaces.
    if receiver.location().as_slice() == b"OpenSSL::Cipher::Cipher" {
        // The pin raises on the zero-argument alias. Keep a useful finding,
        // never panic or expose its malformed correction on that input.
        return arguments
            .first()
            .map_or_else(String::new, |a| text(a.location().as_slice(), file));
    }
    if parent != "OpenSSL::Cipher" {
        return std::iter::once(format!("'{name}'"))
            .chain(
                arguments
                    .iter()
                    .map(|a| text(a.location().as_slice(), file)),
            )
            .collect::<Vec<_>>()
            .join(", ");
    }
    let algorithm = if NO_ARG_ALGORITHM.contains(&name) {
        name.to_owned()
    } else {
        // Ruby scan(/.{3}/) drops an incomplete final chunk.
        name.chars()
            .collect::<Vec<_>>()
            .as_chunks::<3>()
            .0
            .iter()
            .map(|chunk| chunk.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("-")
    };
    let mut parts: Vec<String> = algorithm
        .to_lowercase()
        .split('-')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    if arguments.is_empty()
        && parts
            .first()
            .is_some_and(|p| NO_ARG_ALGORITHM.contains(&p.to_uppercase().as_str()))
    {
        return format!("'{}'", parts[0]);
    }
    let mut size_mode = Vec::new();
    for argument in arguments {
        let value = if matches!(argument, Node::SourceFileNode { .. }) {
            file.path.to_string_lossy().into_owned()
        } else {
            argument.as_string_node().map_or_else(
                || text(argument.location().as_slice(), file),
                |s| text(s.unescaped(), file),
            )
        };
        let sanitized = value.replace([':', '\''], "");
        // Ruby String#split('-') preserves interior/leading empty fields and
        // drops trailing ones. An empty string produces no fields.
        let mut fields: Vec<_> = sanitized.split('-').map(str::to_lowercase).collect();
        while fields.last().is_some_and(String::is_empty) {
            fields.pop();
        }
        size_mode.extend(fields);
    }
    if size_mode.is_empty() {
        size_mode.push("cbc".into());
    }
    parts.extend(size_mode);
    format!(
        "'{}'",
        parts.into_iter().take(3).collect::<Vec<_>>().join("-")
    )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/deprecated_open_ssl_constant_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == DeprecatedOpenSSLConstant.name())
            .collect()
    }
    #[test]
    fn complete_pinned_examples_and_static_translation_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deprecated_open_ssl_constant.json"
        ))
        .unwrap();
        for row in rows {
            let source = row["source"].as_str().unwrap();
            let mut expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            if source.contains("__FILE__") {
                for finding in &mut expected {
                    finding.3 = finding.3.replace("example.rb", "test.rb");
                }
            }
            let found = offenses(source);
            assert_eq!(
                found
                    .iter()
                    .map(|o| (o.line, o.col, o.end_line, o.message.clone()))
                    .collect::<Vec<_>>(),
                expected,
                "{row}"
            );
            let requested =
                crate::cops::testing::correction_offenses(&DeprecatedOpenSSLConstant, source);
            assert!(requested.iter().all(|o| o.correction.is_none()), "{row}");
        }
    }

    #[test]
    fn reference_error_inputs_still_report_without_panicking_or_editing() {
        // Pinned Team replay independently verifies these upstream exceptions.
        for (source, replacement) in [
            ("OpenSSL::Cipher::Cipher.new", "OpenSSL::Cipher.new()"),
            (
                "OpenSSL::Cipher::AB.new(:GCM)",
                "OpenSSL::Cipher.new('gcm')",
            ),
        ] {
            let found =
                crate::cops::testing::correction_offenses(&DeprecatedOpenSSLConstant, source);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(
                found[0].message,
                format!("Use `{replacement}` instead of `{source}`.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn compound_assignment_getters_have_exact_diagnostics_and_no_edits() {
        for method in ["new", "digest"] {
            for operator in ["+=", "||=", "&&="] {
                for dispatch in [".", "::"] {
                    let getter = format!("OpenSSL::Digest::SHA256{dispatch}{method}");
                    let source = format!("# header\n  {getter} {operator}\n    1\n");
                    let found = crate::cops::testing::correction_offenses(
                        &DeprecatedOpenSSLConstant,
                        &source,
                    );
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert_eq!(
                        found[0].message,
                        format!("Use `OpenSSL::Digest.{method}('SHA256')` instead of `{getter}`.")
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }

    #[test]
    fn compound_assignment_safe_navigation_and_plain_setters_are_not_getters() {
        for method in ["new", "digest"] {
            for operator in ["+=", "||=", "&&="] {
                let source = format!("OpenSSL::Digest::SHA256&.{method} {operator} 1\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
            let source = format!("OpenSSL::Digest::SHA256.{method} = 1\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }

    #[test]
    fn closed_loaded_sha256_literal_digest_has_a_real_canonical_correction() {
        for root in ["", "::"] {
            for literal in ["'foo'", "\"authored task proof\"", "''", "'a:b-c !'"] {
                let before = format!(
                    "require 'openssl'\n[{root}OpenSSL::Digest::SHA256.digest({literal})]\n"
                );
                let after = format!(
                    "require 'openssl'\n[{root}OpenSSL::Digest.digest('SHA256', {literal})]\n"
                );
                crate::cops::testing::assert_correction(
                    &DeprecatedOpenSSLConstant,
                    &before,
                    &after,
                );
                assert!(offenses(&before)[0].correction.is_none());
            }
        }
    }

    #[test]
    fn uncertain_crypto_binding_evaluation_comments_and_observers_keep_findings() {
        let call = "OpenSSL::Digest::SHA256.digest('foo')";
        for source in [
            format!("[{call}]"),
            format!("require 'openssl'\n{call}"),
            format!("require 'openssl'\n[{call}, 1]"),
            format!("require 'openssl'\n[defined?({call})]"),
            format!("require 'openssl'\n[({call})]"),
            format!("require 'openssl'\n[-> {{ {call} }}]"),
            format!("require 'openssl'\n[{call}]\n__END__\ndata"),
            format!("require 'openssl'\n[{call}]\np __LINE__"),
            format!("require 'openssl'\n[{call}]\nFile.read(__FILE__)"),
            format!(
                "require 'openssl'\n[{call}]\nRubyVM::InstructionSequence.compile_file(__FILE__)"
            ),
            format!("require 'openssl'\n[{call}]\nObjectSpace.each_object {{}}"),
            format!("require 'openssl'\n[{call}]\nGC.stat"),
            format!("require 'openssl'\nTracePoint.new(:call) {{}}.enable\n[{call}]"),
            format!("require 'openssl'\nOpenSSL = Other\n[{call}]"),
            format!("require 'openssl'\nmodule M\n[{call}]\nend"),
            format!("require 'openssl'\n[{call} {{ work }}]"),
            "require 'openssl'\n[OpenSSL::Digest::SHA256.new('foo')]".into(),
            "require 'openssl'\n[OpenSSL::Cipher::AES.new(128, :GCM)]".into(),
            "require 'openssl'\n[OpenSSL::Digest::SHA1.digest('foo')]".into(),
            "require 'openssl'\n[OpenSSL::Digest::SHA256.digest(nil)]".into(),
            "require 'openssl'\n[OpenSSL::Digest::SHA256.digest('foo', 'bar')]".into(),
            "require 'openssl'\n[OpenSSL::Digest::SHA256.digest(\"#{1}\")]".into(),
            "require 'openssl'\n[OpenSSL::Digest::SHA256.digest('f\\oo')]".into(),
            "require 'openssl'\n[OpenSSL::Digest::SHA256.digest(\n# keep\n'foo')]".into(),
            "require 'openssl'\n[OpenSSL::Digest::SHA256.digest(<<~DOC)]\nfoo\nDOC\n".into(),
        ] {
            let found =
                crate::cops::testing::correction_offenses(&DeprecatedOpenSSLConstant, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn canonical_cipher_diagnostic() {
        let found = offenses("# before\n  OpenSSL::Cipher::AES.new(128, :GCM)\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Use `OpenSSL::Cipher.new('aes-128-gcm')` instead of `OpenSSL::Cipher::AES.new(128, :GCM)`."
        );
    }
}
