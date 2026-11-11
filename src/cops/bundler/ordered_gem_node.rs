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
    cops::{
        statically_inert,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense, Severity},
    source::SourceFile,
};

/// One matched declaration: its pinned `send` span and its `str` name.
pub(crate) struct Declaration {
    /// Byte offset where the `send` starts (the offense location).
    start: usize,
    /// End of the pinned `send`: a literal block is a separate `block` node.
    send_end: usize,
    first_line: u32,
    last_line: u32,
    name: Vec<u8>,
    /// The method name (`get_dependency_name`).
    method: Vec<u8>,
    /// Whether the declaration may move as whole source lines: it is a
    /// complete statement of a statement list, with neither a literal block,
    /// a heredoc nor a local variable binding (moving a binding past a read
    /// turns the read into a method call, or back).
    movable: bool,
    /// The statement list it belongs to, by start offset.
    parent: Option<usize>,
}

/// Declarations whose call `matches` (receiver and method checks) and whose
/// first argument is a plain `str`, in parser pre-order.
pub(crate) fn declarations<'pr>(
    root: &Node<'pr>,
    file: &SourceFile,
    mut matches: impl FnMut(&CallNode<'pr>) -> bool,
) -> Vec<Declaration> {
    parser_order_calls(root, |call, ancestors| {
        if !matches(call) {
            return None;
        }
        let name = str_value(&call.arguments()?.arguments().iter().next()?, file)?;
        let send_end = send_end(call);
        let literal_block = call.block().is_some_and(|b| b.as_block_node().is_some());
        let parent = ancestors
            .last()
            .and_then(|node| node.as_statements_node())
            .map(|list| list.location().start_offset());
        Some(Declaration {
            start: call.location().start_offset(),
            send_end,
            first_line: file.line_of(call.location().start_offset()),
            last_line: file.line_of(send_end),
            name,
            method: call.name().as_slice().to_vec(),
            movable: parent.is_some() && !literal_block && !pinned(&call.as_node()),
            parent,
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
    root: &Node<'_>,
    file: &SourceFile,
    declarations: &[Declaration],
    same_group: impl Fn(&Declaration, &Declaration) -> bool,
    message: impl Fn(&str, &str) -> String,
    out: &mut Vec<Offense>,
) {
    let mut editable = None;
    for pair in declarations.windows(2) {
        let (previous, current) = (&pair[0], &pair[1]);
        if previous.last_line + 1 != current.first_line
            || canonical(&current.name) >= canonical(&previous.name)
            || !same_group(previous, current)
        {
            continue;
        }
        let (line, col) = crate::cops::lint::reference_line_col(current.start, file);
        let editable = *editable.get_or_insert_with(|| inactive_program(root, file));
        out.push(Offense {
            cop,
            severity,
            line,
            col,
            end_line: current.last_line,
            message: message(&text(&current.name), &text(&previous.name)),
            correction: editable.then(|| swap(file, previous, current)).flatten(),
        });
    }
}

/// Same `get_dependency_name`.
pub(crate) fn same_method(previous: &Declaration, current: &Declaration) -> bool {
    previous.method == current.method
}

// Swapping two declarations reorders observable DSL calls (`gem` order is
// Bundler's require order; a gemspec's dependency order is its metadata
// order), so a live file never gets the edit. The upstream canonical swap is
// proven only where nothing runs: the program's sole statement is a closed
// top-level `if false ... end` and there is no data section. Shared
// main-parse guards (listed observers, positions, heredocs, atomic writes)
// still decide whether the proposed edit is applied.
// A listed source observer holds every edit, even in dead code.
// The gemspec alias rename and deprecated-attribute removal reuse this
// envelope for the same reason (unknown receivers, observable DSL calls).
pub(crate) fn inactive_program(root: &Node<'_>, file: &SourceFile) -> bool {
    file.data_start().is_none()
        && statically_inert(root)
        && !file
            .class_method_bindings(root)
            .has_listed_source_observer(false)
}

/// `OrderedGemCorrector`: swap the two declarations' whole lines (trailing
/// comments and line endings travel with them). Only declarations that are
/// complete statements of the same list, alone on their lines, without a
/// literal block, heredoc or local binding, are moved; anything else keeps
/// its bytes.
fn swap(file: &SourceFile, previous: &Declaration, current: &Declaration) -> Option<Correction> {
    if !previous.movable || !current.movable || previous.parent != current.parent {
        return None;
    }
    let previous_range = whole_lines(file, previous)?;
    let current_range = whole_lines(file, current)?;
    if previous_range.end != current_range.start
        || !file.bytes[..current_range.end].ends_with(b"\n")
    {
        return None;
    }
    let mut replacement = file.bytes[current_range.clone()].to_vec();
    replacement.extend_from_slice(&file.bytes[previous_range.clone()]);
    Some(Correction {
        range: previous_range.start..current_range.end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

/// The declaration's lines, final line break included, when only blanks
/// precede it and only blanks or a comment follow it on those lines.
fn whole_lines(file: &SourceFile, declaration: &Declaration) -> Option<std::ops::Range<usize>> {
    let start = file.line_start(declaration.first_line);
    let end = file.line_start(declaration.last_line + 1);
    let before = &file.bytes[start..declaration.start];
    let after = &file.bytes[declaration.send_end..end];
    let after = after.strip_suffix(b"\n").unwrap_or(after);
    let after = after.strip_suffix(b"\r").unwrap_or(after);
    let code = after
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .map_or(&[][..], |i| &after[i..]);
    (before.iter().all(|b| matches!(b, b' ' | b'\t'))
        && (code.is_empty() || code.starts_with(b"#")))
    .then_some(start..end)
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
