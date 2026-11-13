//! `Style/FormatString` — fixed `format` style.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::collections::{HashMap, HashSet};
pub struct FormatString;
impl Cop for FormatString {
    fn name(&self) -> &'static str {
        "Style/FormatString"
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
        if !formatter(&call) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let current = if call.name().as_slice() == b"%" {
            "String#%"
        } else {
            "sprintf"
        };
        let correction = cx
            .corrections_requested()
            .then(|| correction(&call, cx, file))
            .flatten();
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Favor `format` over `{current}`."),
            line,
            col,
            end_line: file.line_of(selector.end_offset().saturating_sub(1)),
            correction,
        });
    }
}
fn string(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::StringNode { .. } | Node::InterpolatedStringNode { .. }
    )
}
fn formatter(call: &CallNode<'_>) -> bool {
    if call.is_safe_navigation() {
        return false;
    }
    let args = ast::call_arguments(call);
    match (call.name().as_slice(), call.receiver()) {
        (b"sprintf", None) => args.len() >= 2,
        (b"%", Some(receiver)) => {
            string(&receiver)
                || matches!(args.as_slice(), [n] if n.as_array_node().is_some() || n.as_hash_node().is_some() || n.as_keyword_hash_node().is_some())
        }
        _ => false,
    }
}
fn correction(
    call: &CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let facts = file.format_string_facts(root);
    let loc = call.location();
    let args = ast::call_arguments(call);
    if !facts.core_before(loc.start_offset()) || call.block().is_some() || !args.iter().all(|n| facts.closed(n))
        // Heredoc bodies execute at their header's evaluation position, not at
        // their physical offsets. Do not use source-order dispatch facts inside
        // a pending body or for subsequent same-header arguments.
        || file.heredoc_pending_at(root, loc.start_offset())
    {
        return None;
    }
    let edit = if call.name().as_slice() == b"sprintf" {
        Correction {
            range: call.message_loc()?.start_offset()..call.message_loc()?.end_offset(),
            replacement: "format".into(),
        }
    } else {
        let receiver = call.receiver()?;
        // An arbitrary receiver may implement numeric/custom %, not String#%.
        if !string(&receiver)
            || !facts.closed(&receiver)
            || args.len() != 1
            || file.contains_heredoc(root, &call.as_node())
            || super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
            || loc.as_slice().contains(&b'\n')
        {
            return None;
        }
        let rhs = &args[0];
        let range = if let Some(array) = rhs.as_array_node() {
            // Literal arrays spread; never wrap the array as a single argument.
            let open = array.opening_loc()?;
            if open.as_slice() != b"[" {
                return None;
            }
            open.end_offset()..array.closing_loc()?.start_offset()
        } else if let Some(hash) = rhs.as_hash_node() {
            // Keep named fields as a single keyword Hash, including empty hashes.
            hash.opening_loc().end_offset()..hash.closing_loc().start_offset()
        } else if rhs.as_keyword_hash_node().is_some() {
            rhs.location().start_offset()..rhs.location().end_offset()
        } else {
            if !scalar(rhs) {
                return None;
            }
            rhs.location().start_offset()..rhs.location().end_offset()
        };
        // Preserve exact scalar tokens and nonempty container interiors. Even
        // ASCII whitespace may be token data: ?\<space> or %q<space>x<space>.
        // Only the main AST can distinguish an empty container from its trivia.
        let empty = rhs
            .as_array_node()
            .is_some_and(|array| array.elements().is_empty())
            || rhs
                .as_hash_node()
                .is_some_and(|hash| hash.elements().is_empty());
        let value = std::str::from_utf8(&file.bytes[range]).ok()?;
        let receiver = std::str::from_utf8(receiver.location().as_slice()).ok()?;
        let replacement = if empty {
            if rhs.as_hash_node().is_some() {
                format!("format({receiver}, {{}})")
            } else {
                format!("format({receiver})")
            }
        } else {
            format!("format({receiver}, {value})")
        };
        Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement,
        }
    };
    super::literal_spelling::safe_edit(file, cx, edit)
}
fn scalar(n: &Node<'_>) -> bool {
    string(n)
        || matches!(
            n,
            Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::RationalNode { .. }
                | Node::ImaginaryNode { .. }
                | Node::SymbolNode { .. }
                | Node::NilNode { .. }
                | Node::TrueNode { .. }
                | Node::FalseNode { .. }
        )
}
/// Only primitive String trim/alias sends. This is not conversion or local
/// type flow. Chains are bounded by the pipeline's 64-node AST path budget.
fn primitive_string_api(call: &CallNode<'_>) -> bool {
    if call.block().is_some() || call.arguments().is_some_and(|a| !a.arguments().is_empty()) {
        return false;
    }
    let Some(mut receiver) = call.receiver() else {
        return false;
    };
    if matches!(call.name().as_slice(), b"intern" | b"to_sym")
        && receiver.as_symbol_node().is_some()
    {
        return true;
    }
    if !matches!(
        call.name().as_slice(),
        b"intern" | b"to_sym" | b"lstrip" | b"rstrip" | b"strip"
    ) {
        return false;
    }
    for _ in 0..crate::cops::depth::MAX_AST_DEPTH {
        if receiver.as_string_node().is_some() {
            return true;
        }
        let Some(inner) = receiver.as_call_node() else {
            return false;
        };
        if !matches!(inner.name().as_slice(), b"lstrip" | b"rstrip" | b"strip")
            || inner.block().is_some()
            || inner.arguments().is_some_and(|a| !a.arguments().is_empty())
        {
            return false;
        }
        let Some(next) = inner.receiver() else {
            return false;
        };
        receiver = next;
    }
    false
}
fn key(n: &Node<'_>) -> (usize, usize) {
    (n.location().start_offset(), n.location().end_offset())
}
/// Closed eager core-dispatch proof, built lazily once from the main parse.
/// Unknown completed calls invalidate later candidates. Declarations, scopes,
/// hooks and unsupported executable syntax invalidate globally. No value-flow,
/// sibling rescans, candidate reparses or assumed conversion return types.
#[derive(Default)]
pub(crate) struct Facts {
    closed: HashMap<(usize, usize), bool>,
    named_keys: HashMap<(usize, usize), HashSet<Vec<u8>>>,
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        struct Index {
            facts: Facts,
            stack: Vec<(NodeKey, bool, bool, Option<NodeKey>)>,
        }
        type NodeKey = (usize, usize);
        impl<'pr> NodeVisitor<'pr> for Index {
            fn enter(&mut self, n: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                // Only the exact receiver edge of a trusted core API may read
                // its Kernel/Array constant. Arbitrary constant values stay open.
                let core_receiver = self
                    .stack
                    .last()
                    .is_some_and(|(_, _, _, receiver)| *receiver == Some(key(&n)));
                let container = core_receiver
                    || matches!(
                        n,
                        Node::ProgramNode { .. }
                            | Node::StatementsNode { .. }
                            | Node::ArgumentsNode { .. }
                            | Node::ArrayNode { .. }
                            | Node::HashNode { .. }
                            | Node::KeywordHashNode { .. }
                            | Node::AssocNode { .. }
                            | Node::EmbeddedStatementsNode { .. }
                            | Node::ParenthesesNode { .. }
                    );
                let elements = n
                    .as_hash_node()
                    .map(|h| h.elements())
                    .or_else(|| n.as_keyword_hash_node().map(|h| h.elements()));
                if let Some(elements) = elements {
                    let keys = elements
                        .iter()
                        .filter_map(|e| {
                            let key = e.as_assoc_node()?.key();
                            Some(key.as_symbol_node()?.unescaped().to_vec())
                        })
                        .collect();
                    self.facts.named_keys.insert(key(&n), keys);
                }
                let call = n.as_call_node();
                let trusted_call = call.as_ref().is_some_and(|c| {
                    primitive_string_api(c)
                        || super::array_join::primitive_api(c)
                        || super::sample::primitive_api(c)
                        || super::redundant_array_constructor::primitive_api(c)
                        || super::empty_literal::primitive_api(c)
                        || super::unpack_first::primitive_api(c)
                        || (!c.is_safe_navigation()
                            && c.block().is_none()
                            && ((c
                                .receiver()
                                .is_none_or(|r| ast::is_global_const(&r, &["Kernel"]))
                                && matches!(c.name().as_slice(), b"sprintf" | b"format"))
                                || (c.name().as_slice() == b"%"
                                    && c.receiver().is_some_and(|r| string(&r)))))
                });
                let local_write = n.as_local_variable_write_node();
                self.facts.changed |= local_write
                    .as_ref()
                    .is_some_and(|w| w.name().as_slice() == b"format")
                    || !(scalar(&n)
                        || container
                        || call.is_some()
                        || local_write.is_some()
                        || n.as_local_variable_read_node().is_some()
                        || n.as_source_line_node().is_some());
                self.stack.push((
                    key(&n),
                    scalar(&n) || container || trusted_call,
                    call.is_some()
                        || n.as_interpolated_string_node().is_some()
                        || n.as_embedded_statements_node().is_some(),
                    call.as_ref()
                        .filter(|_| trusted_call)
                        .and_then(|c| c.receiver())
                        .filter(|r| {
                            ast::is_global_const(r, &["Kernel"])
                                || ast::is_global_const(r, &["Array"])
                                || ast::is_global_const(r, &["Hash"])
                                || ast::is_global_const(r, &["String"])
                        })
                        .map(|r| key(&r)),
                ));
            }
            fn leave(&mut self) {
                let (key, closed, effect, _) = self.stack.pop().unwrap();
                if effect && !closed {
                    self.facts.first_effect =
                        Some(self.facts.first_effect.map_or(key.1, |old| old.min(key.1)));
                }
                self.facts.closed.insert(key, closed);
                if let Some((_, parent, _, _)) = self.stack.last_mut() {
                    *parent &= closed;
                }
            }
        }
        let mut index = Index {
            facts: Self::default(),
            stack: Vec::new(),
        };
        walk::walk(&mut index, root);
        index.facts
    }
    pub(crate) fn named_keys(&self, hash: &Node<'_>) -> Option<&HashSet<Vec<u8>>> {
        self.named_keys.get(&key(hash))
    }
    pub(crate) fn closed(&self, n: &Node<'_>) -> bool {
        self.closed.get(&key(n)).copied().unwrap_or(false)
    }
    pub(crate) fn core_before(&self, start: usize) -> bool {
        !self.changed && self.first_effect.is_none_or(|end| end > start)
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop format_string_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed format style only.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == FormatString.name())
            .collect()
    }
    #[test]
    fn pinned_selector_positions_and_messages() {
        for (s, col, current) in [
            ("puts \"%d\" % 10", 11, "String#%"),
            ("puts x % [10, 11]", 8, "String#%"),
            ("puts x % { a: 10, b: 11 }", 8, "String#%"),
            ("puts \"#{x * 5} %d #{@test}\" % 10", 29, "String#%"),
            ("sprintf(something, a, b)", 1, "sprintf"),
            ("sprintf('%020d', 123)", 1, "sprintf"),
        ] {
            let o = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col + 2, 2), "{s}");
            assert_eq!(o[0].message, format!("Favor `format` over `{current}`."));
        }
    }
    #[test]
    fn pinned_unknown_values_and_conversions_still_report() {
        for rhs in [
            "a",
            "@pair",
            "@@pair",
            "$pair",
            "PAIR",
            "Foo::PAIR",
            "(pair)",
            "pair&.foo",
            "a.to_s",
            "a.to_h",
            "a.to_i",
            "a.to_f",
            "a.to_r",
            "a.to_d",
            "a.to_sym",
            "a&.to_s",
        ] {
            let s = format!("puts \"%s\" % {rhs}");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, 11));
        }
        assert_eq!(offenses("a = something()\nputs \"%d\" % a")[0].line, 2);
    }
    #[test]
    fn pinned_and_structural_no_offense_lookalikes() {
        for s in [
            "puts 10 % 4",
            "puts x % Y",
            "format(something, a, b)",
            "sprintf :xml",
            "sprintf",
            "object.sprintf('%s', 1)",
            "Kernel.sprintf('%s', 1)",
            "::Kernel.sprintf('%s', 1)",
            "object&.sprintf('%s', 1)",
            "x % value",
            "x&.%(value)",
            "'%s'&.%(1)",
            "`no` % 1",
            "sprintf = 1; sprintf",
            "def sprintf(a, b); end",
            "# sprintf('%s', 1)\n'sprintf'",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn canonical_scalar_array_hash_and_selector_corrections() {
        for (before, after) in [
            ("puts \"%d\" % 10", "puts format(\"%d\", 10)"),
            ("sprintf('%020d', 123)", "format('%020d', 123)"),
            ("\"%s %d\" % [\"x\", 2]", "format(\"%s %d\", \"x\", 2)"),
            ("\"%<a>d\" % { a: 10 }", "format(\"%<a>d\",  a: 10 )"),
            ("\"%s\" % nil", "format(\"%s\", nil)"),
        ] {
            assert_correction(&FormatString, before, after);
        }
    }
    #[test]
    fn unicode_whitespace_in_bare_symbols_is_token_content() {
        for suffix in [
            '\u{0085}', '\u{00a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}',
            '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}',
            '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
        ] {
            let symbol = format!(":foo{suffix}");
            for (before, after) in [
                (
                    format!("\"%s\" % {symbol}"),
                    format!("format(\"%s\", {symbol})"),
                ),
                (
                    format!("\"%s\" % [{symbol}]"),
                    format!("format(\"%s\", {symbol})"),
                ),
                (
                    format!("\"%s\" % [ \t{symbol} \t]"),
                    format!("format(\"%s\",  \t{symbol} \t)"),
                ),
                (
                    format!("\"%{{a}}\" % {{a: {symbol}}}"),
                    format!("format(\"%{{a}}\", a: {symbol})"),
                ),
                (
                    format!("\"%{{a}}\".%(a: {symbol})"),
                    format!("format(\"%{{a}}\", a: {symbol})"),
                ),
            ] {
                assert_correction(&FormatString, &before, &after);
            }
        }
    }
    #[test]
    fn ascii_whitespace_token_endings_and_container_interiors_are_exact() {
        let mut tokens = vec![];
        for value in [' ', '\t', '\x0b', '\x0c', '\r'] {
            tokens.push(format!("?\\{value}"));
        }
        for delimiter in [' ', '\t'] {
            tokens.push(format!("%q{delimiter}hello{delimiter}"));
            tokens.push(format!("%s{delimiter}foo{delimiter}"));
        }
        for token in tokens {
            for (before, after) in [
                (
                    format!("\"%s\" % {token}"),
                    format!("format(\"%s\", {token})"),
                ),
                (
                    format!("\"%s\" % [{token}]"),
                    format!("format(\"%s\", {token})"),
                ),
                (
                    format!("\"%s/%s\" % [1, {token}]"),
                    format!("format(\"%s/%s\", 1, {token})"),
                ),
                (
                    format!("\"%s\" % {{x: {token}}}"),
                    format!("format(\"%s\", x: {token})"),
                ),
                (
                    format!("\"%{{x}}\" % {{a: 1, x: {token}}}"),
                    format!("format(\"%{{x}}\", a: 1, x: {token})"),
                ),
                (
                    format!("\"%{{x}}\".%(x: {token})"),
                    format!("format(\"%{{x}}\", x: {token})"),
                ),
                (
                    format!("\"%s\" % [ \t{token} \t]"),
                    format!("format(\"%s\",  \t{token} \t)"),
                ),
                (
                    format!("\"%{{x}}\" % {{ \tx: {token} \t}}"),
                    format!("format(\"%{{x}}\",  \tx: {token} \t)"),
                ),
            ] {
                assert_correction(&FormatString, &before, &after);
            }
        }
    }
    #[test]
    fn empty_containers_are_classified_by_ast_not_source_whitespace() {
        for interior in ["", " ", "\t", " \t\x0b\x0c\r "] {
            assert_correction(
                &FormatString,
                &format!("\"%%\" % [{interior}]"),
                "format(\"%%\")",
            );
            assert_correction(
                &FormatString,
                &format!("\"%s\" % {{{interior}}}"),
                "format(\"%s\", {})",
            );
        }
    }
    #[test]
    fn every_applicable_pinned_correction_pair() {
        // 14 calls: 2 apply, 12 intentionally withheld, independently of oracle detection.
        let mut pairs = vec![
            (
                "puts \"%d\" % 10".to_string(),
                "puts format(\"%d\", 10)".to_string(),
                true,
            ),
            (
                "puts x % [10, 11]".into(),
                "puts format(x, 10, 11)".into(),
                false,
            ),
            (
                "puts x % { a: 10, b: 11 }".into(),
                "puts format(x, a: 10, b: 11)".into(),
                false,
            ),
            (
                "puts \"%s\" % a&.to_s".into(),
                "puts format(\"%s\", a&.to_s)".into(),
                false,
            ),
            (
                "puts \"#{x * 5} %d #{@test}\" % 10".into(),
                "puts format(\"#{x * 5} %d #{@test}\", 10)".into(),
                false,
            ),
            (
                "sprintf(something, a, b)".into(),
                "format(something, a, b)".into(),
                false,
            ),
            (
                "sprintf('%020d', 123)".into(),
                "format('%020d', 123)".into(),
                true,
            ),
        ];
        for method in ["to_s", "to_h", "to_i", "to_f", "to_r", "to_d", "to_sym"] {
            pairs.push((
                format!("puts \"%s\" % a.{method}"),
                format!("puts format(\"%s\", a.{method})"),
                false,
            ));
        }
        assert_eq!(pairs.len(), 14);
        for (before, after, applies) in pairs {
            if applies {
                assert_correction(&FormatString, &before, &after);
            } else {
                let o = correction_offenses(&FormatString, &before);
                assert_eq!(o.len(), 1, "{before} -> {after}");
                assert!(o[0].correction.is_none(), "{before} -> {after}");
            }
        }
    }
    #[test]
    fn percent_argument_and_receiver_shapes() {
        for s in [
            "\"%s\".%()",
            "\"%s\".%(1, 2)",
            "\"%s\".%(1, &block)",
            "sprintf('%s', &block)",
            "x.%([1])",
            "x.%(a: 1)",
            "\"%s\" % %w[a b]",
            "\"%s\" % %i[a b]",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "x.%([1], 2)",
            "x.%([1], &block)",
            "x&.%([1])",
            "sprintf('%s') { 1 }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn array_spread_hash_packaging_escapes_and_precedence() {
        for (before, after) in [
            (
                "\"%s/%s\" % [[1, 2], \"x\"]",
                "format(\"%s/%s\", [1, 2], \"x\")",
            ),
            ("\"%%\" % []", "format(\"%%\")"),
            ("\"%s\" % {}", "format(\"%s\", {})"),
            (
                "\"%{a}\" % { a: \"\\u00e9\" }",
                "format(\"%{a}\",  a: \"\\u00e9\" )",
            ),
            (
                "\"%2$s/%1$s/%%\" % [\"a\", \"b\"]",
                "format(\"%2$s/%1$s/%%\", \"a\", \"b\")",
            ),
            ("(\"%s\" % 1).size", "(format(\"%s\", 1)).size"),
            ("\"x\" + \"%s\" % 1", "\"x\" + format(\"%s\", 1)"),
            ("\"%s\" % 1 == \"1\"", "format(\"%s\", 1) == \"1\""),
            ("\"#{1}:%s\" % false", "format(\"#{1}:%s\", false)"),
        ] {
            assert_correction(&FormatString, before, after);
        }
    }
    #[test]
    fn nonordinary_array_tokens_and_effectful_interpolation_withhold() {
        for s in [
            "\"%s\" % %w[a b]",
            "\"%s\" % %i[a b]",
            "\"%s\" % [*a]",
            "\"%s\" % { **a }",
            "obj = nil; \"#{obj} #{\"%s\" % 1}\"",
            "obj = nil; \"#{obj} #{sprintf('%s', 1)}\"",
            "\"%s\".%(1, 2)",
            "\"%s\".%(1) { 2 }",
            "\"%s\".%(1, &block)",
            "sprintf('%s', 1) { 2 }",
            "sprintf('%s', *a)",
            "\"%s\" % (1 rescue 2)",
            "\"%s\" \\\n % 1",
            "\"%s\" % [1, # keep\n2]",
        ] {
            let o = correction_offenses(&FormatString, s);
            assert!(!o.is_empty(), "{s}");
            assert!(o.iter().all(|o| o.correction.is_none()), "{s}: {o:?}");
        }
    }
    #[test]
    fn source_visible_override_alias_refinement_load_and_lexical_barriers() {
        for prefix in [
            "def format(*x); 'other'; end",
            "def sprintf(*x); 'other'; end",
            "class String; def %(*x); 'other'; end; end",
            "class Object; def to_s; []; end; end",
            "alias format puts",
            "alias sprintf puts",
            "require 'extension'",
            "load 'extension.rb'",
            "eval('def format(*x); end')",
            "using Extension",
            "format = nil",
            "module M; refine String do; def %(*x); nil; end; end; end",
            "at_exit { nil }",
            "effect()",
        ] {
            for body in ["\"%s\" % 1", "sprintf('%s', 1)"] {
                let s = format!("{prefix}\n{body}");
                let o = correction_offenses(&FormatString, &s);
                assert_eq!(o.len(), 1, "{s}");
                assert!(o[0].correction.is_none(), "{s}");
            }
        }
        for s in [
            "def f; \"%s\" % 1; end",
            "items.each { \"%s\" % 1 }",
            "sprintf('%s', mutate())",
            "\"%s\" % a.to_s",
        ] {
            let o = correction_offenses(&FormatString, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn uncertain_values_remain_uncorrected() {
        for s in [
            "puts x % [10, 11]",
            "puts x % { a: 10, b: 11 }",
            "puts \"%s\" % a.to_s",
            "puts \"%s\" % a&.to_s",
            "puts \"%s\" % a",
            "a = something()\nputs \"%d\" % a",
            "puts \"#{x * 5} %d #{@test}\" % 10",
            "sprintf(something, a, b)",
        ] {
            let o = correction_offenses(&FormatString, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
}
