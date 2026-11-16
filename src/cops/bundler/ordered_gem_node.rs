//! The `OrderedGemNode` projection shared by `Bundler/OrderedGems` and
//! `Gemspec/OrderedDependencies`, at the fixed
//! `TreatCommentsAsGroupSeparators: true` and `ConsiderPunctuation: false`.
//!
//! Declarations are collected in the parser gem's pre-order
//! (`def_node_search`); each consecutive pair is compared when the previous
//! `send` ends on the line just before the current one starts. A comment or
//! blank line between them therefore separates groups. Names compare
//! case-insensitively with `-` and `_` removed. Nothing is evaluated.
use ruby_prism::{CallNode, Node};

use super::declarations::{parser_order_calls, str_value, text};
use crate::{
    cops::walk::{NodeVisitor, walk},
    offense::{Correction, Offense, Severity},
    source::SourceFile,
};

/// One matched declaration: its pinned `send` span and its `str` name.
pub(crate) struct Declaration {
    /// Byte offset where the `send` starts (the offense location).
    start: usize,
    first_line: u32,
    last_line: u32,
    name: Vec<u8>,
    /// The method name (`get_dependency_name`).
    method: Vec<u8>,
}

/// Declarations whose call `matches` (receiver and method checks) and whose
/// first argument is a plain `str`, in parser pre-order.
pub(crate) fn declarations<'pr>(
    root: &Node<'pr>,
    file: &SourceFile,
    mut matches: impl FnMut(&CallNode<'pr>) -> bool,
) -> Vec<Declaration> {
    parser_order_calls(root, |call, _| {
        if !matches(call) {
            return None;
        }
        let name = str_value(&call.arguments()?.arguments().iter().next()?, file)?;
        let send_end = send_end(call);
        Some(Declaration {
            start: call.location().start_offset(),
            first_line: file.line_of(call.location().start_offset()),
            last_line: file.line_of(send_end),
            name,
            method: call.name().as_slice().to_vec(),
        })
    })
}

/// Where the pinned `send` ends: a literal block is not part of it.
pub(crate) fn send_end(call: &CallNode<'_>) -> usize {
    if call.block().is_none_or(|b| b.as_block_node().is_none()) {
        return call.location().end_offset();
    }
    if let Some(closing) = call.closing_loc() {
        closing.end_offset()
    } else if let Some(arguments) = call.arguments() {
        arguments.location().end_offset()
    } else {
        call.message_loc()
            .map_or(call.location().start_offset(), |m| m.end_offset())
    }
}

/// Whether the subtree contains a heredoc or binds a local variable
/// (assignment, multiple assignment, named capture, pattern capture).
pub(crate) fn pinned(node: &Node<'_>) -> bool {
    struct Finder(bool);
    impl<'pr> NodeVisitor<'pr> for Finder {
        fn enter(&mut self, node: Node<'pr>) {
            if matches!(
                node,
                Node::LocalVariableWriteNode { .. }
                    | Node::LocalVariableOperatorWriteNode { .. }
                    | Node::LocalVariableOrWriteNode { .. }
                    | Node::LocalVariableAndWriteNode { .. }
                    | Node::LocalVariableTargetNode { .. }
            ) {
                self.0 = true;
                return;
            }
            let opening = match &node {
                Node::StringNode { .. } => node.as_string_node().unwrap().opening_loc(),
                Node::InterpolatedStringNode { .. } => {
                    node.as_interpolated_string_node().unwrap().opening_loc()
                }
                Node::XStringNode { .. } => Some(node.as_x_string_node().unwrap().opening_loc()),
                Node::InterpolatedXStringNode { .. } => {
                    Some(node.as_interpolated_x_string_node().unwrap().opening_loc())
                }
                _ => None,
            };
            self.0 |= opening.is_some_and(|o| o.as_slice().starts_with(b"<<"));
        }
        fn leave(&mut self) {}
    }
    let mut finder = Finder(false);
    walk(&mut finder, node);
    finder.0
}

/// `gem_canonical_name` at `ConsiderPunctuation: false`: `-` and `_`
/// removed, then `downcase` (full Unicode case mapping without context, as
/// Ruby's; bytes that are not UTF-8 are folded as ASCII).
fn canonical(name: &[u8]) -> Vec<u8> {
    let name: Vec<u8> = name
        .iter()
        .copied()
        .filter(|b| *b != b'-' && *b != b'_')
        .collect();
    match std::str::from_utf8(&name) {
        Ok(text) => text
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .into_bytes(),
        Err(_) => name.to_ascii_lowercase(),
    }
}

/// The pinned investigation: every consecutive pair on adjacent lines whose
/// current name sorts before the previous one (and that `same_group`
/// accepts) is reported at the current `send`. `message` receives the
/// current and the previous name.
#[allow(clippy::too_many_arguments)]
pub(crate) fn investigate(
    cop: &'static str,
    severity: Severity,
    file: &SourceFile,
    declarations: &[Declaration],
    same_group: impl Fn(&Declaration, &Declaration) -> bool,
    message: impl Fn(&str, &str) -> String,
    out: &mut Vec<Offense>,
) {
    for pair in declarations.windows(2) {
        let (previous, current) = (&pair[0], &pair[1]);
        if previous.last_line + 1 != current.first_line
            || canonical(&current.name) >= canonical(&previous.name)
            || !same_group(previous, current)
        {
            continue;
        }
        let (line, col) = crate::cops::lint::reference_line_col(current.start, file);
        out.push(Offense {
            cop,
            severity,
            line,
            col,
            end_line: current.last_line,
            message: message(&text(&current.name), &text(&previous.name)),
            correction: swap(file, previous, current),
        });
    }
}

/// Same `get_dependency_name`.
pub(crate) fn same_method(previous: &Declaration, current: &Declaration) -> bool {
    previous.method == current.method
}

/// `OrderedGemCorrector`: swap the two declarations' whole lines; a last
/// line without a line break gets one.
fn swap(file: &SourceFile, previous: &Declaration, current: &Declaration) -> Option<Correction> {
    let lines = |d: &Declaration| file.line_start(d.first_line)..file.line_start(d.last_line + 1);
    let (previous_range, current_range) = (lines(previous), lines(current));
    let mut replacement = file.bytes[current_range.clone()].to_vec();
    if !replacement.ends_with(b"\n") {
        replacement.push(b'\n');
    }
    replacement.extend_from_slice(&file.bytes[previous_range.start..current_range.start]);
    Some(Correction {
        range: previous_range.start..current_range.end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::canonical;

    #[test]
    fn canonical_names_drop_punctuation_and_fold_case() {
        assert_eq!(canonical(b"Active-Admin_x"), b"activeadminx");
        assert!(canonical(b"a") < canonical(b"Z"));
        assert!(canonical(b"activeadmin") < canonical(b"active_admin_other_plugin"));
        assert_eq!(canonical(b"paper_trail"), canonical(b"papertrail"));
        // Ruby's downcase has no final-sigma context; neither does this.
        assert_eq!(canonical("ΑΣ".as_bytes()), "ασ".as_bytes());
        assert_eq!(canonical(b"\xFFA"), b"\xFFa");
    }
}
