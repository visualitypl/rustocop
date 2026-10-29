//! `Naming/VariableNumber`: normalcase numbering at the pinned fixed defaults.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct VariableNumber;
impl Cop for VariableNumber {
    fn name(&self) -> &'static str {
        "Naming/VariableNumber"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let symbol = node.as_symbol_node();
        let (name, offset, kind, end) = if let Some(def) = node.as_def_node() {
            (
                def.name().as_slice(),
                def.name_loc().start_offset(),
                "method name",
                def.name_loc().end_offset(),
            )
        } else if let Some(sym) = symbol.as_ref() {
            // Standalone physically split symbols are dsym in the reference;
            // quoted hash labels remain sym, as do percent-array elements.
            if sym
                .closing_loc()
                .is_some_and(|close| !close.as_slice().ends_with(b":"))
                && !crate::cops::ast::is_reference_symbol(node, file)
            {
                return;
            }
            // Shorthand hash-pattern keys become match_var, not sym, in
            // RuboCop. Ordinary implicit hash values still have a symbol key.
            if cx
                .parent()
                .and_then(Node::as_assoc_node)
                .is_some_and(|assoc| assoc.value().as_implicit_node().is_some())
                && cx
                    .ancestors()
                    .nth(1)
                    .is_some_and(|p| p.as_hash_pattern_node().is_some())
            {
                return;
            }
            // Static quoted symbols are checked by value; dynamic symbols are not.
            (
                sym.unescaped(),
                sym.location().start_offset(),
                "symbol",
                sym.location().end_offset(),
            )
        } else if let Some((name, offset)) = assignment_name(node, cx) {
            (name, offset, "variable", offset + name.len())
        } else {
            return;
        };
        // AllowedIdentifiers deletes every @/$, including inside symbols.
        if normalcase(
            name,
            file,
            symbol.as_ref().is_some_and(|s| s.is_forced_utf8_encoding()),
        ) || ALLOWED.iter().any(|allowed| {
            name.iter()
                .copied()
                .filter(|b| !matches!(b, b'@' | b'$'))
                .eq(allowed.iter().copied())
        }) {
            return;
        }
        if node
            .as_def_node()
            .is_some_and(|def| def.receiver().is_some())
            && super::class_emitters::is_class_emitter(name, cx, file)
        {
            return;
        }
        let (line, col) = file.line_col(offset);
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use normalcase for {kind} numbers."),
            line,
            end_line: file.line_of(end.saturating_sub(1).max(offset)),
            col,
        });
    }
}

const ALLOWED: [&[u8]; 9] = [
    b"TLS1_1",
    b"TLS1_2",
    b"capture3",
    b"iso8601",
    b"rfc1123_date",
    b"rfc822",
    b"rfc2822",
    b"rfc3339",
    b"x86_64",
];

// Port of ConfigurableNumbering's normalcase regexp. Only a trailing ASCII
// digit run preceded by '_' is forbidden; integer and implicit-param names pass.
fn normalcase(name: &[u8], file: &SourceFile, forced_utf8: bool) -> bool {
    let start = name
        .iter()
        .rposition(|b| !b.is_ascii_digit())
        .map_or(0, |i| i + 1);
    if start == name.len() || start == 0 || name[start - 1] != b'_' || start == 1 {
        return true;
    }
    // Unicode escapes force decoded symbol values to UTF-8 independently of
    // the source encoding. In UTF-8, ASCII '_' is always a character boundary.
    if forced_utf8 {
        return false;
    }
    // Otherwise decoded symbols retain the source encoding. An ASCII-looking
    // trailing byte (e.g. Shift_JIS ダ = 83 5f) is not an underscore.
    let underscore = start - 1;
    let mut offset = 0;
    while offset < underscore {
        offset += file.encoded_character_width(&name[offset..]).max(1);
    }
    offset != underscore
}

/// Syntactic writes, not reads/pattern bindings. No binding-flow analysis or
/// renaming is involved: the existing naming extractor owns assignment forms.
fn assignment_name<'pr>(node: &Node<'pr>, cx: &NodeContext<'_, 'pr>) -> Option<(&'pr [u8], usize)> {
    if let Some(target) = node.as_local_variable_target_node() {
        let assignment = cx
            .ancestors()
            .find(|p| !matches!(p, Node::MultiTargetNode { .. } | Node::SplatNode { .. }));
        return assignment
            .filter(|p| {
                matches!(
                    p,
                    Node::MultiWriteNode { .. } | Node::ForNode { .. } | Node::RescueNode { .. }
                )
            })
            .map(|_| (target.name().as_slice(), target.location().start_offset()));
    }
    macro_rules! global {
        ($cast:ident, $loc:ident) => {{
            let n = node.$cast()?;
            Some((n.name().as_slice(), n.$loc().start_offset()))
        }};
    }
    match node {
        Node::GlobalVariableWriteNode { .. } => global!(as_global_variable_write_node, name_loc),
        Node::GlobalVariableAndWriteNode { .. } => {
            global!(as_global_variable_and_write_node, name_loc)
        }
        Node::GlobalVariableOrWriteNode { .. } => {
            global!(as_global_variable_or_write_node, name_loc)
        }
        Node::GlobalVariableOperatorWriteNode { .. } => {
            global!(as_global_variable_operator_write_node, name_loc)
        }
        Node::GlobalVariableTargetNode { .. } => global!(as_global_variable_target_node, location),
        Node::LocalVariableReadNode { .. }
        | Node::OptionalParameterNode { .. }
        | Node::RestParameterNode { .. }
        | Node::RequiredKeywordParameterNode { .. }
        | Node::OptionalKeywordParameterNode { .. }
        | Node::KeywordRestParameterNode { .. }
        | Node::BlockParameterNode { .. } => None,
        _ => super::variable_name::variable_name(node),
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's naming/variable_number_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov), normalcase fixed defaults.
    use crate::pipeline::lint_str;
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        assert_eq!(
            crate::parser::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/VariableNumber")
            .map(|o| {
                assert!(o.correction.is_none());
                (o.line, o.col, o.message)
            })
            .collect()
    }
    #[test]
    fn pinned_normalcase_assignments() {
        for name in [
            "local_1",
            "sha_256",
            "@local_1",
            "@@local_1",
            "myAttribute_1",
            "@myAttribute_1",
            "_myLocal_1",
            "localFOO_1",
            "local_FOO_1",
            "$arg_1",
        ] {
            assert_eq!(
                found(&format!("{name} = 1")),
                [(1, 1, "Use normalcase for variable numbers.".into())]
            );
        }
        assert_eq!(found("a_1 = 1\na2 = 1").len(), 1);
    }
    #[test]
    fn pinned_methods_arguments_and_symbols() {
        for (source, col, kind) in [
            (":sym_1", 1, "symbol"),
            ("def method(arg_1); end", 12, "variable"),
            ("def method(funnyArg_1); end", 12, "variable"),
            ("def method_1; end", 5, "method name"),
            ("def self.method_1; end", 10, "method name"),
        ] {
            assert_eq!(
                found(source),
                [(1, col, format!("Use normalcase for {kind} numbers."))]
            );
        }
    }
    #[test]
    fn normalcase_lookalikes_and_allowlist() {
        for name in [
            "local1",
            "local_",
            "user1_id",
            "sha256",
            "foo10_bar",
            "target_u2f_device",
            "localFOO1",
            "snake_case",
            "user_1_id",
            "_",
            "_foo",
            "@foo",
            "@__foo__",
            "emparejó",
        ] {
            assert!(found(&format!("{name} = 1")).is_empty(), "{name}");
        }
        for source in [
            ":\"x@86_64\"",
            ":\"TLS$1_1\"",
            ":sym1",
            ":\"42\"",
            "%i[1 2 3]",
            "x = { \"\": [] }",
            ":TLS1_1",
            ":TLS1_2",
            ":x86_64",
            "def x86_64; end",
            "@x86_64 = 1",
            "@@x86_64 = 1",
            "$x86_64 = 1",
            "foo { _1 }",
            "foo { _2 }",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn singleton_class_emitters_keep_the_shared_formatting_exemption() {
        for source in [
            "class A; class Foo_1; end; def self.Foo_1; end; end",
            "module A; def self.included; def self.Foo_1; end; end; class Foo_1; end; end",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        for source in [
            "class A; class Foo_1; end; def Foo_1; end; end",
            "class A; class Foo_1; end; def self.Foo_2; end; end",
            "class A; class Foo_1; end; def self.included; before; def self.Foo_1; end; end; end",
        ] {
            assert_eq!(found(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn class_emitters_use_direct_reference_parent_children() {
        for source in [
            "if cond; def self.Foo_1; end; else; class Foo_1; end; end",
            "if cond; class Foo_1; end; else; def self.Foo_1; end; end",
            "unless cond; def self.Foo_1; end; else; class Foo_1; end; end",
            "begin; def self.Foo_1; end; ensure; class Foo_1; end; end",
            "begin; class Foo_1; end; ensure; def self.Foo_1; end; end",
            "if cond; def self.included; def self.Foo_1; end; end; else; class Foo_1; end; end",
            "begin; def self.included; def self.Foo_1; end; end; ensure; class Foo_1; end; end",
            "begin; def self.Foo_1; end; rescue; other; else; class Foo_1; end; end",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        // Multi-expression branches and nested structural nodes own their
        // children: sharing an outer if/begin is not sufficient.
        for source in [
            "if cond; def self.Foo_1; end; before; else; class Foo_1; end; end",
            "if cond; def self.Foo_1; end; else; before; class Foo_1; end; end",
            "begin; def self.Foo_1; end; before; ensure; class Foo_1; end; end",
            "begin; def self.Foo_1; end; ensure; before; class Foo_1; end; end",
            "begin; def self.Foo_1; end; rescue; class Foo_1; end; end",
            "begin; def self.Foo_1; end; rescue; other; ensure; class Foo_1; end; end",
            "if cond; def Foo_1; end; else; class Foo_1; end; end",
        ] {
            assert_eq!(found(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn legacy_trailing_bytes_are_not_number_separators() {
        use crate::{cops::run_cops_with_corrections, source::SourceFile};
        for encoding in ["Shift_JIS", "Windows-31J"] {
            for (prefix, suffix, col) in [
                ("", " = 1", 1),
                (":", "", 1),
                (":\"", "\"", 1),
                ("def m(", "); end", 7),
                ("foo { |", "| }", 8),
                ("def ", "; end", 5),
            ] {
                for (separator, count) in [("", 0), ("_", 1)] {
                    let mut bytes = format!("# encoding: {encoding}\n{prefix}foo").into_bytes();
                    // Shift_JIS ダ: its trailing byte looks like ASCII '_'.
                    bytes.extend_from_slice(b"\x83\x5f");
                    bytes.extend_from_slice(format!("{separator}1{suffix}\n").as_bytes());
                    let file = SourceFile::new("x.rb".into(), bytes);
                    let parsed = crate::parser::parse(&file.bytes);
                    assert_eq!(parsed.errors().count(), 0);
                    let mut found = Vec::new();
                    run_cops_with_corrections(
                        &[&super::VariableNumber],
                        &parsed,
                        &file,
                        &mut found,
                        false,
                    );
                    assert_eq!(
                        found.len(),
                        count,
                        "{encoding}: {prefix}fooダ{separator}1{suffix}"
                    );
                    if count == 1 {
                        assert_eq!((found[0].line, found[0].col), (2, col));
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn unicode_escaped_symbols_use_the_decoded_value_encoding() {
        for encoding in ["Shift_JIS", "Windows-31J", "EUC-JP", "ISO-8859-1"] {
            for (symbol, col) in [
                (":\"\\u{401}_1\"", 1),
                (":\"\\u0401_1\"", 1),
                ("%I[\\u{401}_1]", 4),
                ("{\"\\u{401}_1\": 0}", 2),
            ] {
                assert_eq!(
                    found(&format!("# encoding: {encoding}\n{symbol}\n")),
                    [(2, col, "Use normalcase for symbol numbers.".into())],
                    "{encoding}: {symbol}"
                );
            }
            for symbol in [":\"\\u{401}1\"", ":\"\\u{401}_\"", ":\"\\u{401}_1x\""] {
                assert!(found(&format!("# encoding: {encoding}\n{symbol}\n")).is_empty());
            }
        }
    }
    #[test]
    fn physical_symbol_lines_follow_reference_sym_versus_dsym_shapes() {
        for source in [":\"sym_1\nfoo_2\"", "%s(sym_1\nfoo_2)", ":'sym_\\\n1'"] {
            assert!(found(source).is_empty(), "{source}");
        }
        for source in [
            ":\"sym_1\\nfoo_2\"",
            ":\"sym_\\\n1\"",
            ":\"\\\nfoo_1\"",
            "%i[sym_1\nfoo_2]",
            "{\"foo_1\nbar_2\": 1}",
        ] {
            assert!(!found(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn implicit_pattern_keys_are_not_symbol_nodes_in_the_reference() {
        assert!(found("case {}; in {arg_1:}; end").is_empty());
        assert_eq!(found("case {}; in {arg_1: x}; end").len(), 1);
        assert_eq!(found("arg_1 = 1; foo(arg_1:)").len(), 2);
    }
    #[test]
    fn assignment_context_not_reads_patterns_or_non_arg_parameters() {
        for source in [
            "a_1 ||= 1",
            "a_1 &&= 1",
            "a_1 += 1",
            "a_1, b = 1,2",
            "a, (b, *c_1) = 1,[2,3]",
            "for a_1 in []; end",
            "begin; rescue => a_1; end",
            "@a_1, @@b_1, $c_1 = 1,2,3",
            "foo { |(a_1, b)| }",
        ] {
            assert!(!found(source).is_empty(), "{source}");
        }
        for source in [
            "foo_1()",
            "@a_1",
            "$a_1",
            "case []; in [a_1]; end",
            "/(?<a_1>.)/ =~ s",
            "foo { |a; b_1| }",
            "def m(a_1 = 1, *b_1, c_1:, d_1: 2, **e_1, &f_1); end",
            "foo { |a_1 = 1| }",
            "A_1 = 1",
            ":\"foo_#{value}\"",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        assert_eq!(found("a_1 = 1; a_1").len(), 1);
        assert_eq!(found("alias foo_1 bar_1").len(), 2);
    }
}
