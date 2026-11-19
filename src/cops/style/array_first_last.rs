//! `Style/ArrayFirstLast`: opt-in indexed-read suggestions; unsafe correction.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ArrayFirstLast;
impl Cop for ArrayFirstLast {
    fn name(&self) -> &'static str {
        "Style/ArrayFirstLast"
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
        let Some(preferred) = preferred_name(&call) else {
            return;
        };
        // RuboCop records a range only once, parent first. A safe [] parent
        // can suggest its own preference at this ordinary receiver's range.
        // Suppress that duplicate locally rather than searching prior offenses.
        if ordinary_braces(&call)
            && cx
                .parent()
                .and_then(|n| n.as_call_node())
                .is_some_and(|parent| {
                    parent.is_safe_navigation()
                        && preferred_name(&parent).is_some()
                        && parent.receiver().is_some_and(|r| {
                            r.location().start_offset() == node.location().start_offset()
                                && r.location().end_offset() == node.location().end_offset()
                        })
                })
        {
            return;
        }
        // Only one receiver edge can survive the pinned innermost-braces
        // rule: a second ordinary [] edge necessarily has a brace parent.
        // Avoid rescanning arbitrarily long chains. Literal blocks create a
        // synthetic parent/receiver boundary in RuboCop, unlike Prism.
        let receiver = call
            .receiver()
            .and_then(|n| n.as_call_node())
            .filter(ordinary_braces);
        let (inner, inner_parent) = if let Some(receiver) = receiver {
            if !call.is_safe_navigation()
                || receiver
                    .receiver()
                    .and_then(|n| n.as_call_node())
                    .is_some_and(|c| ordinary_braces(&c))
            {
                return;
            }
            (receiver, Some(call.as_node()))
        } else {
            (call, None)
        };
        let parent = inner_parent.as_ref().or_else(|| {
            cx.ancestors()
                .find(|n| n.as_arguments_node().is_none() && n.as_statements_node().is_none())
        });
        if !literal_block(&inner) && parent.is_some_and(|p| brace_parent(p, &inner.as_node())) {
            return;
        }
        let Some(selector) = inner.message_loc() else {
            return;
        };
        let start = selector.start_offset();
        let end = if inner.call_operator_loc().is_some() {
            ast::source_end(&inner.as_node())
        } else {
            selector.end_offset()
        };
        let correction = cx.corrections_requested().then(|| {
            let (end, replacement) = if inner.call_operator_loc().is_some() {
                (
                    crate::cops::bundler::ordered_gem_node::send_end(&inner),
                    preferred.to_owned(),
                )
            } else {
                (selector.end_offset(), format!(".{preferred}"))
            };
            crate::offense::Correction {
                range: start..end,
                replacement,
            }
        });
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{preferred}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction,
        });
    }
}
fn preferred_name(call: &ruby_prism::CallNode<'_>) -> Option<&'static str> {
    if call.name().as_slice() != b"[]" {
        return None;
    }
    let args = ast::call_arguments(call);
    let [arg] = args.as_slice() else {
        return None;
    };
    let value: i32 = arg.as_integer_node()?.value().try_into().ok()?;
    match value {
        0 => Some("first"),
        -1 => Some("last"),
        _ => None,
    }
}
fn literal_block(call: &ruby_prism::CallNode<'_>) -> bool {
    call.block().is_some_and(|b| b.as_block_node().is_some())
}
fn ordinary_braces(call: &ruby_prism::CallNode<'_>) -> bool {
    !call.is_safe_navigation() && !literal_block(call) && call.name().as_slice() == b"[]"
}
fn brace_parent(node: &Node<'_>, child: &Node<'_>) -> bool {
    if node
        .as_call_node()
        .is_some_and(|c| !c.is_safe_navigation() && matches!(c.name().as_slice(), b"[]" | b"[]="))
        || node.as_index_target_node().is_some()
    {
        return true;
    }
    let value = match node {
        Node::IndexAndWriteNode { .. } => node.as_index_and_write_node().unwrap().value(),
        Node::IndexOrWriteNode { .. } => node.as_index_or_write_node().unwrap().value(),
        Node::IndexOperatorWriteNode { .. } => node.as_index_operator_write_node().unwrap().value(),
        _ => return false,
    };
    child.location().end_offset() <= value.location().start_offset()
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop array_first_last_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(&format!("# rustocop:enable Style/ArrayFirstLast\n{s}"))
            .into_iter()
            .filter(|o| o.cop == ArrayFirstLast.name())
            .collect()
    }
    #[test]
    fn all_pinned_positive_examples_have_exact_diagnostics() {
        for (s, col, method) in [
            ("arr[0]", 4, "first"),
            ("arr[-1]", 4, "last"),
            ("arr.[](0)", 5, "first"),
            ("arr&.[](0)", 6, "first"),
            ("arr.[] 0", 5, "first"),
            ("arr&.[] 0", 6, "first"),
            ("arr.[](-1)", 5, "last"),
            ("arr&.[](-1)", 6, "last"),
            ("arr.[] -1", 5, "last"),
            ("arr&.[] -1", 6, "last"),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col, 2), "{s}");
            assert_eq!(o[0].message, format!("Use `{method}`."));
        }
    }
    #[test]
    fn all_pinned_negatives_and_write_shapes() {
        for s in [
            "arr[1]",
            "arr[index]",
            "arr[0] = 1",
            "arr.first",
            "arr.last",
            "arr[0][-1]",
            "arr[0] += 1",
            "arr[-1] -= 1",
            "arr[0] ||= 1",
            "arr[-1] &&= 1",
            "arr.[](0) += 1",
            "arr&.[](-1) ||= 1",
            "arr[0], arr[-1] = values",
            "arr[0][1] = 1",
            "arr[1][0]",
            "arr[0, 1]",
            "arr[0..1]",
            "arr[*[0]]",
            "arr[0, &block]",
            "arr[0.0]",
            "arr[-2]",
            "arr[(0)]",
            "arr[0r]",
            "arr[0i]",
            "arr[1000000000000000000000000000000]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn unknown_array_like_receivers_and_encoded_literals_are_corrected_too() {
        for s in [
            "[][0]",
            "[[]][-1]",
            "{0 => 1}[0]",
            "'abc'[-1]",
            "object[0]",
            "effect()[0]",
            "%w[a\\ b c][0]",
            "%W[a#{effect} b][-1]",
            "%i[a b][0]",
            "%I[a#{effect} b][-1]",
            "arr[-0x1]",
            "arr[0b0]",
            "arr[+0]",
            "arr[0] { effect }",
        ] {
            let o = crate::cops::testing::correction_offenses(&ArrayFirstLast, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_some());
        }
        assert!(!ArrayFirstLast.safe_fixes());
        assert!(!ArrayFirstLast.default_enabled());
    }
    #[test]
    fn safe_navigation_chains_report_each_range_once_with_outer_preference() {
        for (s, col, message) in [
            ("a[0]&.[](-1)", 2, "Use `last`."),
            ("a[-1]&.[](0)", 2, "Use `first`."),
            ("a[0]&.[](1)", 2, "Use `first`."),
            ("a[0]&.[](0)&.[](-1)", 2, "Use `first`."),
        ] {
            let o = offenses(s);
            let at_inner: Vec<_> = o.iter().filter(|o| o.col == col).collect();
            assert_eq!(at_inner.len(), 1, "{s}: {o:?}");
            assert_eq!(at_inner[0].message, message, "{s}");
        }
    }
    #[test]
    fn literal_blocks_are_synthetic_receiver_and_parent_boundaries() {
        for s in ["a[0] {}[0]", "a[0] {}.[](0)"] {
            let o = offenses(s);
            assert_eq!(o.len(), 2, "{s}: {o:?}");
            assert_eq!(o[0].message, "Use `first`.");
            assert_eq!(o[1].message, "Use `first`.");
        }
    }
    #[test]
    fn indexed_receiver_parentheses_arguments_and_rhs_boundaries() {
        for s in ["arr[foo[0]]", "arr.[]=(other[0], 1)", "arr[0][other[-1]]"] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "(arr[0])[1]",
            "arr[0].foo",
            "arr[0] += other[-1]",
            "consume(arr[0])",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
}
