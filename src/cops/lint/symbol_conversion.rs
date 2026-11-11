//! `Lint/SymbolConversion` — unnecessary conversions and quoted symbols (strict).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SymbolConversion;
impl Cop for SymbolConversion {
    fn name(&self) -> &'static str {
        "Lint/SymbolConversion"
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
        let correction = if let Some(call) = node.as_call_node() {
            if call.is_safe_navigation() || !matches!(call.name().as_slice(), b"to_sym" | b"intern")
            {
                return;
            }
            let Some(receiver) = call.receiver() else {
                return;
            };
            let Some(correction) = conversion(&receiver, file) else {
                return;
            };
            correction
        } else if let Some(symbol) = node.as_symbol_node() {
            let Some(value) = symbol_value(&symbol, file) else {
                return;
            };
            let inspected = inspect_symbol(&value, file, symbol.is_forced_binary_encoding());
            let source = project(node.location().as_slice(), file);
            if properly_quoted(&source, &inspected) {
                return;
            }
            if cx
                .parent()
                .is_some_and(|p| p.as_alias_method_node().is_some())
                || cx
                    .parent()
                    .and_then(|p| p.as_array_node())
                    .and_then(|a| a.opening_loc())
                    .is_some_and(|l| {
                        l.as_slice().starts_with(b"%i") || l.as_slice().starts_with(b"%I")
                    })
            {
                return;
            }
            if let Some(pair) = cx
                .parent()
                .and_then(|p| p.as_assoc_node())
                .filter(|p| p.key().location().start_offset() == node.location().start_offset())
            {
                // Pattern shorthand is match_var, not a sym/pair callback.
                if pair.value().as_implicit_node().is_some()
                    && cx
                        .ancestors()
                        .nth(1)
                        .is_some_and(|p| p.as_hash_pattern_node().is_some())
                {
                    return;
                }
                if !value
                    .first()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    return;
                }
                let colon = pair.operator_loc().is_none();
                let correction = if colon {
                    &inspected[1..]
                } else {
                    &inspected[..]
                };
                // Prism includes the final label colon in the SymbolNode.
                let source = if colon {
                    source.strip_suffix(b":").unwrap_or(&source)
                } else {
                    &source
                };
                if properly_quoted(source, correction) {
                    return;
                }
                let mut result = correction.to_vec();
                if colon {
                    result.push(b':');
                }
                result
            } else {
                inspected
            }
        } else {
            return;
        };
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: format!(
                "Unnecessary symbol conversion; use `{}` instead.",
                diagnostic(&correction)
            ),
            correction: cx
                .corrections_requested()
                .then(|| canonical_edit(node, cx, file))
                .flatten(),
        });
    }
}
// Ordinary conversion removal changes dispatch, exceptions, allocations and
// observability. Prove ONLY the inventoried canonical spelling in a closed,
// top-level inactive envelope; all other findings remain unedited. No argument,
// block, interpolation, binding, delimiter or line movement can enter this edit.
// Existing main-parse encoding/runtime/source/code-object/heap/atomic guards
// still decide whether the proposed individual edit may be applied.
fn canonical_edit(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    if file.data_start().is_some() || node.location().as_slice() != b":foo.to_sym" {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let body = ancestors.next()?.as_statements_node()?;
    let conditional = ancestors.next()?.as_if_node()?;
    let top = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some()
        || body.body().len() != 1
        || top.body().len() != 1
        || conditional.predicate().as_false_node().is_none()
        || conditional.subsequent().is_some()
        || conditional.end_keyword_loc().is_none()
        || conditional.if_keyword_loc()?.as_slice() != b"if"
    {
        return None;
    }
    Some(crate::offense::Correction {
        range: node.location().start_offset() + 4..node.location().end_offset(),
        replacement: String::new(),
    })
}
fn diagnostic(bytes: &[u8]) -> String {
    let mut result = String::new();
    for chunk in bytes.utf8_chunks() {
        result.push_str(chunk.valid());
        for b in chunk.invalid() {
            use std::fmt::Write;
            let _ = write!(result, "\\x{b:02X}");
        }
    }
    result
}
fn project(raw: &[u8], file: &SourceFile) -> Vec<u8> {
    let bytes = file.match_pattern_source(raw);
    bytes
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b != b'\r' || bytes.get(i + 1) != Some(&b'\n')).then_some(b))
        .collect()
}
fn payload(raw: &[u8], value: &[u8], single: bool, file: &SourceFile) -> Vec<u8> {
    let mut parts = ast::require_literal::parts(raw, value, single, file);
    // Buffer CRLF normalization precedes Prism's own normalization. Only
    // physical newline payloads lose this extra CR, never numeric escapes.
    let (mut r, mut part) = (0, 0);
    while r < raw.len() {
        if raw[r] == b'\\' {
            r += ast::require_literal::escape(&raw[r..], single, file)
                .0
                .max(1);
        } else if raw[r..].starts_with(b"\r\r\n") {
            if let Some(p) = parts.get_mut(part).filter(|p| p.ends_with(b"\r\n")) {
                p.remove(p.len() - 2);
            }
            r += 3;
            part += 1;
        } else if raw[r..].starts_with(b"\r\n") || raw[r] == b'\n' {
            r += if raw[r] == b'\r' { 2 } else { 1 };
            part += 1;
        } else {
            r += file.encoded_character_width(&raw[r..]).max(1);
        }
    }
    if !single {
        super::lambda_without_literal_block::project_buffer_continuations(raw, &mut parts, file);
    }
    parts.concat()
}
// Port of the pinned Prism translator's emitted str-token grouping (MIT,
// Shopify Inc.). Payload bytes still come ONLY from the main parse/alignment.
fn string_parts(raw: &[u8], value: &[u8], single: bool, file: &SourceFile) -> Vec<Vec<u8>> {
    let escaped = project(raw, file);
    let value = payload(raw, value, single, file);
    if !escaped.contains(&b'\n') {
        return vec![value];
    }
    let cooked: Vec<_> = value.split_inclusive(|&b| b == b'\n').collect();
    if single {
        return escaped
            .split_inclusive(|&b| b == b'\n')
            .enumerate()
            .map(|(i, _)| cooked.get(i).unwrap_or(&b"".as_slice()).to_vec())
            .collect();
    }
    let mut parts = Vec::new();
    let (mut slots, mut group_slots, mut previous) = (0, 1, 0);
    let mut lines = escaped.split_inclusive(|&b| b == b'\n').peekable();
    while let Some(line) = lines.next() {
        let (mut backslashes, mut newlines) = (0, 0usize);
        for &b in line {
            if b == b'\\' {
                backslashes += 1;
            } else {
                newlines += usize::from(b == b'n' && backslashes % 2 == 1);
                backslashes = 0;
            }
        }
        if !line.ends_with(b"\n") && newlines > 0 {
            newlines -= 1;
        }
        group_slots += newlines;
        let before_lf = line.strip_suffix(b"\n").unwrap_or(line);
        let before_lf = before_lf.strip_suffix(b"\r").unwrap_or(before_lf);
        let continuation = line.ends_with(b"\n")
            && before_lf.iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
        if !continuation || lines.peek().is_none() {
            slots += group_slots;
            if slots <= cooked.len() {
                parts.push(cooked[previous..slots].concat());
                previous = slots;
            }
            group_slots = 1;
        }
    }
    parts
}
// Also used by ToEnumArguments' sym-only selectors and keyword keys. Keep this
// main-AST emitted-value projection separate from structural identity helpers.
pub(super) fn symbol_value(
    symbol: &ruby_prism::SymbolNode<'_>,
    file: &SourceFile,
) -> Option<Vec<u8>> {
    let raw = symbol.value_loc();
    let raw = raw.as_ref().map_or(symbol.unescaped(), |l| l.as_slice());
    let single = symbol
        .opening_loc()
        .is_some_and(|l| l.as_slice().ends_with(b"'") || l.as_slice().starts_with(b"%s"));
    // pair_quoted builds one str directly, unlike standalone symbol_compose.
    if symbol
        .closing_loc()
        .is_some_and(|l| l.as_slice().ends_with(b":"))
    {
        return Some(payload(raw, symbol.unescaped(), single, file));
    }
    if symbol.closing_loc().is_some() && raw.is_empty() {
        return None;
    }
    let parts = string_parts(raw, symbol.unescaped(), single, file);
    (parts.len() == 1).then(|| parts.into_iter().next().unwrap())
}
fn conversion(receiver: &Node<'_>, file: &SourceFile) -> Option<Vec<u8>> {
    if receiver.as_source_file_node().is_some() {
        return Some(inspect_symbol(
            file.source_path().as_os_str().as_encoded_bytes(),
            file,
            false,
        ));
    }
    if let Some(symbol) = receiver.as_symbol_node() {
        return Some(inspect_symbol(
            &symbol_value(&symbol, file)?,
            file,
            symbol.is_forced_binary_encoding(),
        ));
    }
    if let Some(string) = receiver.as_string_node() {
        let opening = string.opening_loc();
        let single = opening
            .as_ref()
            .is_some_and(|l| l.as_slice().ends_with(b"'") || l.as_slice().starts_with(b"%q"));
        let raw = string.content_loc().as_slice();
        let heredoc = opening
            .as_ref()
            .is_some_and(|l| l.as_slice().starts_with(b"<<"));
        let mut parts = string_parts(raw, string.unescaped(), single, file);
        if string.unescaped().is_empty()
            && opening
                .as_ref()
                .is_some_and(|l| l.as_slice().starts_with(b"%") || heredoc)
        {
            parts.clear();
        }
        if heredoc {
            parts.retain(|p| !p.is_empty());
        }
        if parts.len() == 1 {
            return Some(inspect_symbol(
                &parts[0],
                file,
                string.is_forced_binary_encoding(),
            ));
        }
        if heredoc {
            return None;
        }
        let mut correction = b":\"".to_vec();
        if opening.as_ref().is_some_and(|l| l.as_slice() == b"\"") {
            correction.extend(project(raw, file));
        } else {
            correction.extend(parts.concat());
        }
        correction.push(b'"');
        return Some(correction);
    }
    let string = receiver.as_interpolated_string_node()?;
    if let Some(opening) = string
        .opening_loc()
        .filter(|l| l.as_slice().starts_with(b"<<"))
    {
        let single = opening.as_slice().ends_with(b"'");
        let mut children: Vec<Vec<u8>> = Vec::new();
        for part in string.parts().iter() {
            let part = part.as_string_node()?;
            for child in string_parts(
                part.content_loc().as_slice(),
                part.unescaped(),
                single,
                file,
            ) {
                if child.is_empty() {
                    continue;
                }
                if let Some(previous) = children.last_mut().filter(|p| !p.ends_with(b"\n")) {
                    previous.extend(child);
                } else {
                    children.push(child);
                }
            }
        }
        return (children.len() == 1).then(|| inspect_symbol(&children[0], file, false));
    }
    let mut correction = b":\"".to_vec();
    if string.opening_loc().is_some_and(|l| l.as_slice() == b"\"") {
        let raw = receiver.location().as_slice();
        correction.extend(project(&raw[1..raw.len() - 1], file));
    } else {
        correction.extend(dynamic_value(receiver, file));
    }
    correction.push(b'"');
    Some(correction)
}
fn dynamic_value(node: &Node<'_>, file: &SourceFile) -> Vec<u8> {
    if let Some(string) = node.as_string_node() {
        let single = string
            .opening_loc()
            .is_some_and(|l| l.as_slice().ends_with(b"'") || l.as_slice().starts_with(b"%q"));
        string_parts(
            string.content_loc().as_slice(),
            string.unescaped(),
            single,
            file,
        )
        .concat()
    } else if let Some(string) = node.as_interpolated_string_node() {
        string
            .parts()
            .iter()
            .flat_map(|p| dynamic_value(&p, file))
            .collect()
    } else {
        project(node.location().as_slice(), file)
    }
}
fn properly_quoted(source: &[u8], value: &[u8]) -> bool {
    if !source.iter().any(|b| matches!(b, b'\'' | b'"')) || value.ends_with(b"=") {
        return true;
    }
    if source == value {
        return true;
    }
    let mut single = Vec::new();
    for &b in source {
        if b == b'"' {
            single.push(b'\\');
        }
        single.push(if b == b'\'' { b'"' } else { b });
    }
    single == value
}
// Diagnostic Symbol#inspect grammar (Ruby symbol.c), not a source edit builder.
fn inspect_symbol(value: &[u8], file: &SourceFile, binary: bool) -> Vec<u8> {
    let printable = if binary || file.match_pattern_binary() {
        value.is_ascii() && value.iter().all(|b| (0x20..0x7f).contains(b))
    } else {
        std::str::from_utf8(value).is_ok_and(|s| {
            s.chars().all(|c| {
                let mut b = [0; 4];
                (c == '\u{85}'
                    || file
                        .inspection_nonprint(c.encode_utf8(&mut b).as_bytes())
                        .is_none())
                    && !c.is_ascii_control()
            })
        })
    };
    let mut result = vec![b':'];
    if printable && symbol_name(value) {
        result.extend(value);
    } else {
        // ASCII-only symbols are interned as US-ASCII, even from UTF-8 strings.
        if value.is_ascii() || binary || file.match_pattern_binary() {
            inspect_bytes(value, &mut result);
        } else {
            super::duplicate_match_pattern::inspect_source(value, file, &mut result);
        }
    }
    result
}
fn inspect_bytes(value: &[u8], out: &mut Vec<u8>) {
    out.push(b'"');
    for (i, &b) in value.iter().enumerate() {
        let escaped: Option<&[u8]> = match b {
            b'"' => Some(b"\\\""),
            b'\\' => Some(b"\\\\"),
            b'\n' => Some(b"\\n"),
            b'\r' => Some(b"\\r"),
            b'\t' => Some(b"\\t"),
            7 => Some(b"\\a"),
            8 => Some(b"\\b"),
            11 => Some(b"\\v"),
            12 => Some(b"\\f"),
            27 => Some(b"\\e"),
            b'#' if value
                .get(i + 1)
                .is_some_and(|c| matches!(c, b'{' | b'@' | b'$')) =>
            {
                Some(b"\\#")
            }
            _ => None,
        };
        if let Some(escaped) = escaped {
            out.extend(escaped);
        } else if !(0x20..0x7f).contains(&b) {
            out.extend(format!("\\x{b:02X}").bytes());
        } else {
            out.push(b);
        }
    }
    out.push(b'"');
}
fn symbol_name(value: &[u8]) -> bool {
    if matches!(
        value,
        b"<" | b"<<"
            | b"<="
            | b"<=>"
            | b">"
            | b">>"
            | b">="
            | b"=~"
            | b"=="
            | b"==="
            | b"*"
            | b"**"
            | b"+"
            | b"+@"
            | b"-"
            | b"-@"
            | b"|"
            | b"^"
            | b"&"
            | b"/"
            | b"%"
            | b"~"
            | b"`"
            | b"[]"
            | b"[]="
            | b"!"
            | b"!="
            | b"!~"
    ) {
        return true;
    }
    let ident = |s: &[u8]| {
        s.first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_' || *b >= 0x80)
            && s.iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b >= 0x80)
    };
    if let Some(name) = value.strip_prefix(b"$") {
        return (name.len() == 1 && b"~*$?!@/\\;,.=:<>\"&`'+0".contains(&name[0]))
            // $0 is the special case above; numbered references cannot start with zero.
            || (name.first().is_some_and(|b| matches!(b, b'1'..=b'9'))
                && name.iter().all(u8::is_ascii_digit))
            || name.strip_prefix(b"-").is_some_and(|n| {
                std::str::from_utf8(n).is_ok_and(|s| s.chars().count() == 1)
                    && n.iter()
                        .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b >= 0x80)
            })
            || ident(name);
    }
    if let Some(name) = value
        .strip_prefix(b"@@")
        .or_else(|| value.strip_prefix(b"@"))
    {
        return ident(name);
    }
    if let Some(value) = value.strip_suffix(b"=") {
        return ident(value);
    }
    let value = value
        .strip_suffix(b"!")
        .or_else(|| value.strip_suffix(b"?"))
        .unwrap_or(value);
    ident(value)
}

#[cfg(test)]
mod tests {
    //! RuboCop `lint/symbol_conversion_spec.rb`, MIT, (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/SymbolConversion")
            .collect()
    }
    #[test]
    fn pinned_conversion_taxonomy_and_exact_messages() {
        for (from, to) in [
            (":foo.to_sym", ":foo"),
            ("\"foo\".to_sym", ":foo"),
            ("\"foo_bar\".to_sym", ":foo_bar"),
            ("\"foo-bar\".to_sym", ":\"foo-bar\""),
            ("\"foo-#{bar}\".to_sym", ":\"foo-#{bar}\""),
            (r#""foo#{bar}\"qux".to_sym"#, r#":"foo#{bar}\"qux""#),
            ("%{foo#{bar}}.to_sym", ":\"foo#{bar}\""),
            ("%Q{foo#{bar}}.to_sym", ":\"foo#{bar}\""),
            ("%(foo#{bar}).to_sym", ":\"foo#{bar}\""),
            ("%<foo#{bar}>.to_sym", ":\"foo#{bar}\""),
            ("\"x\" \"y#{z}\".to_sym", ":\"xy#{z}\""),
            (":\"foo\"", ":foo"),
            (":\"foo_bar\"", ":foo_bar"),
        ] {
            for source in [from.to_string(), from.replace("to_sym", "intern")] {
                let found = offenses(&format!("nil\n  {source}\n"));
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (2, 3, 2),
                    "{source}"
                );
                assert_eq!(
                    found[0].message,
                    format!("Unnecessary symbol conversion; use `{to}` instead."),
                    "{source}"
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn leading_zero_numeric_globals_require_quoted_symbols() {
        for name in ["$00", "$01", "$000", "$010"] {
            for source in [format!(":\"{name}\""), format!(":'{name}'")] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn numeric_global_conversion_advice_preserves_required_quotes() {
        for (name, to) in [
            ("$00", ":\"$00\""),
            ("$01", ":\"$01\""),
            ("$000", ":\"$000\""),
            ("$010", ":\"$010\""),
            ("$0", ":$0"),
            ("$1", ":$1"),
            ("$10", ":$10"),
            ("$101", ":$101"),
        ] {
            for selector in ["to_sym", "intern"] {
                let source = format!("nil\n  \"{name}\".{selector}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}: {found:?}");
                let offense = &found[0];
                assert_eq!((offense.line, offense.col, offense.end_line), (2, 3, 2));
                assert_eq!(
                    offense.message,
                    format!("Unnecessary symbol conversion; use `{to}` instead."),
                    "{source}"
                );
                assert!(offense.correction.is_none());
            }
        }
    }
    #[test]
    fn pinned_strict_hash_keys_and_values() {
        for (s, col, to) in [
            ("{ 'foo': 'bar' }", 3, "foo:"),
            ("{ 'foo!': 'bar' }", 3, "foo!:"),
            ("{ 'foo?': 'bar' }", 3, "foo?:"),
            ("{ :'foo' => :bar }", 3, ":foo"),
            ("{ foo: :'bar' }", 8, ":bar"),
            ("{ foo: \"bar-#{'baz'}\".to_sym }", 8, ":\"bar-#{'baz'}\""),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{s}");
            assert_eq!(
                found[0].message,
                format!("Unnecessary symbol conversion; use `{to}` instead.")
            );
        }
    }
    #[test]
    fn pinned_and_callback_lookalikes() {
        for s in [
            ":foo",
            ":\"foo-bar\"",
            ":'foo-bar'",
            ":'foo-bar\"\"'",
            ":'Foo/Bar/Baz'",
            "{ foo: 'bar' }",
            "{ 'foo-bar': 'bar' }",
            "{ 'foo:bar': 'bar' }",
            "{ 'foo=': 'bar' }",
            "{ '==': 'bar' }",
            "{ \"foo-#{'bar'}\": 'baz' }",
            "{ foo: :bar }",
            "alias foo bar\nalias == equal\nalias eq? ==",
            "%i(foo bar foo-bar)",
            "%I(foo bar foo-bar)",
            "to_sym == other",
            "obj.to_sym",
            "'foo'&.to_sym",
            ":foo&.intern",
            ":\"foo=\"",
            "<<~TEXT.to_sym\n  foo#{bar}\nTEXT\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn physical_token_projection_distinguishes_strings_symbols_labels_and_heredocs() {
        for (source, correction, end_line) in [
            ("'a\nb'.to_sym", ":\"a\nb\"", 2),
            ("\"a\nb\".to_sym", ":\"a\nb\"", 2),
            ("{\"foo\nbar\": 1}", "\"foo\\nbar\":", 2),
            ("<<~DOC.to_sym\na\\\nb\nDOC\n", ":\"ab\\n\"", 1),
            ("\"a\\x00b\".to_sym", ":\"a\\x00b\"", 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].end_line, end_line);
            assert_eq!(
                found[0].message,
                format!("Unnecessary symbol conversion; use `{correction}` instead.")
            );
        }
        for source in [
            ":\"foo\nbar\"",
            ":\"\"",
            ":\"\".to_sym",
            "<<~DOC.to_sym\na\nb\nDOC\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pattern_shorthand_is_a_binding_not_a_symbol_callback() {
        assert!(offenses("case x; in {\"foo\":}; end").is_empty());
        for source in [
            "case x; in {\"foo\": y}; end",
            "case x; in {foo: :\"bar\"}; end",
            "case x; in [:\"foo\"]; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn dispatch_has_no_invented_arity_guard_and_excludes_literal_block_range() {
        for s in [
            "'foo'.to_sym(1)",
            "'foo'.intern(*args)",
            "'foo'.to_sym { :bar }",
            "'foo'.to_sym do\n :bar\nend",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, 1, 1),
                "{s}"
            );
        }
    }
}
