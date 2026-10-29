//! `Naming/AsciiIdentifiers`: non-ASCII identifier tokens and constants.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct AsciiIdentifiers;
impl Cop for AsciiIdentifiers {
    fn name(&self) -> &'static str {
        "Naming/AsciiIdentifiers"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((start, end, constant)) = token(node, cx, file) else {
            return;
        };
        let bytes = &file.bytes[start..end];
        let mut offset = 0;
        while offset < bytes.len() {
            let width = file.encoded_character_width(&bytes[offset..]).max(1);
            if !bytes[offset..offset + width].is_ascii() {
                let (line, col) = file.line_col(start + offset);
                out.push(Offense {
                    correction: None,
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Use only ascii symbols in {}.",
                        if constant { "constants" } else { "identifiers" }
                    ),
                    line,
                    end_line: line,
                    col,
                });
                break;
            }
            offset += width;
        }
    }
}

/// AST-owned lexical names, not arbitrary source words. Reads/targets are
/// checked independently, exactly as the reference's identifier token scan.
pub(super) fn token(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<(usize, usize, bool)> {
    // Implicit values/bindings share a keyword label's source, not an
    // identifier token. Test AST ownership, not an adjacent colon: compact
    // ternaries can put a real identifier immediately before their colon.
    if cx.parent().is_some_and(|p| p.as_implicit_node().is_some()) {
        return None;
    }
    macro_rules! loc {
        ($cast:ident, $loc:ident, $constant:expr) => {{
            let n = node.$cast()?;
            let l = Option::<ruby_prism::Location<'_>>::from(n.$loc())?;
            Some((l.start_offset(), l.end_offset(), $constant))
        }};
    }
    match node {
        Node::ConstantReadNode { .. } => loc!(as_constant_read_node, location, true),
        Node::ConstantTargetNode { .. } => loc!(as_constant_target_node, location, true),
        Node::ConstantPathNode { .. } => loc!(as_constant_path_node, name_loc, true),
        Node::ConstantPathTargetNode { .. } => loc!(as_constant_path_target_node, name_loc, true),
        Node::ConstantWriteNode { .. } => loc!(as_constant_write_node, name_loc, true),
        Node::ConstantAndWriteNode { .. } => loc!(as_constant_and_write_node, name_loc, true),
        Node::ConstantOrWriteNode { .. } => loc!(as_constant_or_write_node, name_loc, true),
        Node::ConstantOperatorWriteNode { .. } => {
            loc!(as_constant_operator_write_node, name_loc, true)
        }
        Node::LocalVariableTargetNode { .. } => {
            if cx
                .parent()
                .is_some_and(|p| p.as_match_write_node().is_some())
            {
                return None;
            }
            loc!(as_local_variable_target_node, location, false)
        }
        Node::BlockLocalVariableNode { .. } => loc!(as_block_local_variable_node, location, false),
        Node::DefNode { .. } => {
            let def = node.as_def_node()?;
            let name = def.name().as_slice();
            // Translation::Parser changes a METHOD_NAME into tIDENTIFIER only
            // immediately after `def`, not after a singleton receiver's dot.
            if def.receiver().is_some() && matches!(name.last(), Some(b'?' | b'!')) {
                return None;
            }
            loc!(as_def_node, name_loc, starts_uppercase(file, name))
        }
        Node::CallNode { .. } => {
            let call = node.as_call_node()?;
            if matches!(call.name().as_slice().last(), Some(b'?' | b'!')) {
                return None;
            }
            selector_token(call.message_loc()?, call.name().as_slice(), file)
        }
        Node::CallTargetNode { .. }
        | Node::CallAndWriteNode { .. }
        | Node::CallOrWriteNode { .. }
        | Node::CallOperatorWriteNode { .. } => {
            macro_rules! selector {
                ($cast:ident, $name:ident) => {{
                    let call = node.$cast()?;
                    selector_token(
                        Option::<ruby_prism::Location<'_>>::from(call.message_loc())?,
                        call.$name().as_slice(),
                        file,
                    )
                }};
            }
            match node {
                Node::CallTargetNode { .. } => selector!(as_call_target_node, name),
                Node::CallAndWriteNode { .. } => selector!(as_call_and_write_node, read_name),
                Node::CallOrWriteNode { .. } => selector!(as_call_or_write_node, read_name),
                _ => selector!(as_call_operator_write_node, read_name),
            }
        }
        Node::SymbolNode { .. }
            if cx.parent().is_some_and(|p| {
                matches!(p, Node::AliasMethodNode { .. } | Node::UndefNode { .. })
            }) =>
        {
            let sym = node.as_symbol_node()?;
            let l = sym.location();
            if sym.opening_loc().is_some() || matches!(l.as_slice().last(), Some(b'?' | b'!')) {
                return None;
            }
            Some((
                l.start_offset(),
                l.end_offset(),
                starts_uppercase(file, l.as_slice()),
            ))
        }
        Node::RequiredKeywordParameterNode { .. } | Node::OptionalKeywordParameterNode { .. } => {
            None
        }
        _ => {
            let (name, start) = super::variable_name::variable_name(node)?;
            if matches!(name.first(), Some(b'@' | b'$')) {
                return None;
            }
            Some((start, start + name.len(), false))
        }
    }
}

/// A message location need not be a selector token: Prism's implicit `[]` /
/// `[]=` sends cover the entire bracket argument list. Verify ownership against
/// the semantic name before inspecting bytes. Setters may omit their synthetic
/// trailing `=` in source; explicitly written operators are ASCII and harmless.
/// Slice equality rejects a bracket range by length without scanning its contents;
/// receiver/argument/interpolation nodes remain independently visited as usual.
fn selector_token(
    location: ruby_prism::Location<'_>,
    name: &[u8],
    file: &SourceFile,
) -> Option<(usize, usize, bool)> {
    let written = location.as_slice();
    if written != name && Some(written) != name.strip_suffix(b"=") {
        return None;
    }
    Some((
        location.start_offset(),
        location.end_offset(),
        starts_uppercase(file, written),
    ))
}

fn starts_uppercase(file: &SourceFile, name: &[u8]) -> bool {
    !name.is_empty()
        && !matches!(name.last(), Some(b'?' | b'!' | b'='))
        && file
            .identifier_has_uppercase(&name[..file.encoded_character_width(name).min(name.len())])
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's naming/ascii_identifiers_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::pipeline::lint_str;
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        assert_eq!(
            crate::parser::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/AsciiIdentifiers")
            .map(|o| {
                assert!(o.correction.is_none());
                (o.line, o.col, o.message)
            })
            .collect()
    }
    #[test]
    fn pinned_identifiers_and_constants() {
        for (source, col, kind) in [
            ("älg = 1", 1, "identifiers"),
            ("foo∂∂bar = baz", 4, "identifiers"),
            ("class Foö\nend", 9, "constants"),
        ] {
            assert_eq!(
                found(source),
                [(1, col, format!("Use only ascii symbols in {kind}."))]
            );
        }
        for source in ["x.empty?", "\u{feff}\nputs 'foo'", "", "class Foo\nend"] {
            assert!(found(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn ternary_colon_does_not_turn_a_real_identifier_into_a_label() {
        assert_eq!(
            found("condition ? föö: bar")
                .into_iter()
                .map(|(l, c, _)| (l, c))
                .collect::<Vec<_>>(),
            [(1, 14)]
        );
        assert_eq!(found("föö = 1; foo(föö:)").len(), 1);
        assert!(found("case {}; in {föö:}; end").is_empty());
    }
    #[test]
    fn lexical_constant_and_method_token_kinds() {
        for (source, col, kind) in [
            ("def Foö?; end", 7, "identifiers"),
            ("def Foö=(value); end", 7, "identifiers"),
            ("def self.Foö; end", 12, "constants"),
            ("obj.Foö += 1", 7, "constants"),
            ("Foö::Bär ||= 1", 3, "constants"),
        ] {
            assert_eq!(
                found(source)[0],
                (1, col, format!("Use only ascii symbols in {kind}."))
            );
        }
        assert!(found("def self.föö?; end\nobj.föö!").is_empty());
        assert_eq!(found("Foö::Bär ||= 1").len(), 2);
    }
    #[test]
    fn bracket_messages_are_not_identifier_tokens() {
        for source in [
            "params[:عرض_gteq] # Arabic character (non-ascii)",
            "obj[:föö]",
            "obj[\"föö\"]",
            "obj[:föö] = 1",
            "obj[:föö], other = values",
            "obj[:föö] += 1",
            "obj[:föö] ||= 1",
            "obj[:föö] &&= 1",
            "obj.[](:föö)",
            "obj&.[](:föö)",
            "obj.[]=(:föö, 1)",
            "obj[\n  :föö\n]",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn index_children_keep_their_own_identifier_positions_once() {
        for (source, positions) in [
            ("obj[föö]", vec![(1, 6)]),
            ("föö[:bär]", vec![(1, 2)]),
            ("föö[bär]", vec![(1, 2), (1, 6)]),
            ("obj[föö] = bär", vec![(1, 6), (1, 13)]),
            ("obj[föö], other = bär", vec![(1, 6), (1, 20)]),
            ("obj[föö] += bär", vec![(1, 6), (1, 14)]),
            ("obj[föö] ||= bär", vec![(1, 6), (1, 15)]),
            ("obj[föö] &&= bär", vec![(1, 6), (1, 15)]),
            ("obj[\"föö#{bär}\"]", vec![(1, 12)]),
            ("obj[:\"föö#{bär}\"]", vec![(1, 13)]),
            ("obj[föö[:bär]]", vec![(1, 6)]),
            ("obj.[](föö)", vec![(1, 9)]),
            ("obj.[]=(föö, bär)", vec![(1, 10), (1, 15)]),
            ("obj&.[](föö)", vec![(1, 10)]),
            ("obj[\n  föö\n]", vec![(2, 4)]),
        ] {
            assert_eq!(
                found(source)
                    .into_iter()
                    .map(|(l, c, _)| (l, c))
                    .collect::<Vec<_>>(),
                positions,
                "{source}"
            );
        }
    }

    #[test]
    fn written_selectors_in_reads_setters_and_compound_targets_remain_identifiers() {
        for source in [
            "obj.föö",
            "obj.föö = 1",
            "obj.föö=(1)",
            "obj.föö, other = values",
            "obj.föö += 1",
            "obj.föö ||= 1",
            "obj.föö &&= 1",
            "obj&.föö = 1",
            "obj&.föö += 1",
        ] {
            let col = if source.starts_with("obj&.") { 7 } else { 6 };
            assert_eq!(
                found(source),
                [(1, col, "Use only ascii symbols in identifiers.".to_owned())],
                "{source}"
            );
        }
    }

    #[test]
    fn many_bracket_messages_do_not_rescan_or_duplicate_argument_names() {
        let source = "obj[:föö, :bär, föö]\n".repeat(2000);
        let actual = crate::pipeline::lint_str_with(&[&super::AsciiIdentifiers], &source);
        let expected: Vec<_> = (1..=2000).map(|line| (line, 18)).collect();
        assert_eq!(
            actual.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            expected
        );
        assert!(actual.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn regexp_capture_is_not_an_identifier_token() {
        assert!(found("/(?<föö>.)/ =~ s").is_empty());
    }
    #[test]
    fn actual_identifier_tokens_not_literal_or_sigil_names() {
        let source = "# föö\n'föö'; :föö; :\"föö\"; %i[föö]; {föö: 1}\n@föö = 1; @@föö = 1; $föö = 1\nföö = 1; föö; obj.föö; obj.föö?\ndef föö?; end\nalias föö bär\n";
        assert_eq!(
            found(source)
                .into_iter()
                .map(|(l, c, _)| (l, c))
                .collect::<Vec<_>>(),
            [(4, 2), (4, 11), (4, 20), (5, 6), (6, 8), (6, 12)]
        );
    }
}
