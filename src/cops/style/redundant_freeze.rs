//! `Style/RedundantFreeze` — immutable literals and pinned immutable-result idioms.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;
pub struct RedundantFreeze;
impl Cop for RedundantFreeze {
    fn name(&self) -> &'static str {
        "Style/RedundantFreeze"
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
        if call.is_safe_navigation() || call.name().as_slice() != b"freeze" {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let inner = strip(&receiver);
        if !immutable(inner.as_ref().unwrap_or(&receiver), file) && !operation(&receiver) {
            return;
        }
        let loc = call.location();
        let (line, col) = file.line_col(loc.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Arguments/blocks can have effects or change the dispatch's arity.
                if call.arguments().is_some() || call.block().is_some() {
                    return None;
                }
                let root = cx.ancestors().next_back()?;
                let facts = file.freeze_facts(root);
                if !facts.core_before(loc.start_offset()) || !facts.frozen(&receiver) {
                    return None;
                }
                let dot = call.call_operator_loc()?;
                let end = call.closing_loc().or(call.message_loc())?.end_offset();
                let range = dot.start_offset()..end;
                if super::correction::contains_comment(file, range.clone())
                    || file.bytes[range.clone()].contains(&b'\n')
                {
                    return None;
                }
                Some(Correction {
                    range,
                    replacement: String::new(),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(loc),
            message: "Do not freeze immutable objects, as freezing them has no effect.".into(),
            correction,
        });
    }
}
fn strip<'pr>(n: &Node<'pr>) -> Option<Node<'pr>> {
    n.as_parentheses_node()
        .and_then(|p| p.body())
        .and_then(|b| b.as_statements_node().and_then(|s| s.body().iter().next()))
}
fn immutable(n: &Node<'_>, file: &SourceFile) -> bool {
    matches!(
        n,
        Node::IntegerNode { .. }
            | Node::SourceLineNode { .. }
            | Node::FloatNode { .. }
            | Node::RationalNode { .. }
            | Node::ImaginaryNode { .. }
            | Node::SymbolNode { .. }
            | Node::InterpolatedSymbolNode { .. }
            | Node::NilNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::RegularExpressionNode { .. }
            | Node::InterpolatedRegularExpressionNode { .. }
            | Node::RangeNode { .. }
    ) || (file.frozen_string_setting() == Some(true)
        && (n.as_source_file_node().is_some()
            || n.as_string_node().is_some()
            || n.as_interpolated_string_node()
                .is_some_and(|s| s.parts().iter().all(|p| p.as_string_node().is_some()))))
}
fn numeric(n: &Node<'_>) -> bool {
    if matches!(n, Node::IntegerNode { .. } | Node::FloatNode { .. }) {
        return true;
    }
    if let Some(inner) = strip(n) {
        return numeric(&inner);
    }
    let Some(c) = n.as_call_node() else {
        return false;
    };
    let args = ast::call_arguments(&c);
    matches!(
        c.name().as_slice(),
        b"+" | b"-" | b"*" | b"**" | b"/" | b"%"
    ) && c.receiver().is_some_and(|r| numeric(&r))
        && matches!(args.as_slice(), [a] if numeric(a))
}
fn operation(n: &Node<'_>) -> bool {
    if let Some(c) = n.as_call_node() {
        return !c.is_safe_navigation()
            && matches!(c.name().as_slice(), b"count" | b"length" | b"size");
    }
    if n.as_parentheses_node().is_none() {
        return false;
    }
    let Some(n) = strip(n) else { return false };
    let Some(c) = n.as_call_node() else {
        return false;
    };
    let args = ast::call_arguments(&c);
    let [rhs] = args.as_slice() else { return false };
    if matches!(
        c.name().as_slice(),
        b"==" | b"===" | b"!=" | b"<=" | b">=" | b"<" | b">"
    ) {
        return true;
    }
    let Some(lhs) = c.receiver() else {
        return false;
    };
    (matches!(lhs, Node::FloatNode { .. } | Node::IntegerNode { .. })
        && matches!(
            c.name().as_slice(),
            b"+" | b"-" | b"*" | b"**" | b"/" | b"%" | b"<<"
        ))
        || (numeric(&lhs)
            && matches!(rhs, Node::FloatNode { .. } | Node::IntegerNode { .. })
            && matches!(
                c.name().as_slice(),
                b"+" | b"-" | b"*" | b"**" | b"/" | b"%"
            ))
}
/// A single lazy postorder proof; arbitrary similarly named methods are never
/// core APIs. Unknown eager effects and all deferred declarations block edits.
#[derive(Default)]
pub(crate) struct Facts {
    values: HashMap<(usize, usize, u8), Value>,
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
#[derive(Clone, Copy, Default)]
struct Value {
    closed: bool,
    frozen: bool,
    number: bool,
    collection: bool,
    string: bool,
}
fn key(n: &Node<'_>) -> (usize, usize, u8) {
    // Transparent list nodes can have exactly their child's source range.
    let tag = match n {
        Node::StatementsNode { .. } => 1,
        Node::ArgumentsNode { .. } => 2,
        Node::ParenthesesNode { .. } => 3,
        Node::ProgramNode { .. } => 4,
        _ => 0,
    };
    (n.location().start_offset(), n.location().end_offset(), tag)
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        // Keep Prism lifetimes local to the walk, not in the cached facts.
        struct Build<'f, 'pr> {
            facts: Facts,
            stack: Vec<(Node<'pr>, bool)>,
            file: &'f SourceFile,
        }
        impl<'pr> NodeVisitor<'pr> for Build<'_, 'pr> {
            fn enter(&mut self, n: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                self.stack.push((n, true));
            }
            fn leave(&mut self) {
                let (n, children) = self.stack.pop().unwrap();
                let mut v = Value::default();
                let atom = matches!(
                    n,
                    Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::RationalNode { .. }
                        | Node::ImaginaryNode { .. }
                        | Node::SymbolNode { .. }
                        | Node::NilNode { .. }
                        | Node::TrueNode { .. }
                        | Node::FalseNode { .. }
                        | Node::StringNode { .. }
                        | Node::RegularExpressionNode { .. }
                        | Node::SourceLineNode { .. }
                        | Node::SourceFileNode { .. }
                );
                let container = matches!(
                    n,
                    Node::ProgramNode { .. }
                        | Node::StatementsNode { .. }
                        | Node::ArgumentsNode { .. }
                        | Node::ParenthesesNode { .. }
                        | Node::ArrayNode { .. }
                        | Node::HashNode { .. }
                        | Node::AssocNode { .. }
                        | Node::RangeNode { .. }
                        | Node::ConstantWriteNode { .. }
                        | Node::LocalVariableWriteNode { .. }
                        | Node::InterpolatedStringNode { .. }
                );
                v.closed = children && (atom || container);
                v.number = matches!(
                    n,
                    Node::IntegerNode { .. } | Node::FloatNode { .. } | Node::SourceLineNode { .. }
                );
                v.string = n.as_string_node().is_some()
                    || n.as_interpolated_string_node()
                        .is_some_and(|s| s.parts().iter().all(|p| p.as_string_node().is_some()));
                v.collection = matches!(n, Node::ArrayNode { .. } | Node::HashNode { .. });
                v.frozen = immutable(&n, self.file);
                if let Some(c) = n.as_call_node() {
                    let r = c
                        .receiver()
                        .map(|r| self.facts.value(&r))
                        .unwrap_or_default();
                    let args = ast::call_arguments(&c);
                    let number_args = args.iter().all(|a| self.facts.value(a).number);
                    let numeric = r.number
                        && number_args
                        && args.len() == 1
                        && matches!(
                            c.name().as_slice(),
                            b"+" | b"-" | b"*" | b"**" | b"/" | b"%" | b"<<"
                        );
                    let comparison = (r.number && number_args
                        || r.string && args.iter().all(|a| self.facts.value(a).string))
                        && args.len() == 1
                        && matches!(
                            c.name().as_slice(),
                            b"==" | b"===" | b"!=" | b"<=" | b">=" | b"<" | b">"
                        );
                    let count = (r.collection || r.string)
                        && matches!(c.name().as_slice(), b"size" | b"length" | b"count");
                    let freeze = c.name().as_slice() == b"freeze" && r.frozen && args.is_empty();
                    v.closed = children
                        && !c.is_safe_navigation()
                        && c.block().is_none()
                        && (numeric || comparison || count || freeze);
                    v.frozen = v.closed;
                    v.number = numeric || count;
                    if !v.closed {
                        self.facts.first_effect = Some(
                            self.facts
                                .first_effect
                                .map_or(key(&n).1, |old| old.min(key(&n).1)),
                        );
                    }
                } else {
                    self.facts.changed |=
                        !atom && !container && n.as_local_variable_read_node().is_none();
                }
                if let Some(p) = n.as_parentheses_node() {
                    let inner = p.body().map(|b| self.facts.value(&b)).unwrap_or_default();
                    v = Value {
                        closed: v.closed,
                        ..inner
                    };
                } else if let Some(s) = n.as_statements_node() {
                    let inner = s
                        .body()
                        .iter()
                        .last()
                        .map(|b| self.facts.value(&b))
                        .unwrap_or_default();
                    v = Value {
                        closed: v.closed,
                        ..inner
                    };
                }
                self.facts.values.insert(key(&n), v);
                if let Some((_, closed)) = self.stack.last_mut() {
                    *closed &= v.closed;
                }
            }
        }
        let mut b = Build {
            facts: Self::default(),
            stack: Vec::new(),
            file,
        };
        walk::walk(&mut b, root);
        b.facts
    }
    fn value(&self, n: &Node<'_>) -> Value {
        self.values.get(&key(n)).copied().unwrap_or_default()
    }
    fn frozen(&self, n: &Node<'_>) -> bool {
        let v = self.value(n);
        v.closed && v.frozen
    }
    fn core_before(&self, start: usize) -> bool {
        !self.changed && self.first_effect.is_none_or(|end| end > start)
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_freeze_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    #[test]
    fn exact_positions_literals_mutability_and_safe_navigation() {
        for literal in [
            "nil", "false", "true", "1", "1.5", "1r", "1i", ":sym", "/x/", "(1..3)", "__LINE__",
        ] {
            let source = format!("# header\n  {literal}.freeze\n");
            let found = crate::pipeline::lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == RedundantFreeze.name())
                .collect::<Vec<_>>();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                "Do not freeze immutable objects, as freezing them has no effect."
            );
            crate::cops::testing::assert_correction(
                &RedundantFreeze,
                &source,
                &format!("# header\n  {literal}\n"),
            );
        }
        for source in [
            "nil&.freeze",
            "false&.freeze",
            "1&.freeze",
            "[1].freeze",
            "{}.freeze",
            "'a'.freeze",
            "object.freeze",
            "Other.freeze",
            "freeze",
            "\"#{1}\".freeze",
            "# frozen_string_literal: true\n\"#{1}\".freeze",
            "(a + b).freeze",
            "('a' * 2).freeze",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(&RedundantFreeze, source).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn actual_frozen_string_metadata_and_encoding_bytes_are_preserved() {
        for (before, after) in [
            (
                "# frozen_string_literal: true\n\"a\\x00\\x82\".freeze()\n",
                "# frozen_string_literal: true\n\"a\\x00\\x82\"\n",
            ),
            (
                "# frozen_string_literal: true\n(\"a\" \\\n\"b\").freeze\n",
                "# frozen_string_literal: true\n(\"a\" \\\n\"b\")\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&RedundantFreeze, before, after);
        }
        for header in [
            "",
            "# frozen_string_literal: false\n",
            "value = 1\n# frozen_string_literal: true\n",
        ] {
            assert!(
                crate::cops::testing::correction_offenses(
                    &RedundantFreeze,
                    &format!("{header}'a'.freeze")
                )
                .is_empty()
            );
        }
    }
    #[test]
    fn arbitrary_size_methods_comparisons_blocks_and_freeze_arguments_are_detection_only() {
        for source in [
            "object.size.freeze",
            "count.freeze",
            "(a > b).freeze",
            "(1 + a).freeze",
            "1.freeze(side_effect)",
            "1.freeze(&block)",
            "1.freeze { side_effect }",
            "def side_effect; end\n1.freeze",
            "side_effect\n1.freeze",
            "class Integer; def freeze; :changed; end; end\n1.freeze",
            "class Array; def size; []; end; end\n[].size.freeze",
            "(1; []).freeze",
            "/#{effect}/.freeze",
            "(effect..other).freeze",
        ] {
            let found = crate::cops::testing::correction_offenses(&RedundantFreeze, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn literal_operation_provenance_allows_canonical_edits_without_re_evaluation() {
        for (before, after) in [
            ("[].size.freeze", "[].size"),
            ("(1 + 2).freeze", "(1 + 2)"),
            ("(false == nil).freeze", "(false == nil)"),
        ] {
            if before.contains("false ==") {
                // Detection is broader than the closed numeric/String proof.
                assert!(
                    crate::cops::testing::correction_offenses(&RedundantFreeze, before)[0]
                        .correction
                        .is_none()
                );
            } else {
                crate::cops::testing::assert_correction(&RedundantFreeze, before, after);
            }
        }
    }
    #[test]
    fn pinned_corrections_and_individual_dispatch_holds() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_freeze_pinned.json"
        ))
        .unwrap();
        let mut counts = (0, 0);
        for case in cases.as_array().unwrap() {
            let Some(after) = case["correction"].as_str() else {
                continue;
            };
            let before = case["source"].as_str().unwrap();
            if before.contains("(a > b)") || before.contains("{ ") {
                let found = crate::cops::testing::correction_offenses(&RedundantFreeze, before);
                assert_eq!(found.len(), 1);
                assert!(found[0].correction.is_none(), "{case}");
                counts.1 += 1;
            } else {
                crate::cops::testing::assert_correction(&RedundantFreeze, before, after);
                counts.0 += 1;
            }
        }
        assert_eq!(counts, (15, 3));
    }
    #[test]
    fn all_pinned_diagnostics_and_messages() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_freeze_pinned.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let found = crate::cops::testing::correction_offenses(
                &RedundantFreeze,
                case["source"].as_str().unwrap(),
            );
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{case}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(messages), case["messages"], "{case}");
        }
    }
}
