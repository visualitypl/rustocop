//! `Layout/SpaceInsideStringInterpolation` — no blank just inside the
//! delimiters of a string interpolation.
//!
//! A port of RuboCop's cop (97f30c4, MIT) at the fixed `EnforcedStyle:
//! no_space`. The inherited `Interpolation` callbacks (`on_dstr`,
//! `on_xstr`, `on_dsym`, `on_regexp`) call `on_interpolation` for every
//! `begin` child: Prism's `EmbeddedStatementsNode`, `#{...}` in a string,
//! heredoc, symbol, regexp (match-last-line included), backtick or `%x`
//! command, or `%W`/`%I` element (`#@a` is an embedded variable and is not
//! checked). An interpolation whose `#{` and `}` are on different lines is
//! accepted, as is an empty one (the parser-gem tokens `#{` and `}` are
//! adjacent: nothing but Ruby's layout whitespace between them). Otherwise
//! the spaces and tabs directly after `#{` and directly before `}` are
//! each reported (`Do not use space inside string interpolation.`) at their
//! range, as `SurroundingSpace#no_space_offenses`.
//!
//! The runs never enter the interpolated statements: a blank that ends a
//! token (`?\ `, the delimiters of `% x `) is adjacent to `}` in upstream's
//! tokens and is not reported.
//!
//! The correction removes each reported run (`SpaceCorrector.remove_space`).
//! The runs are layout between the `#{`/`}` delimiters and the
//! interpolated code, never string content: `#{` leaves the lexer at the
//! start of an expression and `}` closes the interpolation with or without
//! a blank, so the string's value, the code's tokens and the encoding are
//! kept; the edit removes only ASCII blanks. The listed source
//! observers hold every edit.
use ruby_prism::Node;

use super::space_after_colon::observed;
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceInsideStringInterpolation;

const MSG: &str = "Do not use space inside string interpolation.";

fn blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

impl Cop for SpaceInsideStringInterpolation {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideStringInterpolation"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_embedded_statements_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(interpolation) = node.as_embedded_statements_node() else {
            return;
        };
        let open = interpolation.opening_loc().end_offset();
        let close = interpolation.closing_loc().start_offset();
        if open > close || file.line_of(open) != file.line_of(close) {
            return;
        }
        let inside = &file.bytes[open..close];
        // Ruby's layout whitespace: the parser-gem tokens are adjacent.
        if inside
            .iter()
            .all(|b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r'))
        {
            return;
        }
        // The runs stop at the interpolated code: a blank inside its last
        // token (`?\ `, `% x `) is adjacent to `}` in upstream's tokens.
        let (code_start, code_end) = interpolation
            .statements()
            .map(|s| {
                let loc = s.as_node().location();
                (loc.start_offset(), loc.end_offset())
            })
            .filter(|&(start, end)| open <= start && start <= end && end <= close)
            .unwrap_or((close, open));
        let left = file.bytes[open..code_start.max(open)]
            .iter()
            .take_while(|b| blank(**b))
            .count();
        let right = file.bytes[code_end.min(close)..close]
            .iter()
            .rev()
            .take_while(|b| blank(**b))
            .count();
        let edit = cx.corrections_requested() && !observed(cx, file);
        for range in [open..open + left, close - right..close] {
            if range.is_empty() {
                continue;
            }
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                correction: edit.then(|| Correction {
                    range,
                    replacement: String::new(),
                }),
                cop: self.name(),
                severity: self.severity(),
                message: MSG.into(),
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests;
