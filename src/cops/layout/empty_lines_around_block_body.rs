//! `Layout/EmptyLinesAroundBlockBody` — no padding inside block bodies.
//!
//! Upstream's `on_block`/`on_numblock`/`on_itblock` (including lambda
//! literals, which the parser gem represents as blocks) at the fixed
//! `no_empty_lines` style. The body starts after the opening `{`/`do`, or
//! after explicit block parameters that end on a later line; numbered and
//! `it` parameters are implicit.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};

pub struct EmptyLinesAroundBlockBody;

impl Cop for EmptyLinesAroundBlockBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundBlockBody"
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
        let (opening, parameters) = if let Some(block) = node.as_block_node() {
            (block.opening_loc(), block.parameters())
        } else if let Some(lambda) = node.as_lambda_node() {
            (lambda.opening_loc(), lambda.parameters())
        } else {
            return;
        };
        let mut first = file.line_of(opening.start_offset());
        if let Some(parameters) = parameters.as_ref().and_then(explicit_parameters) {
            first = first.max(file.last_line_of(parameters));
        }
        let start = out.len();
        super::body_padding::check(
            self,
            "block",
            (first, file.last_line_of(node.location())),
            cx,
            file,
            out,
        );
        super::body_padding::hold_for_listed_observers(cx, file, &mut out[start..]);
    }
}

/// The location of non-empty explicit block parameters (`|a|`, `|;b|`,
/// `(a)`); `||`, `()` and the implicit numbered/`it` parameters have no
/// parser arguments.
fn explicit_parameters<'pr>(node: &Node<'pr>) -> Option<Location<'pr>> {
    let block = node.as_block_parameters_node()?;
    (block.parameters().is_some() || block.locals().iter().next().is_some())
        .then(|| node.location())
}

#[cfg(test)]
mod tests;
