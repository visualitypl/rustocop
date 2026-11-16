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
                .and_then(|call| correction(call, literal, kind, cx)),
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
/// RuboCop's corrector. A hash that is the first argument of a call
/// without parentheses gets them, with the other arguments: `{}` there
/// would be a block.
fn correction(
    call: &CallNode<'_>,
    literal: &str,
    kind: Kind,
    cx: &NodeContext<'_, '_>,
) -> Option<Correction> {
    let start = call.location().start_offset();
    let end = ast::source_end(&call.as_node());
    if matches!(kind, Kind::Hash) {
        let parent = cx.ancestors().find(|p| p.as_arguments_node().is_none());
        let arguments = parent.and_then(|p| {
            if let Some(c) = p.as_call_node().filter(|c| c.opening_loc().is_none()) {
                c.arguments()
            } else {
                p.as_super_node()
                    .filter(|s| s.lparen_loc().is_none())?
                    .arguments()
            }
        });
        if let Some(arguments) = arguments.filter(|a| {
            a.arguments()
                .iter()
                .next()
                .is_some_and(|a| a.location().start_offset() == start)
        }) {
            let rest = arguments
                .arguments()
                .iter()
                .skip(1)
                .map(|a| std::str::from_utf8(a.location().as_slice()).ok())
                .collect::<Option<Vec<_>>>()?;
            let last = arguments.arguments().iter().last()?.location().end_offset();
            return Some(Correction {
                range: start.checked_sub(1)?..last,
                replacement: format!(
                    "({})",
                    std::iter::once("{}")
                        .chain(rest)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
    }
    Some(Correction {
        range: start..end,
        replacement: literal.into(),
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop empty_literal_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed double quotes.
    use super::*;
    #[test]
    fn corrects_as_rubocop() {
        for (before, after) in [
            ("puts { Array.new }\n", "puts { [] }\n"),
            ("puts Hash.new, 1\n", "puts({}, 1)\n"),
            ("x = Hash.new\n", "x = {}\n"),
        ] {
            crate::cops::testing::assert_correction(&EmptyLiteral::default(), before, after);
        }
    }
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/EmptyLiteral")
            .collect()
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
