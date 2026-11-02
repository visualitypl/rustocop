//! `Lint/SendWithMixinArgument` — static indirect mixin dispatch.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
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
            correction: cx
                .corrections_requested()
                .then(|| inactive_mixin(node, method, cx, file))
                .flatten(),
        });
    }
}
// Indirect dispatch can be overridden, bypass visibility or change warnings,
// allocation, hooks, exceptions and values. Only the sole call in a closed
// top-level `if false` is proved. Exact ASCII constant paths and a plain symbol
// remove no binding/comment/heredoc and execute neither dispatch. Existing main
// parse source/encoding/position/code-object/heap guards remain authoritative.
fn inactive_mixin(
    node: &Node<'_>,
    method: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if file.data_start().is_some() {
        return None;
    }
    let call = node.as_call_node()?;
    if call.block().is_some() {
        return None;
    }
    let args = ast::call_arguments(&call);
    let mut paths = Vec::new();
    for module in &args[1..] {
        paths.push(plain_constant(module)?);
    }
    let receiver = match call.receiver() {
        None => String::new(),
        Some(r) if r.as_self_node().is_some() => "self.".into(),
        Some(r) => format!("{}.", plain_constant(&r)?),
    };
    let selector = call.message_loc()?;
    let name = std::str::from_utf8(call.name().as_slice()).ok()?;
    let modules = paths.join(", ");
    if node.location().as_slice() != format!("{receiver}{name}(:{method}, {modules})").as_bytes() {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let body = ancestors.next()?.as_statements_node()?;
    let outer = ancestors.next()?.as_if_node()?;
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some()
        || body.body().len() != 1
        || top.body().len() != 1
        || outer.predicate().as_false_node().is_none()
        || outer.subsequent().is_some()
        || outer.end_keyword_loc().is_none()
        || outer.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    Some(Correction {
        range: selector.start_offset()..ast::source_end(node),
        replacement: format!("{method} {modules}"),
    })
}
fn plain_constant(node: &Node<'_>) -> Option<String> {
    let text = std::str::from_utf8(node.location().as_slice()).ok()?;
    let path = text.strip_prefix("::").unwrap_or(text);
    path.split("::")
        .all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .then(|| text.to_owned())
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
    fn canonical_edit_and_dispatch_holds() {
        use super::SendWithMixinArgument;
        for method in ["include", "prepend", "extend"] {
            for dispatch in ["send", "public_send", "__send__"] {
                for receiver in ["", "self.", "Foo.", "A::Foo."] {
                    let call = format!("{receiver}{dispatch}(:{method}, Bar, Baz)");
                    let source = format!("if false\n  {call}\nend\n");
                    let found =
                        crate::cops::testing::correction_offenses(&SendWithMixinArgument, &source);
                    assert_eq!(found.len(), 1);
                    assert!(found[0].correction.is_some(), "{source}");
                    let after = crate::corrections::apply(source.as_bytes(), &found).unwrap();
                    assert_eq!(
                        after.bytes,
                        source
                            .replace(
                                &format!("{dispatch}(:{method}, Bar, Baz)"),
                                &format!("{method} Bar, Baz")
                            )
                            .as_bytes()
                    );
                    let found =
                        crate::cops::testing::correction_offenses(&SendWithMixinArgument, &call);
                    assert_eq!(found.len(), 1);
                    assert!(found[0].correction.is_none());
                }
            }
        }
        for s in [
            "if false; Foo.send(:include, Bar) {}; end",
            "if false; Foo.send(:include, Bar); nil; end",
            "if false; Foo.send(:include, Bar); end\np __LINE__",
            "if false; Foo.send(:include, Bar); end\n__END__\ndata",
            "if false; Foo.send(:include, # keep\n Bar); end",
        ] {
            let found = crate::cops::testing::correction_offenses(&SendWithMixinArgument, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none(), "{s}");
        }
    }
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
