//! `Layout/ClosingHeredocIndentation` — a `<<-`/`<<~` heredoc's closing line
//! is indented like its opening line or, for a heredoc used as an argument or
//! receiver, like the line starting the outermost enclosing method call.
use ruby_prism::Node;

use super::heredoc::{CallType, Role, call_parent, has_literal_block};
use crate::{
    cops::{
        Cop, NodeContext,
        naming::heredoc::{Heredoc, strip},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct ClosingHeredocIndentation;

impl Cop for ClosingHeredocIndentation {
    fn name(&self) -> &'static str {
        "Layout/ClosingHeredocIndentation"
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
        let Some(heredoc) = Heredoc::of(node, file) else {
            return;
        };
        // `heredoc_type(node) == SIMPLE_HEREDOC`: a plain `<<` must close at
        // column zero anyway.
        if !matches!(heredoc.opening.as_slice().get(2), Some(b'~' | b'-')) {
            return;
        }
        let opening_line = file.line_text(file.line_of(heredoc.opening.start_offset()));
        let closing_number = file.line_of(heredoc.closing.start_offset());
        let closing_line = file.line_text(closing_number);
        let opening = spaces(opening_line);
        let closing = spaces(closing_line);
        if opening == closing {
            return;
        }
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let parent = call_parent(node, &ancestors);
        let argument =
            parent.is_some_and(|(_, kind, role)| kind == CallType::Send && role == Role::Argument);
        let chained = parent.is_some_and(|(_, _, role)| role == Role::Receiver);
        if (argument || chained)
            && let Some((index, _, _)) = parent
            && spaces(file.line_text(
                file.line_of(outermost_send(&ancestors, index).location().start_offset()),
            )) == closing
        {
            return;
        }
        let message = format!(
            "`{}` is not aligned with `{}`{}.",
            String::from_utf8_lossy(strip(closing_line)),
            String::from_utf8_lossy(strip(opening_line)),
            if argument {
                " or beginning of method definition"
            } else {
                ""
            },
        );
        let start = file.line_start(closing_number);
        let correction = if cx.corrections_requested() {
            let root = ancestors.last().copied().unwrap_or(node);
            (!file
                .class_method_bindings(root)
                .has_listed_source_observer(file.data_start().is_some()))
            .then(|| Correction {
                range: start..start + closing,
                replacement: " ".repeat(opening),
            })
        } else {
            None
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            end_line: line,
            col,
        });
    }
}

/// `source_line[/\A */].length`: leading spaces only.
fn spaces(line: &[u8]) -> usize {
    line.iter().take_while(|&&b| b == b' ').count()
}

/// `find_node_used_heredoc_argument(node.parent)`: from the heredoc's parent
/// call, climb while the parser-gem parent is a `send`.
fn outermost_send<'a, 'pr>(ancestors: &[&'a Node<'pr>], mut index: usize) -> &'a Node<'pr> {
    loop {
        let node = ancestors[index];
        if has_literal_block(node) {
            return node;
        }
        match call_parent(node, &ancestors[index + 1..]) {
            Some((offset, CallType::Send, _)) => index += 1 + offset,
            _ => return node,
        }
    }
}

#[cfg(test)]
mod tests;
