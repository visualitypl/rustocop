//! `Style/RedundantSort` — unsafe sort-then-select hints, never corrections.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct RedundantSort;
impl Cop for RedundantSort {
    fn name(&self) -> &'static str {
        "Style/RedundantSort"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(access) = node.as_call_node() else {
            return;
        };
        let Some(min) = endpoint(&access) else { return };
        let Some(receiver) = access.receiver() else {
            return;
        };
        let Some(sort) = receiver.as_call_node() else {
            return;
        };
        let sorter = sort.name();
        let sorter = sorter.as_slice();
        if !matches!(sorter, b"sort" | b"sort_by") {
            return;
        }
        let args = ast::call_arguments(&sort);
        let literal_block = sort.block().is_some_and(|b| b.as_block_node().is_some());
        let valid_arguments = if literal_block || sorter == b"sort" {
            args.is_empty()
        } else {
            args.len() == 1
        };
        if !valid_arguments {
            return;
        }
        // At the pin only the nonliteral sort_by/index branch requires send
        // rather than call: retain its safe-navigation exemption.
        if sorter == b"sort_by"
            && !literal_block
            && !matches!(access.name().as_slice(), b"first" | b"last")
            && (sort.is_safe_navigation() || access.is_safe_navigation())
        {
            return;
        }
        let Some(selector) = sort.message_loc() else {
            return;
        };
        let Some(access_selector) = access.message_loc() else {
            return;
        };
        let end = ast::source_end(node);
        if kept_apart(&access, cx, file, end) {
            return;
        }
        let (line, col) = file.line_col(selector.start_offset());
        let suggestion = format!(
            "{}{}",
            if min { "min" } else { "max" },
            if sorter == b"sort_by" { "_by" } else { "" }
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!(
                "Use `{suggestion}` instead of `{}...{}`.",
                single_line(sorter),
                single_line(&file.bytes[access_selector.start_offset()..end])
            ),
            correction: None,
        });
    }
}
fn endpoint(call: &CallNode<'_>) -> Option<bool> {
    let args = ast::call_arguments(call);
    match call.name().as_slice() {
        b"first" if args.is_empty() => Some(true),
        b"last" if args.is_empty() => Some(false),
        b"[]" | b"at" | b"slice" => {
            let [arg] = args.as_slice() else { return None };
            let n = integer_literal_value(arg)?;
            match n {
                0 => Some(true),
                -1 => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}
/// Prism keeps spaced unary negation as calls; the pinned AST folds it into
/// the integer literal. Do not fold explicit method calls or grouped operands.
fn integer_literal_value(node: &Node<'_>) -> Option<i32> {
    if let Some(integer) = node.as_integer_node() {
        return integer.value().try_into().ok();
    }
    let call = node.as_call_node()?;
    if call.name().as_slice() != b"-@"
        || call.message_loc()?.as_slice() != b"-"
        || call.call_operator_loc().is_some()
        || call.arguments().is_some()
        || call.block().is_some()
    {
        return None;
    }
    integer_literal_value(&call.receiver()?)?.checked_neg()
}
/// Match the pinned exception for comments/heredocs separating an accessor
/// from a trailing modifier/operator. Only inspect this call's trivia and line.
fn kept_apart(
    call: &CallNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    end: usize,
) -> bool {
    if cx
        .parent()
        .is_some_and(|n| matches!(n, Node::AndNode { .. } | Node::OrNode { .. }))
    {
        return false;
    }
    let receiver_end = call.receiver().unwrap().location().end_offset();
    let dot = call
        .call_operator_loc()
        .or_else(|| call.message_loc())
        .unwrap()
        .start_offset();
    let before = &file.bytes[receiver_end..dot];
    if !before.contains(&b'\n') || before.iter().all(u8::is_ascii_whitespace) {
        return false;
    }
    let rest = &file.bytes[end..];
    let rest = &rest[..rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len())];
    let rest = rest.trim_ascii();
    !rest.is_empty()
        && !matches!(rest[0], b'#' | b'.' | b')' | b']' | b'}' | b',')
        && !rest.starts_with(b"&.")
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/redundant_sort_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantSort.name())
            .collect()
    }
    #[test]
    fn endpoints_blocks_and_navigation() {
        for (sort, suffix) in [
            ("sort", ""),
            ("sort { |a,b| b <=> a }", ""),
            ("sort { _2 <=> _1 }", ""),
            ("sort_by { |x| x.key }", "_by"),
            ("sort_by { _1.key }", "_by"),
            ("sort_by(&:key)", "_by"),
        ] {
            for (access, min) in [
                ("first", true),
                ("last", false),
                ("at(0)", true),
                ("slice(-1)", false),
                ("[](0)", true),
            ] {
                let s = format!("\n  xs.{sort}.{access}");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col), (2, 6));
                let sorter = if suffix.is_empty() { "sort" } else { "sort_by" };
                assert_eq!(
                    o[0].message,
                    format!(
                        "Use `{}{suffix}` instead of `{sorter}...{access}`.",
                        if min { "min" } else { "max" }
                    )
                );
                assert!(
                    crate::cops::testing::correction_offenses(&RedundantSort, &s)[0]
                        .correction
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn spaced_negative_endpoint_literals() {
        for (source, suggestion, accessor, sorter) in [
            ("xs.sort.at(- 1)", "max", "at(- 1)", "sort"),
            ("xs.sort_by(&:key)[- 1]", "max_by", "[- 1]", "sort_by"),
            ("xs.sort.slice(- 0)", "min", "slice(- 0)", "sort"),
            ("xs.sort { |a,b| a <=> b }[- 0x1]", "max", "[- 0x1]", "sort"),
            ("xs.sort.at(- - -1)", "max", "at(- - -1)", "sort"),
        ] {
            let o = offenses(source);
            assert_eq!(o.len(), 1, "{source}");
            assert_eq!((o[0].line, o[0].col), (1, 4));
            assert_eq!(
                o[0].message,
                format!("Use `{suggestion}` instead of `{sorter}...{accessor}`.")
            );
            assert!(o[0].correction.is_none());
        }
        for index in [
            "- 2",
            "- 1.0",
            "+ 0",
            "-(1)",
            "1.-@",
            "1.-@()",
            "- -1",
            "- variable",
        ] {
            let source = format!("xs.sort.at({index})");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_block_pass_index_navigation_exception() {
        for s in [
            "xs&.sort_by(&:key)[0]",
            "xs&.sort_by(&b).at(-1)",
            "xs.sort_by(&:key)&.slice(0)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "xs&.sort_by(&:key).first",
            "xs.sort_by(&:key)&.last",
            "xs&.sort_by { it }&.at(0)",
            "xs&.sort&.slice(-1)",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn rejects_counts_mutators_enumerators_and_separated_trailing_code() {
        for s in [
            "xs.sort.first(1)",
            "xs.sort(1).first",
            "xs.sort(&cmp).first",
            "xs.sort!.first",
            "xs.sort_by!(&:key).last",
            "xs.sort_by.first",
            "xs.sort_by { it }",
            "xs.first.sort",
            "xs.sort[1]",
            "xs.sort_by(&:key).at(-2)",
            "xs.sort # comment\n .first if ok",
            "xs.sort\n # comment\n .first + 1",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "xs.sort # comment\n .first",
            "xs.sort # comment\n .first || []",
            "xs.sort\n .first if ok",
            "xs.sort_by { |x| x.key || other }.last",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
}
