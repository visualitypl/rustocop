//! `Layout/SpaceBeforeBlockBraces` — a blank before the `{` of a block.
//!
//! A port of RuboCop's cop (97f30c4, MIT) at the fixed `EnforcedStyle:
//! space` and `EnforcedStyleForEmptyBraces: space`. `on_block`,
//! `on_numblock` and `on_itblock` see every brace block (numbered-parameter
//! and `it` blocks, `super` blocks and safe-navigation calls included) and
//! every brace lambda literal (`->{}`): Prism's `BlockNode` and `LambdaNode`
//! whose opening is `{`; `do ... end` is skipped. Upstream's
//! `range_with_surrounding_space` looks left of the brace over spaces and
//! tabs, then line breaks, so the brace is reported (`Space missing to the
//! left of {.`) when the byte before it is none of those. Empty braces get
//! the same check (both fixed styles are `space`), and upstream's
//! `Style/BlockDelimiters` `line_count_based` exemption applies only to
//! `no_space`.
//!
//! The correction inserts one space before the brace: a brace binds to the
//! same call with or without a blank before it (native Prism gives the same
//! tokens and AST for `a{}`/`a {}` with a local `a`, `foo a{}`, `A{}`,
//! `A::B{}`, `f!{}`, `super{}`, `a&.b{}`, `->{}` and `->(a){}`). It is
//! withheld for the listed source observers. Upstream declares
//! `Style/BlockDelimiters` and `Style/SymbolProc` autocorrect-incompatible
//! and its Team skips this cop's corrector in a pass where one of them
//! already corrected; [`Cop::autocorrect_deferred_to`] does the same per
//! file and pass (a converted `{` becomes `do`), so default runs, where both
//! are active, still converge. `autocorrect_incompatible_with` stays empty:
//! it would withhold the edit whenever either companion is merely active.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceBeforeBlockBraces;

const MSG: &str = "Space missing to the left of {.";

impl Cop for SpaceBeforeBlockBraces {
    fn name(&self) -> &'static str {
        "Layout/SpaceBeforeBlockBraces"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_deferred_to(&self) -> &'static [&'static str] {
        &["Style/BlockDelimiters", "Style/SymbolProc"]
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let opening = match node {
            Node::BlockNode { .. } => node.as_block_node().unwrap().opening_loc(),
            Node::LambdaNode { .. } => node.as_lambda_node().unwrap().opening_loc(),
            _ => return,
        };
        if opening.as_slice() != b"{" {
            return;
        }
        let brace = opening.start_offset();
        if brace == 0 || matches!(file.bytes[brace - 1], b' ' | b'\t' | b'\n') {
            return;
        }
        let correction = (cx.corrections_requested()).then(|| Correction {
            range: brace..brace,
            replacement: " ".into(),
        });
        let (line, col) = file.line_col(brace);
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
