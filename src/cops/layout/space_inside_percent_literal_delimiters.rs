//! `Layout/SpaceInsidePercentLiteralDelimiters` — no spaces just inside the
//! delimiters of a `%i`/`%I`/`%w`/`%W` array or a `%x` command.
//!
//! A port of RuboCop's cop (97f30c4, MIT). `on_array` and `on_xstr` through
//! the `PercentLiteral` mixin: a Prism `ArrayNode` opened by `%i`, `%I`,
//! `%w` or `%W` and an `XStringNode` or `InterpolatedXStringNode` opened by
//! `%x`, each with its delimiter (backticks, `<<~`CMD`` heredocs, `[...]`
//! arrays and `%q`/`%Q`/`%r`/`%s` literals are not checked). Reported
//! (`Do not use spaces inside percent literal delimiters.`), in upstream's
//! order and once per position: `Base#add_offense` drops a repeated range,
//! and upstream's runner keeps one offense per line, column, cop and
//! message, so the leading spaces of a blank body (`%w( \t )`) are not
//! reported a second time at the body's start:
//!
//! - a blank body, non-empty with only whitespace between the delimiters
//!   (`String#strip` empties it), as a whole, multi-line literals included;
//! - in a single-line literal, the spaces right after the opening delimiter
//!   (`/\A( +)/`) and the spaces right before the closing one that do not
//!   start right after a `\` (`/(?<!\\)( +)\z/`, per source character, so
//!   the second byte of a legacy double-byte character is no backslash).
//!   Tabs are not spaces here.
//!
//! The corrections remove each reported range, as RuboCop's do.
use std::ops::Range;

use ruby_prism::Node;

use super::space_inside_array_percent_literal::{characters, percent_literal};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceInsidePercentLiteralDelimiters;

const MSG: &str = "Do not use spaces inside percent literal delimiters.";

/// `String#strip` whitespace.
fn strip_space(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' | b' ')
}

impl Cop for SpaceInsidePercentLiteralDelimiters {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsidePercentLiteralDelimiters"
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
        let types: &[&[u8]] = if node.as_array_node().is_some() {
            &[b"%i", b"%I", b"%w", b"%W"]
        } else {
            &[b"%x"]
        };
        let Some(literal) = percent_literal(node, file, types) else {
            return;
        };
        let contents = literal.contents.clone();
        let body = &file.bytes[contents.clone()];
        // (range, editable)
        let mut found: Vec<(Range<usize>, bool)> = Vec::new();
        let blank = !body.is_empty() && body.iter().all(|b| strip_space(*b));
        if blank {
            // NUL is stripped but is an element: only an array Prism sees
            // as empty loses its body.
            found.push((contents.clone(), literal.elements.is_empty()));
        }
        if literal.single_line {
            let chars = characters(file, contents.clone());
            let leading = chars.iter().take_while(|c| c.space()).count();
            if leading > 0 {
                let end = chars.get(leading).map_or(contents.end, |c| c.start);
                let range = contents.start..end;
                let editable = !blank
                    && literal
                        .elements
                        .first()
                        .is_none_or(|first| first.start >= range.end);
                found.push((range, editable));
            }
            let trailing = chars.iter().rev().take_while(|c| c.space()).count();
            let mut first = chars.len() - trailing;
            if first > 0 && chars[first - 1].backslash() {
                first += 1;
            }
            if first < chars.len() {
                let range = chars[first].start..contents.end;
                let editable = !blank
                    && literal
                        .elements
                        .last()
                        .is_none_or(|last| last.end <= range.start);
                found.push((range, editable));
            }
        }
        if found.is_empty() {
            return;
        }
        // RuboCop removes each reported range.
        let edit = cx.corrections_requested();
        let mut reported: Vec<usize> = Vec::new();
        for (range, _) in found {
            if reported.contains(&range.start) {
                continue;
            }
            reported.push(range.start);
            let (line, col) = file.line_col(range.start);
            let end_line = file.line_of(range.end.saturating_sub(1).max(range.start));
            out.push(Offense {
                correction: edit.then(|| Correction {
                    range,
                    replacement: String::new(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: MSG.into(),
                line,
                end_line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests;
