//! A narrow port of RuboCop's `FirstElementLineBreak` mixin (97f30c4, MIT)
//! for the fixed `AllowMultilineFinalElement: false`, shared by the array,
//! hash, method argument and method parameter line-break cops.
//!
//! `check_children_line_break`: the element starting on the earliest line
//! (the first one on ties) is reported when it starts on the collection's
//! start line and some element ends on a later line. The correction is
//! upstream's `EmptyLineCorrector.insert_before(first, "\n")`: one line
//! break inserted before the reported element, nothing else moved, even in
//! CRLF files. It is withheld for the listed source observers;
//! line-number observers after the insertion and heredoc bodies that would
//! start at the inserted break are held by the shared `run_cops` boundary.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

/// The element to report: the earliest-starting element when it starts on
/// `start_line` and the elements reach a later line.
pub(crate) fn first_element<'a, 'pr>(
    file: &SourceFile,
    start_line: u32,
    elements: &'a [Node<'pr>],
) -> Option<&'a Node<'pr>> {
    // `min_by(&:first_line)` keeps the first of equally early elements.
    let first = elements
        .iter()
        .min_by_key(|element| file.line_of(element.location().start_offset()))?;
    if file.line_of(first.location().start_offset()) != start_line {
        return None;
    }
    let last_line = elements
        .iter()
        .map(|element| file.last_line_of(element.location()))
        .max()?;
    (last_line != start_line).then_some(first)
}

/// `method_uses_parens?`: the text of the node's first line, up to the first
/// element's column, ends with `(` and blanks (`/\s*\(\s*$/`). Upstream
/// slices the node's first line at the element's column even when the
/// element starts on a later line; such an element never starts on the
/// node's first line, so `first_element` rejects it either way.
pub(crate) fn method_uses_parens(file: &SourceFile, node_start: usize, first: usize) -> bool {
    let line = file.line_of(node_start);
    if file.line_of(first) != line {
        return false;
    }
    let mut prefix = &file.bytes[file.line_start(line)..first];
    // Ruby's `\s`: space, `\t`, `\n`, `\v`, `\f` and `\r`.
    while let [rest @ .., b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'] = prefix {
        prefix = rest;
    }
    prefix.ends_with(b"(")
}

/// Report `element` with `message`, offering the guarded line-break insertion
/// before it when corrections are requested.
pub(crate) fn report(
    cop: &dyn Cop,
    message: &str,
    element: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let location = element.location();
    let start = location.start_offset();
    let observed = || {
        cx.ancestors().next_back().is_none_or(|root| {
            file.class_method_bindings(root)
                .has_listed_source_observer(file.data_start().is_some())
        })
    };
    let correction = (cx.corrections_requested() && !observed()).then(|| Correction {
        range: start..start,
        replacement: "\n".to_owned(),
    });
    let (line, col) = file.line_col(start);
    out.push(Offense {
        correction,
        cop: cop.name(),
        severity: cop.severity(),
        message: message.to_owned(),
        line,
        end_line: file.last_line_of(location),
        col,
    });
}
