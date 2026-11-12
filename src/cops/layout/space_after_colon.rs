//! `Layout/SpaceAfterColon` — a colon of a `key: value` pair or of an
//! optional keyword parameter must be followed by a blank.
//!
//! A port of RuboCop's cop (97f30c4, MIT). `on_pair` checks a colon-style
//! pair (`a: 1`, `"a": 1`, in a hash, keyword arguments or a hash pattern)
//! unless its value is omitted (`{a:}`, `in {a:}`); the colon is the last
//! byte of the key's label. `on_kwoptarg` checks the colon after an optional
//! keyword parameter's name (`def m(a:1)`, block and lambda parameters too).
//! The colon is reported (`Space missing after colon.`) unless the next
//! character is a Ruby `\s` (space, tab, newline, carriage return, vertical
//! tab, form feed). Required keyword parameters, symbols, ternaries and
//! hash rockets are not colon pairs.
//!
//! The correction inserts one space after the colon: a label (or keyword
//! parameter name) is a complete token ending in its colon, and a value
//! after a label is lexed at the beginning of an expression with or without
//! a blank, so only the gap between two tokens changes. It is withheld for
//! the listed source observers.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAfterColon;

const MSG: &str = "Space missing after colon.";

/// Ruby's regexp `\s`.
pub(super) fn ruby_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// Whether the file has one of the listed source observers.
pub(super) fn observed(cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    cx.ancestors().next_back().is_none_or(|root| {
        file.class_method_bindings(root)
            .has_listed_source_observer(file.data_start().is_some())
    })
}

/// The offset of the colon to check, if `node` is a colon pair with a value
/// or an optional keyword parameter.
fn colon(node: &Node<'_>) -> Option<usize> {
    if let Some(pair) = node.as_assoc_node() {
        if pair.operator_loc().is_some() || pair.value().as_implicit_node().is_some() {
            return None;
        }
        return pair.key().location().end_offset().checked_sub(1);
    }
    let parameter = node.as_optional_keyword_parameter_node()?;
    parameter.name_loc().end_offset().checked_sub(1)
}

impl Cop for SpaceAfterColon {
    fn name(&self) -> &'static str {
        "Layout/SpaceAfterColon"
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
        let Some(colon) = colon(node) else {
            return;
        };
        if file.bytes.get(colon) != Some(&b':') {
            return;
        }
        if file
            .bytes
            .get(colon + 1)
            .is_none_or(|byte| ruby_space(*byte))
        {
            return;
        }
        let correction = (cx.corrections_requested() && !observed(cx, file)).then(|| Correction {
            range: colon + 1..colon + 1,
            replacement: " ".into(),
        });
        let (line, col) = file.line_col(colon);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line: line,
            col,
        });
    }
}

#[cfg(test)]
mod tests;
