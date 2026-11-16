//! `Lint/SendWithMixinArgument` — static indirect mixin dispatch.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SendWithMixinArgument;
impl Cop for SendWithMixinArgument {
    fn name(&self) -> &'static str {
        "Lint/SendWithMixinArgument"
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
        if call.is_safe_navigation()
            || !matches!(
                call.name().as_slice(),
                b"send" | b"public_send" | b"__send__"
            )
            || call
                .receiver()
                .is_some_and(|r| r.as_self_node().is_none() && !ast::is_const(&r))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let Some((first, modules)) = args.split_first() else {
            return;
        };
        if modules.is_empty() || !modules.iter().all(ast::is_const) {
            return;
        }
        let Some(method) = mixin_method(first, file) else {
            return;
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let end = ast::source_end(node);
        let names = modules
            .iter()
            .map(|n| source_text(n.location().as_slice(), file))
            .collect::<Vec<_>>()
            .join(", ");
        let bad = source_text(&file.bytes[selector.start_offset()..end], file);
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(selector.start_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!("Use `{method} {names}` instead of `{bad}`."),
            // RuboCop replaces from the selector to the end of the call. A
            // replacement is UTF-8 text: other bytes cannot be carried over.
            correction: (cx.corrections_requested()
                && std::str::from_utf8(&file.bytes[selector.start_offset()..end]).is_ok())
            .then(|| crate::offense::Correction {
                range: selector.start_offset()..end,
                replacement: format!("{method} {names}"),
            }),
        });
    }
}
fn source_text(bytes: &[u8], file: &SourceFile) -> String {
    file.match_pattern_source(bytes)
        .utf8_chunks()
        .map(|c| {
            let mut s = c.valid().to_owned();
            for b in c.invalid() {
                s.push_str(&format!("\\x{b:02X}"));
            }
            s
        })
        .collect::<String>()
        .replace("\r\n", "\n")
}
fn mixin_method(node: &Node<'_>, file: &SourceFile) -> Option<&'static str> {
    let value = if let Some(s) = node.as_symbol_node() {
        if !ast::is_reference_symbol(node, file) {
            return None;
        }
        let raw = s.value_loc();
        let single = s
            .opening_loc()
            .is_some_and(|l| l.as_slice() == b":'" || l.as_slice().starts_with(b"%s"));
        let parts = ast::require_literal::parts(
            raw.as_ref().map_or(s.unescaped(), |l| l.as_slice()),
            s.unescaped(),
            single,
            file,
        );
        if parts.len() != 1 {
            return None;
        }
        parts.into_iter().next()?
    } else if let Some(s) = node.as_string_node() {
        let single = s.opening_loc().is_some_and(|l| {
            l.as_slice().starts_with(b"'")
                || l.as_slice().starts_with(b"%q")
                || l.as_slice().ends_with(b"'")
        });
        let parts =
            ast::require_literal::parts(s.content_loc().as_slice(), s.unescaped(), single, file);
        if parts.len() != 1 {
            return None;
        }
        parts.into_iter().next()?
    } else if node.as_source_file_node().is_some() {
        file.source_path().as_os_str().as_encoded_bytes().to_vec()
    } else {
        return None;
    };
    match value.as_slice() {
        b"include" => Some("include"),
        b"prepend" => Some("prepend"),
        b"extend" => Some("extend"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop lint/send_with_mixin_argument_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::pipeline::lint_str;
    #[test]
    fn pinned_main_examples() {
        for dispatch in ["send", "public_send", "__send__"] {
            for method in ["include", "prepend", "extend"] {
                let s = format!("Foo.{dispatch}(:{method}, Bar)\n");
                let found: Vec<_> = lint_str(&s)
                    .into_iter()
                    .filter(|o| o.cop == "Lint/SendWithMixinArgument")
                    .collect();
                assert_eq!(found.len(), 1);
                assert_eq!((found[0].line, found[0].col), (1, 5));
                assert_eq!(
                    found[0].message,
                    format!("Use `{method} Bar` instead of `{dispatch}(:{method}, Bar)`.")
                );
            }
        }
    }
}
