//! `Layout/FirstArrayElementLineBreak` (opt-in) — a multi-line array whose
//! first element starts on its opening line.
//!
//! A port of RuboCop's `on_array` (97f30c4, MIT) at the fixed defaults
//! `AllowImplicitArrayLiterals: false` and `AllowMultilineFinalElement:
//! false`. Every Prism `ArrayNode` (the parser's `array`) is checked when it
//! has an opening (`[`, `%w(`, `%i[`, ...) or when it is an implicit array
//! whose start line holds nothing but blanks after an `=` before it
//! (`x = 1,\n  2`, `a, b = 1,\n  2`, `a.b = 1,\n  2`). While
//! `Layout/SpaceInsideArrayLiteralBrackets` is enabled with
//! `enforced_style = "compact"`, a bracketed array whose first element's
//! source starts with `[` is exempt. The shared `FirstElementLineBreak` projection
//! (`first_element_line_break`) decides the finding and the guarded
//! line-break insertion before the first element.
use ruby_prism::Node;

use super::first_element_line_break::{first_element, report};
use crate::{
    cops::{Companions, Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

#[derive(Default)]
pub struct FirstArrayElementLineBreak {
    /// `compact_array_brackets_enforced?`.
    compact_brackets: bool,
}

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

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_array_node().is_some()
    }

    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(FirstArrayElementLineBreak {
            compact_brackets: companions.compact_array_brackets,
        }))
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
        // `compact_nested_array?`.
        if self.compact_brackets
            && array.opening_loc().is_some_and(|o| o.as_slice() == b"[")
            && elements
                .first()
                .is_some_and(|e| e.location().as_slice().starts_with(b"["))
        {
            return;
        }
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
