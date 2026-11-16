//! `Lint/DeprecatedOpenSSLConstant` — deprecated algorithm constant sends.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
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
                (
                    call.receiver(),
                    Vec::new(),
                    method,
                    end,
                    call.call_operator_loc(),
                )
            }};
        }
        let (receiver, arguments, method, end, dot) = match node {
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
                    call.call_operator_loc(),
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
            // RuboCop removes the algorithm constant and rewrites the call.
            correction: cx
                .corrections_requested()
                .then(|| {
                    // A replacement is UTF-8 text: other bytes cannot be carried over.
                    std::str::from_utf8(&file.bytes[start..end]).ok()?;
                    super::super::style::correction::combine(
                        file,
                        vec![
                            Correction {
                                range: parent.location().end_offset()
                                    ..algorithm.location().end_offset(),
                                replacement: String::new(),
                            },
                            Correction {
                                range: dot?.end_offset()..end,
                                replacement: format!("{method}({args})"),
                            },
                        ],
                    )
                })
                .flatten(),
        });
    }
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
