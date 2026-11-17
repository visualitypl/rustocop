//! `Layout/SpaceBeforeFirstArg` — exactly one space between the method name
//! and the first argument of a call without parentheses.
//!
//! A port of RuboCop's cop (97f30c4, MIT) at the fixed `AllowForAlignment:
//! true`. Every call (`send` and `csend`) with arguments (a block pass
//! counts) is checked unless it is an operator method, a setter
//! (`a.b = 1`) or parenthesized. The gap is the run of spaces and tabs (then
//! line breaks) left of the first argument; a one-character gap is fine. No
//! gap at all is reported; a longer gap only when the argument starts on the
//! call's first line and is not aligned with something on an adjacent
//! line (`PrecedingFollowingAlignment`).
//!
//! The correction replaces the gap with one space, which never changes how
//! Ruby lexes the argument: the presence of a blank before it (which decides
//! unary or binary operators, `[`/`(`/`::` arguments, `?` characters, `/`
//! regexps, `%` literals and heredoc openers) stays. A missing blank is only
//! inserted before a quoted string; other insertions keep the finding
//! without an edit. Edits are withheld for the listed source
//! observers, and while `Style/MethodCallWithArgsParentheses` is selected.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceBeforeFirstArg;

const MESSAGE: &str = "Put one space between the method name and the first argument.";

/// rubocop-ast's `OPERATOR_METHODS`.
fn operator_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"|" | b"^"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b">"
            | b">="
            | b"<"
            | b"<="
            | b"<<"
            | b">>"
            | b"+"
            | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"**"
            | b"~"
            | b"+@"
            | b"-@"
            | b"!@"
            | b"~@"
            | b"[]"
            | b"[]="
            | b"!"
            | b"!="
            | b"!~"
            | b"`"
    )
}

impl Cop for SpaceBeforeFirstArg {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeFirstArg"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/MethodCallWithArgsParentheses"]
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
        let first = call
            .arguments()
            .and_then(|arguments| arguments.arguments().iter().next())
            .or_else(|| {
                call.block()
                    .filter(|b| b.as_block_argument_node().is_some())
            });
        let (Some(first), Some(selector)) = (first, call.message_loc()) else {
            return;
        };
        if operator_method(call.name().as_slice())
            || call.equal_loc().is_some()
            || call.closing_loc().is_some_and(|c| c.as_slice() == b")")
        {
            return;
        }
        let argument = first.location();
        let start = argument.start_offset();
        let mut gap = start;
        while gap > 0 && matches!(file.bytes[gap - 1], b' ' | b'\t') {
            gap -= 1;
        }
        while gap > 0 && file.bytes[gap - 1] == b'\n' {
            gap -= 1;
        }
        if start - gap == 1 {
            return;
        }
        let touching = selector.end_offset() == start;
        if !touching {
            let line = file.line_of(start);
            if line != file.line_of(node.location().start_offset()) {
                return;
            }
            let end = argument.end_offset();
            let column = file.line_col(start).1 as usize - 1;
            let end_column = file.line_col(end).1 as usize - 1;
            if file.preceding_following().aligned_with_something(
                file,
                start..end,
                line,
                column,
                end_column,
            ) {
                return;
            }
        }
        let editable = if touching {
            matches!(file.bytes.get(start), Some(b'\'' | b'"'))
        } else {
            file.bytes[gap..start]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t'))
        };
        let correction = (editable && cx.corrections_requested()).then(|| Correction {
            range: gap..start,
            replacement: " ".into(),
        });
        let (line, col) = file.line_col(gap);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MESSAGE.into(),
            line,
            end_line: line,
            col,
        });
    }
}

#[cfg(test)]
mod tests;
