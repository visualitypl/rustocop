//! `Lint/UriEscapeUnescape` — removed URI methods, without guessing replacements.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct UriEscapeUnescape;
impl Cop for UriEscapeUnescape {
    fn name(&self) -> &'static str {
        "Lint/UriEscapeUnescape"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism fuses the zero-argument getter send into compound writes.
        macro_rules! getter {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                if call.is_safe_navigation() {
                    return;
                }
                let Some(selector) = call.message_loc() else {
                    return;
                };
                (call.read_name(), call.receiver(), selector.end_offset())
            }};
        }
        let (name, receiver, end) = match node {
            Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
            Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
            Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
            _ => {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if call.is_safe_navigation() {
                    return;
                }
                (call.name(), call.receiver(), ast::source_end(node))
            }
        };
        let method = name.as_slice();
        if !matches!(method, b"escape" | b"encode" | b"unescape" | b"decode") {
            return;
        }
        let Some(receiver) = receiver.filter(|r| ast::is_global_const(r, &["URI"])) else {
            return;
        };
        let root = if receiver.as_constant_path_node().is_some() {
            "::"
        } else {
            ""
        };
        let (cgi, replacement) = if matches!(method, b"escape" | b"encode") {
            ("escape", "encode")
        } else {
            ("unescape", "decode")
        };
        let method = String::from_utf8_lossy(method);
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(), severity: self.severity(), line, col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: format!("`{root}URI.{method}` method is obsolete and should not be used. Instead, use `CGI.{cgi}`, `URI.{replacement}_www_form` or `URI.{replacement}_www_form_component` depending on your specific use case."),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/uri_escape_unescape_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UriEscapeUnescape")
            .collect()
    }
    #[test]
    fn all_four_removed_methods_have_exact_messages() {
        for (method, replacement) in [
            ("escape", "encode"),
            ("encode", "encode"),
            ("unescape", "decode"),
            ("decode", "decode"),
        ] {
            for root in ["", "::"] {
                for args in ["'http://example.com'", "'@?@!', '!?'", "enc_uri"] {
                    let source = format!("# before\n  {root}URI.{method}({args})\n");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    let cgi = if replacement == "encode" {
                        "escape"
                    } else {
                        "unescape"
                    };
                    assert_eq!(
                        found[0].message,
                        format!(
                            "`{root}URI.{method}` method is obsolete and should not be used. Instead, use `CGI.{cgi}`, `URI.{replacement}_www_form` or `URI.{replacement}_www_form_component` depending on your specific use case."
                        )
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn modern_methods_other_receivers_and_safe_navigation_are_clean() {
        for source in [
            "CGI.escape(uri)",
            "CGI.unescape(uri)",
            "URI.encode_uri_component(uri)",
            "URI.decode_uri_component(uri)",
            "URI.encode_www_form(page: 10)",
            "URI.decode_www_form(uri)",
            "URI.encode_www_form_component(uri)",
            "URI.decode_www_form_component(uri)",
            "Other::URI.escape(uri)",
            "::Other::URI.decode(uri)",
            "(URI).escape(uri)",
            "URI&.escape(uri)",
            "uri.escape(uri)",
            "escape(uri)",
            "# URI.escape(uri)\n'URI.decode(uri)'",
            "def URI.escape(x); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
