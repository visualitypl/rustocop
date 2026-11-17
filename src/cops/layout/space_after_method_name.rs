//! `Layout/SpaceAfterMethodName` — no space between a method name and the
//! opening parenthesis of its parameters in a definition.
//!
//! A port of RuboCop's cop (97f30c4, MIT). `on_def` and `on_defs` check a
//! `def`/`def self.`/`def obj.` (endless definitions included) whose
//! parameters are parenthesized: when the character right before the `(` is
//! a space (only `' '`, as upstream's `start_with?(' ')`), that one space is
//! reported (`Do not put a space between a method name and the opening
//! parenthesis.`). A tab, or a definition without parenthesized parameters,
//! is accepted.
//!
//! The correction removes that one space (a run of spaces is corrected one
//! space per pass, as upstream): the method name is a complete token, and
//! Ruby reads a definition's parenthesized parameter list the same with or
//! without the blank, so only the gap between two tokens changes. It is
//! withheld for the listed source observers.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAfterMethodName;

const MSG: &str = "Do not put a space between a method name and the opening parenthesis.";

impl Cop for SpaceAfterMethodName {
    fn name(&self) -> &'static str {
        "Layout/SpaceAfterMethodName"
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
        let Some(paren) = node
            .as_def_node()
            .and_then(|def| def.lparen_loc())
            .map(|paren| paren.start_offset())
        else {
            return;
        };
        let Some(space) = paren.checked_sub(1) else {
            return;
        };
        if file.bytes[space] != b' ' {
            return;
        }
        let correction = (cx.corrections_requested()).then(|| Correction {
            range: space..paren,
            replacement: String::new(),
        });
        let (line, col) = file.line_col(space);
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
