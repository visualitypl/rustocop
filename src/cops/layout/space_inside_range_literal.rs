//! `Layout/SpaceInsideRangeLiteral` — no blank around the operator of a
//! range literal.
//!
//! A port of RuboCop's cop (97f30c4, MIT). `on_irange`/`on_erange` are
//! Prism's `RangeNode` (inclusive and exclusive, beginless and endless
//! included; a flip-flop is a `FlipFlopNode` and is not checked). Upstream
//! tests the node's source text: the first operator directly followed by a
//! line break has that break and the whitespace after it removed (a
//! multi-line range written `0..` + line break is accepted), then the text
//! is reported (`Space inside range literal.`, at the whole node) when any
//! occurrence of the operator is preceded or followed by Ruby `\s`. The
//! test is textual, as upstream: an operator spelled inside an operand
//! (`'a .. b'..'z'`, the `...` of a nested range) counts too.
//!
//! The correction is the canonical edit at the actual operator: the run of
//! `\s` before it and the run after it are removed (the operator is
//! rewritten as itself). Upstream rewrites the first matching occurrence in
//! the node's text instead, which can be inside a string operand; here such
//! a finding with no blank at the actual operator has no edit. The edit is
//! withheld, finding kept, when the run before the operator holds a line
//! break (a line continuation: removing it alone breaks the syntax), when a
//! run after it holds a line break but does not reach the right operand (a
//! comment line or heredoc body follows), and when a `...` is followed by a
//! line break (Ruby warns `... at EOL, should be parenthesized?`; joining
//! would drop that warning). A joined line break is otherwise the upstream
//! edit (`0 ..` + line break + `10` becomes `0..10`): the token after the
//! operator is lexed at the start of an expression either way. `__LINE__`,
//! `DATA` and heredoc boundaries after it are held by the shared `run_cops`
//! guards, and the listed source observers hold every edit.
use std::ops::Range;

use ruby_prism::Node;

use super::space_after_colon::ruby_space;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceInsideRangeLiteral;

const MSG: &str = "Space inside range literal.";

/// Upstream's textual test on the node's source `text` with operator `op`.
fn spaced(text: &[u8], op: &[u8]) -> bool {
    let occurrences = |text: &[u8]| -> Vec<usize> {
        (0..text.len().saturating_sub(op.len() - 1))
            .filter(|&i| text[i..].starts_with(op))
            .collect()
    };
    // `expression.sub!(/#{op}\n\s*/, op)`: the first operator directly
    // followed by a line break loses the break and the whitespace after it.
    let mut joined;
    let text = match occurrences(text)
        .into_iter()
        .find(|&i| text.get(i + op.len()) == Some(&b'\n'))
    {
        Some(i) => {
            let after = i + op.len() + 1;
            let end = after + text[after..].iter().take_while(|b| ruby_space(**b)).count();
            joined = text[..i + op.len()].to_vec();
            joined.extend_from_slice(&text[end..]);
            &joined[..]
        }
        None => text,
    };
    occurrences(text).into_iter().any(|i| {
        i.checked_sub(1).is_some_and(|b| ruby_space(text[b]))
            || text.get(i + op.len()).is_some_and(|b| ruby_space(*b))
    })
}

/// The canonical edit's range: the `\s` runs before and after the operator,
/// when removing them cannot change the program (see the module docs).
fn edit_range(file: &SourceFile, range: &ruby_prism::RangeNode<'_>) -> Option<Range<usize>> {
    let bytes = &file.bytes;
    let operator = range.operator_loc();
    let left_end = range.left().map(|l| l.location().end_offset());
    let start = operator.start_offset()
        - bytes[left_end.unwrap_or(operator.start_offset())..operator.start_offset()]
            .iter()
            .rev()
            .take_while(|b| ruby_space(**b))
            .count();
    let after = operator.end_offset();
    let end = after
        + bytes[after..]
            .iter()
            .take_while(|b| ruby_space(**b))
            .count();
    let end = match range.right() {
        Some(right) => end.min(right.location().start_offset()),
        // An endless range keeps whatever follows it.
        None => after,
    };
    if start == operator.start_offset() && end == after {
        return None;
    }
    if bytes[start..operator.start_offset()].contains(&b'\n') {
        return None;
    }
    if bytes[after..end].contains(&b'\n')
        && (operator.as_slice() == b"..."
            || range
                .right()
                .is_none_or(|right| right.location().start_offset() != end))
    {
        return None;
    }
    Some(start..end)
}

impl Cop for SpaceInsideRangeLiteral {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideRangeLiteral"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_range_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(range) = node.as_range_node() else {
            return;
        };
        let location = node.location();
        let operator = range.operator_loc();
        if !spaced(location.as_slice(), operator.as_slice()) {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| edit_range(file, &range))
            .flatten()
            .map(|range| Correction {
                range,
                replacement: if operator.as_slice() == b"..." {
                    "...".into()
                } else {
                    "..".into()
                },
            });
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line: file.last_line_of(location),
            col,
        });
    }
}

#[cfg(test)]
mod tests;
