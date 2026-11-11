//! `Layout/FirstArrayElementLineBreak` (opt-in) — a multi-line array whose
//! first element starts on its opening line.
//!
//! A port of RuboCop's `on_array` (97f30c4, MIT) at the fixed defaults
//! `AllowImplicitArrayLiterals: false` and `AllowMultilineFinalElement:
//! false`. Every Prism `ArrayNode` (the parser's `array`) is checked when it
//! has an opening (`[`, `%w(`, `%i[`, ...) or when it is an implicit array
//! whose start line holds nothing but blanks after an `=` before it
//! (`x = 1,\n  2`, `a, b = 1,\n  2`, `a.b = 1,\n  2`). Upstream's
//! compact-nested-array exemption needs `Layout/SpaceInsideArrayLiteralBrackets`
//! with `EnforcedStyle: compact`; that cop is fixed at `no_space`, so the
//! exemption never applies. The shared `FirstElementLineBreak` projection
//! (`first_element_line_break`) decides the finding and the guarded
//! line-break insertion before the first element.
use ruby_prism::Node;

use super::first_element_line_break::{first_element, report};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct FirstArrayElementLineBreak;

const MSG: &str = "Add a line break before the first element of a multi-line array.";

impl Cop for FirstArrayElementLineBreak {
    fn name(&self) -> &'static str {
        "Layout/FirstArrayElementLineBreak"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    /// Opt-in as upstream: only its exact name in `--only` or its own
    /// `enabled = true` runs it, never a department selection.
    fn department_selection_enables(&self) -> bool {
        false
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(array) = node.as_array_node() else {
            return;
        };
        let start = node.location().start_offset();
        if array.opening_loc().is_none() && !assignment_on_same_line(file, start) {
            return;
        }
        let elements: Vec<_> = array.elements().iter().collect();
        if let Some(first) = first_element(file, file.line_of(start), &elements) {
            report(self, MSG, first, cx, file, out);
        }
    }
}

/// Upstream's `/\s*=\s*$/` on the start line's text before the array.
fn assignment_on_same_line(file: &SourceFile, start: usize) -> bool {
    let line_start = file.line_start(file.line_of(start));
    let mut prefix = &file.bytes[line_start..start];
    // Ruby's `\s`: space, `\t`, `\n`, `\v`, `\f` and `\r`.
    while let [rest @ .., b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'] = prefix {
        prefix = rest;
    }
    prefix.ends_with(b"=")
}

#[cfg(test)]
mod tests;
