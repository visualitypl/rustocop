//! `Layout/EmptyLinesAroundBeginBody` — no padding inside `begin`/`end`.
//!
//! Upstream's `on_kwbegin` checks the lines after `begin` and before `end`
//! at the fixed `no_empty_lines` style; nested `rescue`/`else`/`ensure`
//! keywords are not body edges.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct EmptyLinesAroundBeginBody;

impl Cop for EmptyLinesAroundBeginBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundBeginBody"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_begin_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Only an explicit `begin` keyword is a parser `kwbegin`; Prism also
        // uses BeginNode for implicit method, block and class bodies.
        let Some(begin) = node.as_begin_node() else {
            return;
        };
        let Some(keyword) = begin.begin_keyword_loc() else {
            return;
        };
        super::body_padding::check(
            self,
            "`begin`",
            (
                file.line_of(keyword.start_offset()),
                file.last_line_of(node.location()),
            ),
            cx,
            file,
            out,
        );
    }
}

#[cfg(test)]
mod tests;
