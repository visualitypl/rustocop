//! `Style/SendWithLiteralMethodName` — static public sends, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SendWithLiteralMethodName;
const RESERVED: &str = "BEGIN END alias and begin break case class def defined? do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield";
impl Cop for SendWithLiteralMethodName {
    fn name(&self) -> &'static str {
        "Style/SendWithLiteralMethodName"
    }
    fn unsafe_fixes(&self) -> bool {
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
        if call.name().as_slice() != b"public_send" {
            return;
        }
        let args = ast::call_arguments(&call);
        let Some(first) = args.first() else { return };
        let symbol = first.as_symbol_node();
        let string = first.as_string_node();
        let Some(value) = symbol
            .as_ref()
            .map(|s| s.unescaped())
            .or_else(|| string.as_ref().map(|s| s.unescaped()))
        else {
            return;
        };
        let Some((&head, tail)) = value.split_first() else {
            return;
        };
        let tail = tail
            .strip_suffix(b"?")
            .or_else(|| tail.strip_suffix(b"!"))
            .unwrap_or(tail);
        if !(head.is_ascii_alphabetic() || head == b'_')
            || !tail.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            return;
        }
        let method = std::str::from_utf8(value).expect("ASCII method name");
        if symbol.is_some() && RESERVED.split_whitespace().any(|word| word == method) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        // Call the method directly: the selector through the end becomes the
        // name, or, with more arguments, the selector does and the name
        // argument goes.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let end = crate::cops::bundler::ordered_gem_node::send_end(&call);
                match args.get(1) {
                    None => Some(crate::offense::Correction {
                        range: selector.start_offset()..end,
                        replacement: method.to_owned(),
                    }),
                    Some(second) => crate::offense::Correction::from_edits(
                        &file.bytes,
                        vec![
                            (
                                selector.start_offset()..selector.end_offset(),
                                method.to_owned(),
                            ),
                            (
                                first.location().start_offset()..second.location().start_offset(),
                                String::new(),
                            ),
                        ],
                    ),
                }
            })
            .flatten();
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1)),
            message: format!("Use `{method}` method call directly instead."),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/send_with_literal_method_name_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov). Unsafe corrections not ported.
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    #[test]
    fn pinned_literal_shapes_and_safe_navigation_report_selector() {
        for receiver in ["obj.", "obj&.", ""] {
            for (args, method) in [
                (":foo", "foo"),
                ("'foo'", "foo"),
                ("\"name_with_underscore\"", "name_with_underscore"),
                (":foo, bar, 42", "foo"),
                (":foo?", "foo?"),
                (":foo!", "foo!"),
                ("\"f\\x6fo\"", "foo"),
                (":Upper1", "Upper1"),
                (":_", "_"),
            ] {
                for call in [
                    format!("{receiver}public_send({args})"),
                    format!("{receiver}public_send {args}"),
                ] {
                    let found: Vec<_> = lint_str(&call)
                        .into_iter()
                        .filter(|o| o.cop == SendWithLiteralMethodName.name())
                        .collect();
                    assert_eq!(found.len(), 1, "{call}");
                    assert_eq!(
                        (found[0].line, found[0].col),
                        (1, receiver.len() as u32 + 1)
                    );
                    assert_eq!(
                        found[0].message,
                        format!("Use `{method}` method call directly instead.")
                    );
                    assert!(
                        correction_offenses(&SendWithLiteralMethodName, &call)[0]
                            .correction
                            .is_some()
                    );
                }
            }
        }
    }
    #[test]
    fn pinned_dynamic_invalid_writer_reserved_and_allow_send_lookalikes() {
        for receiver in ["obj.", "obj&.", ""] {
            for arg in [
                "variable",
                "\"#{interpolated}string\"",
                ":\"#{name}\"",
                "'name with space'",
                "'name-with-hyphen'",
                "'name='",
                "'{brackets}'",
                "'[square_brackets]'",
                "42",
                "",
                ":+",
                ":[]",
                "'é'",
                "'9name'",
                "'foo??'",
                "'foo\n'",
            ] {
                let source = format!("{receiver}public_send({arg})");
                assert!(
                    correction_offenses(&SendWithLiteralMethodName, &source).is_empty(),
                    "{source}"
                );
            }
            for word in RESERVED.split_whitespace() {
                let source = format!("{receiver}public_send(:{word})");
                assert!(
                    correction_offenses(&SendWithLiteralMethodName, &source).is_empty(),
                    "{source}"
                );
            }
            for method in ["send", "__send__", "foo"] {
                let source = format!("{receiver}{method}(:foo)");
                assert!(
                    correction_offenses(&SendWithLiteralMethodName, &source).is_empty(),
                    "{source}"
                );
            }
        }
    }
    #[test]
    fn reserved_word_strings_report_while_symbols_are_exempt() {
        for receiver in ["obj.", "obj&.", ""] {
            for word in RESERVED.split_whitespace() {
                for quote in ["'", "\""] {
                    let source = format!("{receiver}public_send({quote}{word}{quote})");
                    let found = correction_offenses(&SendWithLiteralMethodName, &source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col),
                        (1, receiver.len() as u32 + 1)
                    );
                    assert_eq!(
                        found[0].message,
                        format!("Use `{word}` method call directly instead.")
                    );
                    assert!(found[0].correction.is_some());
                }
                let source = format!("{receiver}public_send(:{word})");
                assert!(correction_offenses(&SendWithLiteralMethodName, &source).is_empty());
            }
        }
    }
    #[test]
    fn custom_public_send_is_reported_and_rewritten_as_upstream_does() {
        let source = "def obj.public_send(*args); :custom; end\nobj.public_send(:private_method) { side_effect }\n";
        let found = correction_offenses(&SendWithLiteralMethodName, source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 5));
        assert!(found[0].correction.is_some());
    }
}
