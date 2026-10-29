//! `Style/Sample` — detect shuffle selection; preserve coupled RNG behavior.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct Sample;
impl Cop for Sample {
    fn name(&self) -> &'static str {
        "Style/Sample"
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
        let call = node.as_call_node();
        let (receiver, method, args, end) = if let Some(call) = &call {
            let Some(receiver) = call.receiver() else {
                return;
            };
            (
                receiver,
                call.name().as_slice(),
                ast::call_arguments(call),
                ast::source_end(node),
            )
        } else if let Some(getter) = ast::compound_index_getter(node) {
            let end = match node {
                Node::IndexAndWriteNode { .. } => node
                    .as_index_and_write_node()
                    .unwrap()
                    .closing_loc()
                    .end_offset(),
                Node::IndexOrWriteNode { .. } => node
                    .as_index_or_write_node()
                    .unwrap()
                    .closing_loc()
                    .end_offset(),
                Node::IndexOperatorWriteNode { .. } => node
                    .as_index_operator_write_node()
                    .unwrap()
                    .closing_loc()
                    .end_offset(),
                _ => return,
            };
            (getter.receiver, b"[]".as_slice(), getter.arguments, end)
        } else {
            return;
        };
        if !matches!(method, b"first" | b"last" | b"at" | b"slice" | b"[]") {
            return;
        }
        let Some(shuffle) = receiver.as_call_node() else {
            return;
        };
        if shuffle.name().as_slice() != b"shuffle"
            || shuffle.block().is_some_and(|b| b.as_block_node().is_some())
        {
            return;
        }
        let size = if matches!(method, b"first" | b"last") {
            None
        } else {
            let Some(size) = sample_size(&args) else {
                return;
            };
            size
        };
        let shuffle_args = ast::call_arguments(&shuffle);
        let mut preferred = Vec::new();
        if matches!(method, b"first" | b"last") {
            if let Some(arg) = args.first() {
                preferred.push(text(arg));
            }
        } else if matches!(method, b"[]" | b"slice")
            && let Some(size) = size
        {
            preferred.push(size);
        }
        if let Some(arg) = shuffle_args.first() {
            preferred.push(text(arg));
        }
        let preferred = if preferred.is_empty() {
            "sample".into()
        } else {
            format!("sample({})", preferred.join(", "))
        };
        let Some(selector) = shuffle.message_loc() else {
            return;
        };
        let start = selector.start_offset();
        let incorrect = String::from_utf8_lossy(&file.bytes[start..end]);
        let correction = if cx.corrections_requested() {
            call.as_ref().and_then(|call| {
                safe_correction(
                    call,
                    &shuffle,
                    &args,
                    &shuffle_args,
                    method,
                    cx,
                    file,
                    start..end,
                    &preferred,
                )
            })
        } else {
            None
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{preferred}` instead of `{incorrect}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction,
        });
    }
}
fn text(n: &Node<'_>) -> String {
    String::from_utf8_lossy(n.location().as_slice()).into_owned()
}
fn small(n: &Node<'_>) -> Option<i32> {
    n.as_integer_node()?.value().try_into().ok()
}
// Outer None means unknown. Inner None is upstream's scalar/nil sample size,
// including its deliberately permissive zero/>2 argument cases.
fn sample_size(args: &[Node<'_>]) -> Option<Option<String>> {
    match args {
        [n] => {
            if let Some(range) = n.as_range_node() {
                if range.left().is_some_and(|n| small(&n) != Some(0)) {
                    return None;
                }
                let high = range.right()?.as_integer_node()?;
                let value = high.value();
                let (negative, digits) = value.to_u32_digits();
                if negative && digits.iter().any(|d| *d != 0) {
                    return None;
                }
                Some(Some(decimal(digits, !range.is_exclude_end(), false)))
            } else if matches!(small(n), Some(0 | -1)) {
                Some(None)
            } else {
                None
            }
        }
        [low, high] if small(low) == Some(0) => {
            let integer = high.as_integer_node()?;
            let value = integer.value();
            let (negative, digits) = value.to_u32_digits();
            Some(Some(decimal(digits, false, negative)))
        }
        [_, _] => None,
        _ => Some(None),
    }
}
fn decimal(digits: &[u32], increment: bool, negative: bool) -> String {
    let mut decimal = vec![0_u64];
    for digit in digits.iter().rev() {
        let mut carry = u64::from(*digit);
        for part in &mut decimal {
            let next = (*part << 32) + carry;
            *part = next % 1_000_000_000;
            carry = next / 1_000_000_000;
        }
        while carry != 0 {
            decimal.push(carry % 1_000_000_000);
            carry /= 1_000_000_000;
        }
    }
    if increment {
        let mut carry = 1;
        for part in &mut decimal {
            let n = *part + carry;
            *part = n % 1_000_000_000;
            carry = n / 1_000_000_000;
        }
        if carry != 0 {
            decimal.push(carry);
        }
    }
    let mut out = if negative && decimal.iter().any(|d| *d != 0) {
        "-".to_owned()
    } else {
        String::new()
    };
    out.push_str(&decimal.pop().unwrap().to_string());
    for part in decimal.iter().rev() {
        use std::fmt::Write;
        let _ = write!(out, "{part:09}");
    }
    out
}
/// Trust only zero/one closed literal elements and strict scalar selection or
/// first/last(0). Larger arrays change values AND implicit RNG state. No flow.
#[allow(clippy::too_many_arguments)]
fn safe_correction(
    call: &CallNode<'_>,
    shuffle: &CallNode<'_>,
    args: &[Node<'_>],
    shuffle_args: &[Node<'_>],
    method: &[u8],
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    range: std::ops::Range<usize>,
    preferred: &str,
) -> Option<Correction> {
    let array = shuffle.receiver()?.as_array_node()?;
    if array.elements().len() > 1
        || !shuffle_args.is_empty()
        || call.block().is_some()
        || shuffle.block().is_some()
    {
        return None;
    }
    let selection = match method {
        b"first" | b"last" => args.is_empty() || matches!(args,[n] if small(n)==Some(0)),
        b"at" | b"slice" | b"[]" => matches!(args,[n] if matches!(small(n),Some(0 | -1))),
        _ => false,
    };
    let root = cx.ancestors().next_back()?;
    let facts = file.format_string_facts(root);
    if !selection
        || !facts.core_before(call.location().start_offset())
        || !facts.closed(&array.as_node())
        || super::correction::contains_comment(file, range.clone())
        || file.bytes[range.clone()].contains(&b'\n')
        || file.contains_heredoc(root, &call.as_node())
        || file.heredoc_pending_at(root, range.start)
    {
        return None;
    }
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range,
            replacement: preferred.into(),
        },
    )
}
/// Canonical and original degenerate API spellings share closed-dispatch facts.
pub(super) fn primitive_api(call: &CallNode<'_>) -> bool {
    if call.block().is_some() {
        return false;
    }
    if call.name().as_slice() == b"sample" {
        return call
            .receiver()
            .is_some_and(|n| n.as_array_node().is_some_and(|a| a.elements().len() <= 1));
    }
    if call.name().as_slice() == b"shuffle" {
        return ast::call_arguments(call).is_empty()
            && call
                .receiver()
                .is_some_and(|n| n.as_array_node().is_some_and(|a| a.elements().len() <= 1));
    }
    matches!(
        call.name().as_slice(),
        b"first" | b"last" | b"at" | b"slice" | b"[]"
    ) && call
        .receiver()
        .and_then(|n| n.as_call_node())
        .is_some_and(|s| s.name().as_slice() == b"shuffle" && primitive_api(&s))
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop sample_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == Sample.name())
            .collect()
    }
    #[test]
    fn pinned_candidates_exact_messages_and_navigation() {
        for (wrong, right) in [
            ("shuffle.first", "sample"),
            ("shuffle.last", "sample"),
            ("shuffle&.first", "sample"),
            ("shuffle&.last", "sample"),
            ("shuffle[0]", "sample"),
            ("shuffle[-1]", "sample"),
            ("shuffle[...3]", "sample(3)"),
            ("shuffle[0, 3]", "sample(3)"),
            ("shuffle[0..3]", "sample(4)"),
            ("shuffle[0...3]", "sample(3)"),
            ("shuffle.first(2)", "sample(2)"),
            ("shuffle.last(3)", "sample(3)"),
            ("shuffle.first(foo)", "sample(foo)"),
            ("shuffle.last(bar)", "sample(bar)"),
            ("shuffle.at(0)", "sample"),
            ("shuffle.at(-1)", "sample"),
            ("shuffle.slice(0)", "sample"),
            ("shuffle.slice(-1)", "sample"),
            ("shuffle.slice(0, 3)", "sample(3)"),
            ("shuffle.slice(0..3)", "sample(4)"),
            ("shuffle.slice(0...3)", "sample(3)"),
            (
                "shuffle(random: Random.new).first",
                "sample(random: Random.new)",
            ),
            (
                "shuffle(random: Random.new).first(2)",
                "sample(2, random: Random.new)",
            ),
            ("shuffle(random: foo).last(bar)", "sample(bar, random: foo)"),
            (
                "shuffle(random: Random.new)[0..3]",
                "sample(4, random: Random.new)",
            ),
            (
                "shuffle(random: Random.new(123)).first(5)",
                "sample(5, random: Random.new(123))",
            ),
        ] {
            for (op, col) in [(".", 11), ("&.", 12)] {
                let s = format!("# header\n[1, 2, 3]{op}{wrong}\n");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col, 2), "{s}");
                assert_eq!(o[0].message, format!("Use `{right}` instead of `{wrong}`."));
                assert!(
                    correction_offenses(&Sample, &s)[0].correction.is_none(),
                    "RNG: {s}"
                );
            }
        }
    }
    #[test]
    fn arbitrary_precision_sizes_use_decimal_and_preserve_detection_only_oddities() {
        for (s, right) in [
            (
                "ary.shuffle[0..99999999999999999999999999999999999999999999999999]",
                "sample(100000000000000000000000000000000000000000000000000)",
            ),
            (
                "ary.shuffle.slice(0,-99999999999999999999999999999999999999999999999999)",
                "sample(-99999999999999999999999999999999999999999999999999)",
            ),
            (
                "ary.shuffle[0..79228162435036175079279612742456049663]",
                "sample(79228162435036175079279612742456049664)",
            ),
            ("ary.shuffle[0..0xF]", "sample(16)"),
            ("ary.shuffle[0...0]", "sample(0)"),
            ("ary.shuffle.at(0, -2)", "sample"),
            ("ary.shuffle.first(1,2,3)", "sample(1)"),
        ] {
            let o = correction_offenses(&Sample, s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                format!("Use `{right}` instead of `{}`.", &s[4..])
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_lookalikes() {
        for suffix in [
            "sample",
            "shuffle",
            "shuffle.at(2)",
            "shuffle.at(foo)",
            "shuffle.slice(2)",
            "shuffle.slice(3, 3)",
            "shuffle.slice(2..3)",
            "shuffle.slice(2..-3)",
            "shuffle.slice(foo..3)",
            "shuffle.slice(-4..-3)",
            "shuffle.slice(foo)",
            "shuffle.slice(foo, 3)",
            "shuffle.slice(foo..bar)",
            "shuffle.slice(foo, bar)",
            "shuffle[2]",
            "shuffle[3, 3]",
            "shuffle[2..3]",
            "shuffle[2..-3]",
            "shuffle[foo..3]",
            "shuffle[3..]",
            "shuffle[3...]",
            "shuffle[0..]",
            "shuffle[0...]",
            "shuffle.slice(0..)",
            "shuffle[-4..-3]",
            "shuffle[foo]",
            "shuffle[foo, 3]",
            "shuffle[foo..bar]",
            "shuffle[foo, bar]",
            "shuffle(random: Random.new)",
            "shuffle.join([5, 6, 7])",
            "shuffle.map { |e| e }",
            "shuffle(random: Random.new)[2]",
            "shuffle(random: Random.new)[2, 3]",
            "shuffle(random: Random.new).find(&:odd?)",
            "shuffle[0] = 1",
        ] {
            assert!(offenses(&format!("ary.{suffix}")).is_empty(), "{suffix}");
        }
    }
    #[test]
    fn proven_degenerate_core_literal_corrections() {
        for array in ["[]", "[42]", "[:one]", "[nil]", "[[42]]", "%i[one]"] {
            for method in [
                "first", "last", "at(0)", "at(-1)", "slice(0)", "[0]", "[-1]",
            ] {
                let dot = if method.starts_with('[') { "" } else { "." };
                assert_correction(
                    &Sample,
                    &format!("{array}.shuffle{dot}{method}"),
                    &format!("{array}.sample"),
                );
            }
            for method in ["first(0)", "last(0)"] {
                assert_correction(
                    &Sample,
                    &format!("{array}.shuffle.{method}"),
                    &format!("{array}.sample(0)"),
                );
            }
        }
    }
    #[test]
    fn odd_arities_block_passes_and_compound_getters_keep_reports_without_edits() {
        for (s, right) in [
            ("[].shuffle.first(-1)", "sample(-1)"),
            ("[].shuffle.slice(0,-1)", "sample(-1)"),
            ("ary.shuffle.at(0,3)", "sample"),
            ("ary.shuffle.[]()", "sample"),
            ("ary.shuffle.slice(0,1,2)", "sample"),
            ("ary.shuffle.first(&block)", "sample(&block)"),
            ("ary.shuffle[0] += 1", "sample"),
        ] {
            let o = correction_offenses(&Sample, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(
                o[0].message.starts_with(&format!("Use `{right}`")),
                "{s}: {o:?}"
            );
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
}
