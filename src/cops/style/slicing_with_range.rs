//! `Style/SlicingWithRange` — fixed Ruby 3.4, unsafe suggestions only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SlicingWithRange;
impl Cop for SlicingWithRange {
    fn name(&self) -> &'static str {
        "Style/SlicingWithRange"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (args, start, end, dotted, parenthesized) = if let Some(call) = node.as_call_node() {
            if call.name().as_slice() != b"[]" {
                return;
            }
            let Some(message) = call.message_loc() else {
                return;
            };
            (
                ast::call_arguments(&call),
                call.call_operator_loc()
                    .map_or(message.start_offset(), |l| l.start_offset()),
                ast::source_end(node),
                call.call_operator_loc().is_some(),
                call.opening_loc().is_some(),
            )
        } else if let Some(getter) = ast::compound_index_getter(node) {
            macro_rules! locations {
                ($access:ident) => {{
                    let n = node.$access().unwrap();
                    (n.opening_loc().start_offset(), n.closing_loc().end_offset())
                }};
            }
            let (start, end) = match node {
                Node::IndexOperatorWriteNode { .. } => locations!(as_index_operator_write_node),
                Node::IndexOrWriteNode { .. } => locations!(as_index_or_write_node),
                Node::IndexAndWriteNode { .. } => locations!(as_index_and_write_node),
                _ => unreachable!(),
            };
            (getter.arguments, start, end, false, true)
        } else {
            return;
        };
        let [arg] = args.as_slice() else {
            return;
        };
        let Some(range) = arg.as_range_node() else {
            return;
        };
        let (Some(left), Some(right)) = (range.left(), range.right()) else {
            return;
        };
        let nil_end = right.as_nil_node().is_some();
        let inclusive = !range.is_exclude_end();
        let useless = integer(&left, 0) && (nil_end || inclusive && integer(&right, -1));
        let current = String::from_utf8_lossy(&file.bytes[start..end]);
        let message = if useless {
            format!("Remove the useless `{current}`.")
        } else {
            if dotted && !parenthesized {
                return;
            }
            let operator = String::from_utf8_lossy(range.operator_loc().as_slice());
            let prefer = if nil_end || inclusive && integer(&right, -1) {
                format!(
                    "{}{operator}",
                    String::from_utf8_lossy(left.location().as_slice())
                )
            } else if inclusive && left.as_nil_node().is_some() {
                format!(
                    "{operator}{}",
                    String::from_utf8_lossy(right.location().as_slice())
                )
            } else {
                return;
            };
            if dotted {
                format!(
                    "Prefer `{prefer}` over `{}`.",
                    String::from_utf8_lossy(arg.location().as_slice())
                )
            } else {
                format!("Prefer `[{prefer}]` over `{current}`.")
            }
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
fn integer(node: &Node<'_>, value: i32) -> bool {
    node.as_integer_node().is_some_and(|n| {
        let actual: Result<i32, _> = n.value().try_into();
        actual.ok() == Some(value)
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop slicing_with_range_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == SlicingWithRange.name())
            .collect()
    }
    #[test]
    fn pinned_exact_diagnostics() {
        for (s, msg) in [
            ("ary[0..-1]", "Remove the useless `[0..-1]`."),
            ("ary[1..-1]", "Prefer `[1..]` over `[1..-1]`."),
            ("ary[nil..42]", "Prefer `[..42]` over `[nil..42]`."),
            ("ary&.[](0...nil)", "Remove the useless `&.[](0...nil)`."),
            ("ary.[](1...nil)", "Prefer `1...` over `1...nil`."),
            ("ary.[] 0..-1", "Remove the useless `.[] 0..-1`."),
            ("ary.[](nil..42)", "Prefer `..42` over `nil..42`."),
        ] {
            let o = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 6, 2));
            assert_eq!(o[0].message, msg);
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_and_structural_negatives() {
        for s in [
            "ary[0...-1]",
            "ary[1...-1]",
            "ary.push(1..-1)",
            "[1..-1]",
            "ary[0..42]",
            "ary[..-1]",
            "ary[1..]",
            "ary.[] 1..-1",
            "ary.[] nil..42",
            "ary[1..-1, 2]",
            "ary[(1..-1)]",
            "ary[nil...42]",
            "ary[1..-1] = x",
            "ary[1..-1, &b]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn unsafe_array_like_mutation_evaluation_and_compound_getters_never_edit() {
        for s in [
            "[][0..-1]",
            "%w[a b][1..nil]",
            "factory()[start()..-1]",
            "proc { |r| r.sum }[2..-1]",
            "ary[1..-1] += rhs",
            "ary[1..-1] ||= rhs",
            "ary[1..-1] &&= rhs",
            "ary[1..-1] { effect }",
            "ary&.[](nil..42)",
            "ary[(start())..-1]",
        ] {
            let o = crate::cops::testing::correction_offenses(&SlicingWithRange, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none());
        }
        assert!(!SlicingWithRange.safe_fixes());
    }
}
