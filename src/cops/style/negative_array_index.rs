//! `Style/NegativeArrayIndex` — `a[a.length - n]` and its ranges.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::{collections::HashMap, ops::Range};
pub struct NegativeArrayIndex;
impl Cop for NegativeArrayIndex {
    fn name(&self) -> &'static str {
        "Style/NegativeArrayIndex"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (receiver, args) = if let Some(call) = node.as_call_node() {
            if call.name().as_slice() != b"[]" {
                return;
            }
            let Some(receiver) = call.receiver() else {
                return;
            };
            (receiver, ast::call_arguments(&call))
        } else if let Some(getter) = ast::compound_index_getter(node) {
            (getter.receiver, getter.arguments)
        } else {
            return;
        };
        let Some(index) = args.first() else {
            return;
        };
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        let facts = file.negative_array_index_facts(root);
        let receiver_source = text(&receiver);
        let mut offense = index.location();
        let inner_index = first_parenthesized_statement(index);
        let (prefer, current) = if let Some(range) =
            inner_index.as_ref().unwrap_or(index).as_range_node()
        {
            let (Some(start), Some(end)) = (range.left(), range.right()) else {
                return;
            };
            if !facts.preserving(&start) || !facts.preserving(&receiver) {
                return;
            }
            let inner_end = single_range_end_child(&end);
            let Some((Some(length_receiver), n)) = subtraction(inner_end.as_ref().unwrap_or(&end))
            else {
                return;
            };
            if length_receiver.location().as_slice() != receiver.location().as_slice() {
                return;
            }
            let op = if range.is_exclude_end() { "..." } else { ".." };
            let start_source = text(&start);
            let preferred = format!("{start_source}{op}-{n}");
            // Upstream preserves a begin/parentheses source, but uses the
            // unwrapped child's source for other one-child wrappers, even
            // semantically strange arrays/defined?/yield/super.
            let current_end = if end.as_parentheses_node().is_some() {
                &end
            } else {
                inner_end.as_ref().unwrap_or(&end)
            };
            let current = format!("{start_source}{op}{}", text(current_end));
            let outer = index.as_parentheses_node().is_some();
            offense = end.location();
            if outer {
                (format!("({preferred})"), format!("({current})"))
            } else {
                (preferred, current)
            }
        } else {
            let Some((length_receiver, n)) = subtraction(index) else {
                return;
            };
            let matched = if let Some(length_receiver) = length_receiver {
                facts.preserving(&receiver)
                    && facts.preserving(&length_receiver)
                    && (receiver.location().as_slice() == length_receiver.location().as_slice()
                        || facts.base(&receiver).is_some_and(|base| {
                            file.bytes[base]
                                == file.bytes[facts
                                    .base(&length_receiver)
                                    .unwrap_or_else(|| span(&length_receiver))]
                        }))
            } else {
                receiver.as_self_node().is_some()
            };
            if !matched {
                return;
            }
            (format!("-{n}"), text(index))
        };
        let (line, col) = file.line_col(offense.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `{receiver_source}[{prefer}]` instead of `{receiver_source}[{current}]`."
            ),
            line,
            col,
            end_line: file.last_line_of(offense),
            correction: cx.corrections_requested().then(|| Correction {
                range: span(index),
                replacement: prefer.clone(),
            }),
        });
    }
}
fn span(n: &Node<'_>) -> Range<usize> {
    n.location().start_offset()..n.location().end_offset()
}
fn text(n: &Node<'_>) -> String {
    String::from_utf8_lossy(n.location().as_slice()).into_owned()
}
// The pinned outer range projection selects the first statement, even when
// later statements determine the actual index value. Unlike
// range-end unwrapping below, it does not require a sole child.
fn first_parenthesized_statement<'pr>(n: &Node<'pr>) -> Option<Node<'pr>> {
    n.as_parentheses_node()
        .and_then(|p| p.body())
        .and_then(|b| b.as_statements_node()?.body().first())
}
/// The pinned range-end matcher unwraps exactly one *semantic* child, not
/// recursively nested grouping and not Prism's structural Arguments/Statements
/// containers. Only these wrappers can directly contain a subtraction send in
/// this expression position; sends also have receiver/name scalar children and
/// assignments have a name child, so counting AST nodes alone would be wrong.
fn single_range_end_child<'pr>(n: &Node<'pr>) -> Option<Node<'pr>> {
    fn sole<'pr>(nodes: ruby_prism::NodeList<'pr>) -> Option<Node<'pr>> {
        (nodes.len() == 1).then(|| nodes.first()).flatten()
    }
    match n {
        Node::ParenthesesNode { .. } => sole(
            n.as_parentheses_node()?
                .body()?
                .as_statements_node()?
                .body(),
        ),
        Node::ArrayNode { .. } => sole(n.as_array_node()?.elements()),
        Node::DefinedNode { .. } => Some(n.as_defined_node()?.value()),
        Node::BeginNode { .. } => {
            let begin = n.as_begin_node()?;
            if begin.rescue_clause().is_some()
                || begin.else_clause().is_some()
                || begin.ensure_clause().is_some()
            {
                return None;
            }
            sole(begin.statements()?.body())
        }
        Node::YieldNode { .. } => sole(n.as_yield_node()?.arguments()?.arguments()),
        Node::SuperNode { .. } => {
            let super_node = n.as_super_node()?;
            if super_node.block().is_some() {
                return None;
            }
            sole(super_node.arguments()?.arguments())
        }
        _ => None,
    }
}
fn subtraction<'pr>(n: &Node<'pr>) -> Option<(Option<Node<'pr>>, String)> {
    let minus = n.as_call_node()?;
    if minus.is_safe_navigation() || minus.name().as_slice() != b"-" || minus.block().is_some() {
        return None;
    }
    let args = ast::call_arguments(&minus);
    let [integer] = args.as_slice() else {
        return None;
    };
    let integer = integer.as_integer_node()?;
    let value = integer.value();
    let (negative, digits) = value.to_u32_digits();
    if negative || digits.iter().all(|d| *d == 0) {
        return None;
    }
    let length = minus.receiver()?.as_call_node()?;
    if length.is_safe_navigation()
        || length.block().is_some()
        || !matches!(length.name().as_slice(), b"length" | b"size" | b"count")
        || !ast::call_arguments(&length).is_empty()
    {
        return None;
    }
    // Prism exposes arbitrary-precision limbs; don't cap valid positive offsets
    // at a machine integer or preserve a nondecimal spelling in the message.
    let number = ast::unsigned_integer_decimal(digits);
    Some((length.receiver(), number))
}
/// One lazy bottom-up main-AST walk; preserving chains are classified once,
/// including rejected chains, rather than repeatedly traversing receiver trees.
type ReceiverFact = (bool, Option<Range<usize>>);
#[derive(Default)]
pub(crate) struct Facts {
    nodes: HashMap<(usize, usize), ReceiverFact>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        struct Index<'pr> {
            facts: Facts,
            stack: Vec<Node<'pr>>,
        }
        impl<'pr> NodeVisitor<'pr> for Index<'pr> {
            fn enter(&mut self, n: Node<'pr>) {
                self.stack.push(n);
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
            }
            fn leave(&mut self) {
                let n = self.stack.pop().unwrap();
                let (preserving, base) = if let Some(call) = n.as_call_node() {
                    if let Some(receiver) = call.receiver() {
                        (
                            // Upstream classifies preserving calls by receiver and
                            // method name, regardless of literal or passed blocks.
                            matches!(
                                call.name().as_slice(),
                                b"sort" | b"reverse" | b"shuffle" | b"rotate"
                            ) && self.facts.preserving(&receiver),
                            self.facts.base(&receiver).or_else(|| Some(span(&receiver))),
                        )
                    } else {
                        (true, None)
                    }
                } else {
                    (true, None)
                };
                if n.as_call_node().is_some() {
                    self.facts.nodes.insert(
                        (n.location().start_offset(), n.location().end_offset()),
                        (preserving, base),
                    );
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
    fn preserving(&self, n: &Node<'_>) -> bool {
        n.as_call_node().is_none()
            || self
                .nodes
                .get(&(n.location().start_offset(), n.location().end_offset()))
                .is_some_and(|f| f.0)
    }
    fn base(&self, n: &Node<'_>) -> Option<Range<usize>> {
        self.nodes
            .get(&(n.location().start_offset(), n.location().end_offset()))
            .and_then(|f| f.1.clone())
    }
}

pub(crate) static NEGATIVE_ARRAY_INDEX_FACTS: crate::source::Fact<Facts> =
    crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn negative_array_index_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        NEGATIVE_ARRAY_INDEX_FACTS.get(self, || Facts::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop negative_array_index_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == NegativeArrayIndex.name())
            .collect()
    }
    #[test]
    fn compound_assignment_getters_but_not_plain_setters() {
        for op in ["+=", "||=", "&&="] {
            for (getter, col, current, prefer) in [
                ("arr[arr.length - 1]", 5, "arr.length - 1", "-1"),
                (
                    "arr[0..(arr.length - 1)]",
                    8,
                    "0..(arr.length - 1)",
                    "0..-1",
                ),
                ("arr[0...arr.length - 1]", 9, "0...arr.length - 1", "0...-1"),
            ] {
                let s = format!("# header\n  {getter} {op} rhs\n");
                let o = crate::cops::testing::correction_offenses(&NegativeArrayIndex, &s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col + 2, 2));
                assert_eq!(
                    o[0].message,
                    format!("Use `arr[{prefer}]` instead of `arr[{current}]`.")
                );
                assert_eq!(o[0].correction.as_ref().unwrap().replacement, prefer);
                assert!(offenses(&format!("{getter} = rhs")).is_empty());
            }
            for getter in [
                "arr[other.length - 1]",
                "arr[0, arr.length - 1]",
                "arr[arr.length - 0]",
                "arr[0..other.length - 1]",
            ] {
                assert!(offenses(&format!("{getter} {op} rhs")).is_empty());
            }
        }
    }
    #[test]
    fn pinned_simple_receivers_and_preserving_chains() {
        for method in ["length", "size", "count"] {
            for (array, length) in [
                ("CONST", "CONST"),
                ("@arr", "@arr"),
                ("arr", "arr"),
                ("@@arr.sort", "@@arr"),
                ("arr.sort", "arr"),
                ("arr.sort", "arr.reverse"),
                ("arr.reverse", "arr.sort"),
                ("arr.shuffle", "arr.sort"),
                ("arr.rotate", "arr.reverse"),
                ("@arr.reverse", "@arr"),
                ("arr.sort.reverse", "arr.sort"),
                ("arr.sort.reverse", "arr.sort.reverse"),
            ] {
                let s = format!("{array}[{length}.{method} - 2]");
                let o = offenses(&format!("# header\n  {s}\n"));
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!(
                    (o[0].line, o[0].col, o[0].end_line),
                    (2, array.len() as u32 + 4, 2)
                );
                assert_eq!(o[0].message, format!("Use `{array}[-2]` instead of `{s}`."));
            }
        }
    }
    #[test]
    fn literal_blocks_on_preserving_receivers_and_length_receivers() {
        assert!(!NegativeArrayIndex.safe_fixes());
        for (array, length) in [
            ("arr.sort { |a, b| a <=> b }", "arr"),
            ("arr.reverse", "arr.sort { |a, b| a <=> b }"),
            ("arr.sort { |a, b| a <=> b }.reverse", "arr.reverse"),
            ("arr.reverse { ignored }", "arr"),
            ("arr.shuffle { ignored }", "arr"),
            ("arr.rotate { ignored }", "arr"),
            ("arr.sort do |a, b| a <=> b end", "arr"),
            ("factory { [] }", "factory { [] }"),
            ("factory { [] }.sort { |a, b| a <=> b }", "factory { [] }"),
        ] {
            for method in ["length", "size", "count"] {
                let s = format!("{array}[{length}.{method} - 1]");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!(
                    (o[0].line, o[0].col, o[0].end_line),
                    (1, array.len() as u32 + 2, 1)
                );
                assert_eq!(o[0].message, format!("Use `{array}[-1]` instead of `{s}`."));
                assert!(o[0].correction.is_none());
                let fixes = crate::cops::testing::correction_offenses(&NegativeArrayIndex, &s);
                assert_eq!(fixes.len(), 1, "{s}");
                assert!(fixes[0].correction.is_some());
            }
        }
    }
    #[test]
    fn literal_blocks_in_ranges_preserve_strict_receiver_matching() {
        for receiver in ["arr.sort { |a, b| a <=> b }", "factory { [] }"] {
            for start in ["0", "indices.sort { |a, b| a <=> b }"] {
                for op in ["..", "..."] {
                    let s = format!("{receiver}[{start}{op}({receiver}.length - 1)]");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!(
                        (o[0].line, o[0].col, o[0].end_line),
                        (1, (receiver.len() + start.len() + op.len() + 2) as u32, 1)
                    );
                    assert_eq!(
                        o[0].message,
                        format!("Use `{receiver}[{start}{op}-1]` instead of `{s}`.")
                    );
                    let fixes = crate::cops::testing::correction_offenses(&NegativeArrayIndex, &s);
                    assert_eq!(fixes.len(), 1, "{s}");
                    assert!(fixes[0].correction.is_some());
                }
            }
        }
        for s in [
            "arr.map { |x| x }[arr.length - 1]",
            "arr[arr.map { |x| x }.length - 1]",
            "arr[arr.sort { |a, b| a <=> b }.length - 1]",
            "arr.sort { |a, b| a <=> b }[0..(arr.length - 1)]",
            "arr.sort { |a, b| a <=> b }[0..(arr.sort { |a, b| b <=> a }.length - 1)]",
            "arr[indices.map { |x| x }..(arr.length - 1)]",
            "arr[arr.length { ignored } - 1]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn range_end_extracts_exactly_one_semantic_child() {
        assert!(!NegativeArrayIndex.safe_fixes());
        for wrapper in [
            "[EXPR]",
            "defined?(EXPR)",
            "begin EXPR end",
            "yield(EXPR)",
            "super(EXPR)",
            "(EXPR)",
        ] {
            for suffix in ["", " += rhs", " ||= rhs", " &&= rhs"] {
                for (prefix, col) in [("", 8), ("def f; ", 15)] {
                    // yield/super need a method context for valid Ruby.
                    if prefix.is_empty()
                        && (wrapper.starts_with("yield") || wrapper.starts_with("super"))
                    {
                        continue;
                    }
                    let end = wrapper.replace("EXPR", "arr.length - 1");
                    let s = format!(
                        "{prefix}arr[0..{end}]{suffix}{}",
                        if prefix.is_empty() { "" } else { ";end" }
                    );
                    let o = crate::cops::testing::correction_offenses(&NegativeArrayIndex, &s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, col, 1));
                    let current_end = if wrapper == "(EXPR)" {
                        end
                    } else {
                        "arr.length - 1".into()
                    };
                    assert_eq!(
                        o[0].message,
                        format!("Use `arr[0..-1]` instead of `arr[0..{current_end}]`.")
                    );
                    assert!(o[0].correction.is_some(), "{s}");
                }
            }
        }
        let o = offenses("arr.[](0..[arr.length - 1])");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col), (1, 11));
        assert_eq!(
            o[0].message,
            "Use `arr[0..-1]` instead of `arr[0..arr.length - 1]`."
        );
    }
    #[test]
    fn range_end_does_not_search_multiple_children_or_flatten_more_layers() {
        for end in [
            "[arr.length - 1, 0]",
            "[0, arr.length - 1]",
            "(arr.length - 1; 0)",
            "(0; arr.length - 1)",
            "begin arr.length - 1; 0 end",
            "begin 0; arr.length - 1 end",
            "[[arr.length - 1]]",
            "([arr.length - 1])",
            "[(arr.length - 1)]",
            "defined?([arr.length - 1])",
            "defined?((arr.length - 1))",
            "begin (arr.length - 1) end",
            "((arr.length - 1))",
            "[other.length - 1]",
            "[arr.length - 0]",
            "foo(arr.length - 1)",
            "\"#{arr.length - 1}\"",
            "[*arr.length - 1]",
            "(x = arr.length - 1)",
            "yield(arr.length - 1, 0)",
            "yield(0, arr.length - 1)",
            "super(arr.length - 1, 0)",
            "super(0, arr.length - 1)",
            "super(arr.length - 1, &b)",
            "super { arr.length - 1 }",
        ] {
            let s = format!("def f; arr[0..{end}]; end");
            assert_eq!(
                crate::parser::parse(s.as_bytes()).errors().count(),
                0,
                "{s}"
            );
            assert!(offenses(&s).is_empty(), "{s}");
        }
        for index in [
            "[arr.length - 1]",
            "defined?(arr.length - 1)",
            "begin arr.length - 1 end",
            "(arr.length - 1)",
        ] {
            assert!(offenses(&format!("arr[{index}]")).is_empty(), "{index}");
        }
    }
    #[test]
    fn outer_parentheses_extract_first_range_even_with_later_statements() {
        assert!(!NegativeArrayIndex.safe_fixes());
        for method in ["length", "size", "count"] {
            for op in ["..", "..."] {
                for end in [
                    format!("arr.{method} - 1"),
                    format!("(arr.{method} - 1)"),
                    format!("[arr.{method} - 1]"),
                ] {
                    for suffix in ["", " += rhs", " ||= rhs", " &&= rhs"] {
                        let source = format!("# header\n  arr[(0{op}{end}; 0; effect)]{suffix}\n");
                        assert_eq!(
                            crate::parser::parse(source.as_bytes()).errors().count(),
                            0,
                            "{source}"
                        );
                        let o =
                            crate::cops::testing::correction_offenses(&NegativeArrayIndex, &source);
                        assert_eq!(o.len(), 1, "{source}");
                        assert_eq!(
                            (o[0].line, o[0].col, o[0].end_line),
                            (2, (9 + op.len()) as u32, 2)
                        );
                        let current_end = if end.starts_with('[') {
                            format!("arr.{method} - 1")
                        } else {
                            end.clone()
                        };
                        assert_eq!(
                            o[0].message,
                            format!("Use `arr[(0{op}-1)]` instead of `arr[(0{op}{current_end})]`.")
                        );
                        assert!(o[0].correction.is_some(), "{source}");
                    }
                }
            }
        }
        for index in [
            "(0; 0..arr.length - 1)",
            "((0..arr.length - 1); 0)",
            "(0..(arr.length - 1; 0); 0)",
            "(0..(0; arr.length - 1); 0)",
            "(0..[arr.length - 1, 0]; 0)",
            "(0..other.length - 1; 0)",
            "(0..arr.length - 0; 0)",
        ] {
            let source = format!("arr[{index}]");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_ranges_and_parenthesis_positions() {
        for method in ["length", "size", "count"] {
            for receiver in [
                "arr",
                "CONST",
                "@arr",
                "$arr",
                "@@arr",
                "arr.reverse",
                "arr.sort.reverse",
            ] {
                for start in ["0", "-5", "n", "arr.sort.reverse"] {
                    for op in ["..", "..."] {
                        for outer in [false, true] {
                            for inner in [false, true] {
                                let end = format!("{receiver}.{method} - 2");
                                let end = if inner { format!("({end})") } else { end };
                                let range = format!("{start}{op}{end}");
                                let range = if outer { format!("({range})") } else { range };
                                let s = format!("{receiver}[{range}]");
                                let o = offenses(&s);
                                assert_eq!(o.len(), 1, "{s}");
                                assert_eq!(
                                    (o[0].line, o[0].col),
                                    (
                                        1,
                                        (receiver.len()
                                            + 2
                                            + usize::from(outer)
                                            + start.len()
                                            + op.len())
                                            as u32
                                    )
                                );
                                let prefer = format!("{start}{op}-2");
                                let prefer = if outer { format!("({prefer})") } else { prefer };
                                assert_eq!(
                                    o[0].message,
                                    format!("Use `{receiver}[{prefer}]` instead of `{s}`.")
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn pinned_bracket_sends_implicit_self_and_positive_integer_values() {
        for method in ["length", "size", "count"] {
            for (s, col, receiver, current, n) in [
                (
                    format!("arr.[](arr.{method} - 2)"),
                    8,
                    "arr",
                    format!("arr[arr.{method} - 2]"),
                    "2",
                ),
                (
                    format!("arr&.[](arr.{method} - 2)"),
                    9,
                    "arr",
                    format!("arr[arr.{method} - 2]"),
                    "2",
                ),
                (
                    format!("self[{method} - 1]"),
                    6,
                    "self",
                    format!("self[{method} - 1]"),
                    "1",
                ),
                (
                    format!("arr[arr.{method} - 0x10]"),
                    5,
                    "arr",
                    format!("arr[arr.{method} - 0x10]"),
                    "16",
                ),
            ] {
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col), (1, col));
                assert_eq!(
                    o[0].message,
                    format!("Use `{receiver}[-{n}]` instead of `{current}`.")
                );
            }
        }
    }
    #[test]
    fn pinned_no_offenses_and_ast_lookalikes() {
        for method in ["length", "size", "count"] {
            for s in [
                "arr[]".into(),
                "arr[()]".into(),
                "arr[-2]".into(),
                "arr[..-2]".into(),
                format!("arr[{method} - 2]"),
                format!("arr[other.{method} - 2]"),
                format!("arr[arr.reverse.{method} - 2]"),
                format!("arr[arr.{method} - 0]"),
                format!("arr[arr.{method} - (-2)]"),
                format!("arr[arr.{method} - n]"),
                format!("arr[arr.{method}(1) - 2]"),
                format!("arr[arr.{method} + 2]"),
                format!("arr[arr.{method} - 2] = value"),
                format!("arr[0, arr.{method} - 2]"),
                format!("arr[arr.map(&:to_s).{method} - 2]"),
                format!("arr.uniq[arr.uniq.{method} - 2]"),
                format!("arr.sort[(0..(arr.reverse.{method} - 2))]"),
                format!("arr[(arr.method..(arr.{method} - 2))]"),
                format!("arr[(0..(arr.{method} - n))]"),
                format!("arr[arr.{method} - 2 + 1]"),
                format!("{method} = 3; self[{method} - 1]"),
                format!("arr[arr&.{method} - 2]"),
            ] {
                assert!(offenses(&s).is_empty(), "{s}");
            }
        }
    }
    #[test]
    fn arbitrary_precision_positive_offsets_and_multiline_highlights() {
        for (literal, decimal) in [
            (
                "0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff",
                "1461501637330902918203684832716283019655932542975",
            ),
            ("0o100", "64"),
            ("0b100_000", "32"),
        ] {
            let s = format!("arr[arr.size - {literal}]");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                format!("Use `arr[-{decimal}]` instead of `{s}`.")
            );
        }
        let o = offenses("arr[(0..(arr.size -\n 2))]");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 9, 2));
    }

    #[test]
    fn risk_shapes_are_corrected_as_rubocop_does() {
        assert!(!NegativeArrayIndex.safe_fixes());
        for s in [
            "a = []; a[a.size - 2]",
            "a = [1]; a[a.size - 2]",
            "h = {0 => :x}; h[h.size - 1]",
            "arr.sort[arr.reverse.count - 1]",
            "[[]][[[]].size - 1]",
            "%w[a\\ b][%w[a\\ b].size - 1]",
        ] {
            let o = crate::cops::testing::correction_offenses(&NegativeArrayIndex, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_some());
        }
    }
}
