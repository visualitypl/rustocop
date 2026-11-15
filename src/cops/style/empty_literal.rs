//! `Style/EmptyLiteral` — prefer empty literals to exact core constructor shapes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense, single_line},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

#[derive(Default)]
pub struct EmptyLiteral {
    frozen_comment_enabled: bool,
    /// The style of Style/StringLiterals is `single_quotes`.
    single_quotes: bool,
}
impl Cop for EmptyLiteral {
    fn name(&self) -> &'static str {
        "Style/EmptyLiteral"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_frozen_string_comment(&self, enabled: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            frozen_comment_enabled: enabled,
            single_quotes: self.single_quotes,
        }))
    }
    fn with_single_quoted_strings(&self, single: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            frozen_comment_enabled: self.frozen_comment_enabled,
            single_quotes: single,
        }))
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let call = node.as_call_node();
        let (kind, start, end) = if let Some(call) = &call {
            let Some(kind) = candidate(call) else { return };
            (kind, call.location().start_offset(), ast::source_end(node))
        } else {
            let Some(parts) = compound_getter(node) else {
                return;
            };
            parts
        };
        if matches!(kind, Kind::String)
            && file
                .frozen_string_setting()
                .unwrap_or(self.frozen_comment_enabled)
        {
            return;
        }
        let current = single_line(&file.bytes[start..end]);
        let literal = kind.literal(self.single_quotes);
        let message = match kind {
            Kind::String => format!("Use string literal `{literal}` instead of `String.new`."),
            _ => format!(
                "Use {} literal `{literal}` instead of `{current}`.",
                kind.name()
            ),
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: call
                .as_ref()
                .filter(|_| cx.corrections_requested())
                .and_then(|call| correction(call, literal, kind, cx, file)),
        });
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Array,
    Hash,
    String,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Array => "array",
            Self::Hash => "hash",
            Self::String => "string",
        }
    }
    fn literal(self, single_quotes: bool) -> &'static str {
        match self {
            Self::Array => "[]",
            Self::Hash => "{}",
            Self::String => super::empty_heredoc::empty_string(single_quotes),
        }
    }
}
fn receiver_kind(receiver: &Node<'_>) -> Option<Kind> {
    if ast::is_global_const(receiver, &["Array"]) {
        Some(Kind::Array)
    } else if ast::is_global_const(receiver, &["Hash"]) {
        Some(Kind::Hash)
    } else if ast::is_global_const(receiver, &["String"]) {
        Some(Kind::String)
    } else {
        None
    }
}
fn compound_getter(node: &Node<'_>) -> Option<(Kind, usize, usize)> {
    macro_rules! call {
        ($access:ident) => {{
            let c = node.$access()?;
            if c.is_safe_navigation() || c.read_name().as_slice() != b"new" {
                return None;
            }
            let r = c.receiver()?;
            (
                receiver_kind(&r)?,
                r.location().start_offset(),
                c.message_loc()?.end_offset(),
            )
        }};
    }
    macro_rules! index {
        ($access:ident) => {{
            let c = node.$access()?;
            let getter = ast::compound_index_getter(node)?;
            if getter.safe_navigation || !getter.arguments.is_empty() {
                return None;
            }
            let kind = receiver_kind(&getter.receiver)?;
            if matches!(kind, Kind::String) {
                return None;
            }
            (
                kind,
                getter.receiver.location().start_offset(),
                c.closing_loc().end_offset(),
            )
        }};
    }
    Some(match node {
        Node::CallOperatorWriteNode { .. } => call!(as_call_operator_write_node),
        Node::CallOrWriteNode { .. } => call!(as_call_or_write_node),
        Node::CallAndWriteNode { .. } => call!(as_call_and_write_node),
        Node::IndexOperatorWriteNode { .. } => index!(as_index_operator_write_node),
        Node::IndexOrWriteNode { .. } => index!(as_index_or_write_node),
        Node::IndexAndWriteNode { .. } => index!(as_index_and_write_node),
        _ => return None,
    })
}
fn candidate(call: &CallNode<'_>) -> Option<Kind> {
    if call.is_safe_navigation() {
        return None;
    }
    let args = ast::call_arguments(call);
    let empty_array = matches!(args.as_slice(), [a] if a.as_array_node().is_some_and(|a| a.elements().is_empty()));
    let no_literal_block = call.block().is_none_or(|b| b.as_block_node().is_none());
    match (call.name().as_slice(), call.receiver()) {
        (b"new", Some(r))
            if ast::is_global_const(&r, &["Array"])
                && ((args.is_empty() && no_literal_block) || empty_array) =>
        {
            Some(Kind::Array)
        }
        (b"new", Some(r))
            if ast::is_global_const(&r, &["Hash"]) && args.is_empty() && no_literal_block =>
        {
            Some(Kind::Hash)
        }
        (b"new", Some(r)) if ast::is_global_const(&r, &["String"]) && args.is_empty() => {
            Some(Kind::String)
        }
        (b"[]", Some(r)) if args.is_empty() && ast::is_global_const(&r, &["Array"]) => {
            Some(Kind::Array)
        }
        (b"[]", Some(r)) if args.is_empty() && ast::is_global_const(&r, &["Hash"]) => {
            Some(Kind::Hash)
        }
        (b"Array", None) if empty_array => Some(Kind::Array),
        (b"Hash", None) if empty_array => Some(Kind::Hash),
        _ => None,
    }
}
/// Also used by the shared eager core-dispatch index. No arbitrary similarly
/// named method, passed block, or user expression is treated as a core API.
pub(super) fn primitive_api(call: &CallNode<'_>) -> bool {
    candidate(call).is_some() && call.block().is_none()
}
fn correction(
    call: &CallNode<'_>,
    literal: &str,
    kind: Kind,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let loc = call.location();
    let facts = file.format_string_facts(root);
    if call.block().is_some() || !facts.core_before(loc.start_offset()) || !facts.closed(&call.as_node())
        || loc.as_slice().contains(&b'\n')
        || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
        || file.heredoc_pending_at(root, loc.start_offset())
        // String.new is always ASCII-8BIT; a UTF-8 (or other encoded) literal
        // changes observable encoding even when empty. Keep the offense.
        || (matches!(kind, Kind::String) && (!file.binary_encoding() || file.frozen_string_setting() != Some(false)))
    {
        return None;
    }
    let mut edit = Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: literal.into(),
    };
    if matches!(kind, Kind::Hash) {
        let parent = cx.ancestors().find(|p| p.as_arguments_node().is_none());
        let command_args = parent.and_then(|p| {
            if let Some(c) = p.as_call_node().filter(|c| c.opening_loc().is_none()) {
                Some((c.message_loc()?.end_offset(), c.arguments()?))
            } else {
                let s = p.as_super_node().filter(|s| s.lparen_loc().is_none())?;
                Some((s.keyword_loc().end_offset(), s.arguments()?))
            }
        });
        // Inspect the first edge before traversing siblings. Only that single
        // candidate owns the command wrapper; other hashes cost O(1) each.
        if let Some((start, args)) = command_args.filter(|(_, a)| {
            a.arguments()
                .iter()
                .next()
                .is_some_and(|a| a.location().start_offset() == loc.start_offset())
        }) {
            let end = args.arguments().iter().last()?.location().end_offset();
            // Parentheses must touch the command selector: any remaining gap
            // makes `consume ({}, 1)` a grouped expression, not call arguments.
            if start >= loc.start_offset()
                || !file.bytes[start..loc.start_offset()]
                    .iter()
                    .all(|b| matches!(b, b' ' | b'\t'))
                || file.bytes[start..end].contains(&b'\n')
                || super::correction::contains_comment(file, start..end)
            {
                return None;
            }
            edit = Correction {
                range: start..end,
                replacement: format!(
                    "({{}}{})",
                    std::str::from_utf8(&file.bytes[loc.end_offset()..end]).ok()?
                ),
            };
        }
    }
    super::literal_spelling::safe_edit(file, cx, edit)
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop empty_literal_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed double quotes.
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/EmptyLiteral")
            .collect()
    }
    #[test]
    fn all_pinned_calls_and_corrections_are_accounted_for() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/empty_literal_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 51);
        let mut fixes = 0;
        let mut holds = 0;
        for e in cases.as_array().unwrap() {
            let s = e["source"].as_str().unwrap();
            let cop = EmptyLiteral {
                frozen_comment_enabled: e["frozen_comment"].as_bool().unwrap(),
                single_quotes: false,
            };
            let found = correction_offenses(&cop, s);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            let expected: Vec<Vec<u32>> = serde_json::from_value(e["positions"].clone()).unwrap();
            assert_eq!(positions, expected, "spec {}: {s}", e["spec_line"]);
            if let Some(after) = e["correction"].as_str() {
                match e["spec_line"].as_u64().unwrap() {
                    7 | 66 | 205 | 217 => {
                        assert_correction(&cop, s, after);
                        fixes += 1;
                    }
                    // Deferred/unknown dispatch (7), binary/chilled String (8).
                    32 | 138 | 149 | 166 | 177 | 189 | 229 | 248 | 259 | 270 | 294 | 305 | 338
                    | 397 | 419 => {
                        assert_correction(&cop, s, s);
                        holds += 1;
                    }
                    other => panic!("unclassified pinned correction {other}"),
                }
            }
        }
        assert_eq!((fixes, holds), (13, 15));
    }
    #[test]
    fn source_envelopes_preserve_bom_crlf_encoding_line_and_data_values() {
        for (s, after) in [
            (
                "\u{feff}Array.new\r\nHash.new\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
                "\u{feff}[]\r\n{}\r\n__LINE__\r\n__END__\r\npayload\0\r\n",
            ),
            (
                "# encoding: binary\r\n# frozen_string_literal: false\r\nString.new\r\n",
                "# encoding: binary\r\n# frozen_string_literal: false\r\n\"\"\r\n",
            ),
        ] {
            assert_correction(&EmptyLiteral::default(), s, after);
        }
    }
    #[test]
    fn exact_positions_messages_and_canonical_array_hash_fixes() {
        for (constructors, kind, literal) in [
            (
                vec![
                    "Array.new()",
                    "Array.new",
                    "::Array.new()",
                    "Array.new([])",
                    "Array[]",
                    "Array([])",
                    "::Array[]",
                    "Array.[]()",
                ],
                "array",
                "[]",
            ),
            (
                vec![
                    "Hash.new()",
                    "Hash.new",
                    "::Hash.new()",
                    "Hash[]",
                    "Hash([])",
                    "::Hash[]",
                    "Hash.[]()",
                ],
                "hash",
                "{}",
            ),
        ] {
            for constructor in constructors {
                let s = format!("# header\n  test = {constructor}\n");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 10, 2));
                assert_eq!(
                    o[0].message,
                    format!("Use {kind} literal `{literal}` instead of `{constructor}`.")
                );
                assert_correction(
                    &EmptyLiteral::default(),
                    &s,
                    &format!("# header\n  test = {literal}\n"),
                );
            }
        }
    }
    #[test]
    fn string_detection_and_binary_encoding_canonical_correction() {
        for constructor in ["String.new", "String.new()", "::String.new"] {
            let s = format!("  test = {constructor}\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1);
            assert_eq!((o[0].line, o[0].col), (1, 10));
            assert_eq!(
                o[0].message,
                "Use string literal `\"\"` instead of `String.new`."
            );
            assert!(
                correction_offenses(&EmptyLiteral::default(), &s)[0]
                    .correction
                    .is_none(),
                "String.new is binary, not UTF-8"
            );
            assert_correction(
                &EmptyLiteral::default(),
                &format!("# encoding: binary\n# frozen_string_literal: false\n{s}"),
                "# encoding: binary\n# frozen_string_literal: false\n  test = \"\"\n",
            );
        }
    }
    #[test]
    fn compound_getters_report_but_never_become_literal_assignment_targets() {
        for op in ["+=", "||=", "&&="] {
            for getter in [
                "Array.new",
                "::Array.new",
                "Hash.new",
                "String.new",
                "Array[]",
                "::Hash[]",
            ] {
                let s = format!("# h\n  {getter} {op} rhs\n");
                let o = correction_offenses(&EmptyLiteral::default(), &s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2));
                assert!(o[0].correction.is_none());
            }
        }
    }
    #[test]
    fn argument_binding_and_pinned_context_corrections() {
        for (s, after) in [
            ("puts { Array.new }", "puts { Array.new }"),
            ("puts { Hash.new }", "puts { Hash.new }"),
            (
                "test = Hash.new\nHash.new.merge(\"a\" => 3)\nyadayada.map { a }.reduce(Hash.new, :merge)\n",
                "test = Hash.new\nHash.new.merge(\"a\" => 3)\nyadayada.map { a }.reduce(Hash.new, :merge)\n",
            ),
            (
                "yadayada.map { a }.reduce Hash.new",
                "yadayada.map { a }.reduce Hash.new",
            ),
            (
                "yadayada.map { a }.reduce Hash.new, :merge",
                "yadayada.map { a }.reduce Hash.new, :merge",
            ),
            ("assert_equal Hash[], foo", "assert_equal({}, foo)"),
            ("puts Hash([])", "puts({})"),
            (
                "def foo\n  super Hash.new\nend",
                "def foo\n  super Hash.new\nend",
            ),
            (
                "assert_equal Hash[], effect(), &callback",
                "assert_equal Hash[], effect(), &callback",
            ),
            ("consume Hash.new, effect()", "consume({}, effect())"),
            ("consume(1, Hash.new)", "consume(1, {})"),
        ] {
            assert_correction(&EmptyLiteral::default(), s, after);
        }
    }
    #[test]
    fn single_quotes_of_string_literals_are_written_in_the_correction() {
        let cop = EmptyLiteral {
            frozen_comment_enabled: false,
            single_quotes: true,
        };
        let header = "# encoding: binary\n# frozen_string_literal: false\n";
        crate::cops::testing::assert_correction(
            &cop,
            &format!("{header}s = String.new\n"),
            &format!("{header}s = \'\'\n"),
        );
    }
    #[test]
    fn repeated_frozen_directives_use_the_last_applicable_parser_setting() {
        for (directives, frozen) in [
            (
                "# frozen_string_literal: false\n# frozen_string_literal: true\n",
                true,
            ),
            (
                "# frozen_string_literal: true\n# frozen_string_literal: false\n",
                false,
            ),
            (
                "# -*- frozen_string_literal: false; frozen_string_literal: true -*-\n",
                true,
            ),
            (
                "# -*- frozen_string_literal: true; frozen_string_literal: false -*-\n",
                false,
            ),
            (
                "# frozen_string_literal: false\n# frozen_string_literal: invalid\n",
                false,
            ),
        ] {
            let source = format!("# encoding: binary\n{directives}s = String.new\n");
            let o = correction_offenses(&EmptyLiteral::default(), &source);
            assert_eq!(o.len(), usize::from(!frozen), "{source}");
            if !frozen {
                assert_correction(
                    &EmptyLiteral::default(),
                    &source,
                    &source.replace("String.new", "\"\""),
                );
            }
        }
        // A directive after tokens is ignored, even if it is the last comment.
        let source = "# encoding: binary\n# frozen_string_literal: false\ns = String.new\n# frozen_string_literal: true\n";
        assert_correction(
            &EmptyLiteral::default(),
            source,
            &source.replace("String.new", "\"\""),
        );
    }
    #[test]
    fn command_hash_wrapping_replaces_the_complete_horizontal_separator() {
        for separator in ["  ", "\t\t", " \t "] {
            for head in ["consume", "assert_equal"] {
                let source = format!("{head}{separator}Hash.new, 1");
                assert_correction(
                    &EmptyLiteral::default(),
                    &source,
                    &format!("{head}({{}}, 1)"),
                );
            }
        }
    }
    #[test]
    fn frozen_headers_chilled_strings_and_encoding_are_not_conflated() {
        for header in [
            "# frozen_string_literal: true",
            "# -*- frozen_string_literal: true -*-",
            "\u{feff}# frozen_string_literal: true",
            "#!/usr/bin/ruby\n# frozen_string_literal: true",
        ] {
            assert!(offenses(&format!("{header}\nString.new\n")).is_empty());
            assert_eq!(
                offenses(&format!("{header}\nArray.new\nHash.new\n")).len(),
                2
            );
        }
        for s in [
            "# encoding: binary\nString.new",
            "# frozen_string_literal: false\nString.new",
            "# encoding: us-ascii\n# frozen_string_literal: false\nString.new",
            "# encoding: windows-31j\n# frozen_string_literal: false\nString.new",
        ] {
            let o = correction_offenses(&EmptyLiteral::default(), s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn uncertain_dispatch_scope_allocation_blocks_and_source_reads_withhold_not_hide() {
        for s in [
            "Array = Other; Array.new",
            "module M; Array = Other; Array.new; end",
            "class Base; Array = Other; end; class M < Base; Array.new; end",
            "class Array; def self.new; effect; end; end; Array.new",
            "def Array(x); nil; end; Array([])",
            "def Hash(x); false; end; Hash([])",
            "def f; Hash.new; end",
            "require 'other'; Hash.new",
            "Array.new; unknown; Hash.new",
            "module R; refine Array.singleton_class do; def new; nil; end; end; end; using R; Array.new",
            "Array.new([]) { effect }",
            "Array[] { effect }",
            "String.new { effect }",
            "Array.new( # keep\n [])",
            "consume(<<~TEXT, Hash.new)\nx\nTEXT\n",
            "Array.new\np File.binread(__FILE__)\n",
            "Array.new\np ObjectSpace.each_object(Array).to_a\n",
            "Hash.new\np DATA.pos\n__END__\npayload\n",
            "Array.new\np RubyVM::InstructionSequence.compile_file(__FILE__)\n",
        ] {
            let o = correction_offenses(&EmptyLiteral::default(), s);
            assert!(!o.is_empty(), "{s}");
            assert!(o.last().unwrap().correction.is_none(), "{s}");
        }
    }
    #[test]
    fn lookalikes_arguments_blocks_and_safe_navigation() {
        for s in [
            "[]",
            "{}",
            "Array.new(3)",
            "Array[3]",
            "Array [3]",
            "Hash[3,4]",
            "Hash [3,4]",
            "Hash.new(3)",
            "::Hash.new(3)",
            "Hash.new([])",
            "String.new(\"top\")",
            "::String.new(\"top\")",
            "Array.new { 1 }",
            "::Array.new { 1 }",
            "puts { Array.new { 1 } }",
            "Hash.new { block }",
            "::Hash.new { _1[_2] = [] }",
            "Array.new { _1 }",
            "Array.new { it }",
            "Hash.new { it }",
            "Other::Array.new",
            "Other::Hash[]",
            "factory.new",
            "self.Array([])",
            "Kernel.Hash([])",
            "Array(nil)",
            "Hash(false)",
            "Array.new(false)",
            "Hash.new(nil)",
            "new",
            "Array&.new",
            "Hash&.[]()",
            "String&.new",
            "Array.new(&block)",
            "Array([], &block)",
            "Hash(**{})",
            "Array.new(([]))",
            "Array.new([effect])",
            "Array([nil])",
            "Hash([false])",
            "Array[] = rhs",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
