//! Shared implicit-parameter block shapes. The multiline policy compares
//! delimiters, not the receiver's first physical line (matching RuboCop).
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};
pub(super) struct Block<'pr> {
    pub parameters: Option<Node<'pr>>,
    pub body: Option<Node<'pr>>,
    pub opening: Location<'pr>,
    pub closing: Location<'pr>,
}
pub(super) fn block<'pr>(node: &Node<'pr>) -> Option<Block<'pr>> {
    if let Some(b) = node.as_block_node() {
        Some(Block {
            parameters: b.parameters(),
            body: b.body(),
            opening: b.opening_loc(),
            closing: b.closing_loc(),
        })
    } else {
        node.as_lambda_node().map(|b| Block {
            parameters: b.parameters(),
            body: b.body(),
            opening: b.opening_loc(),
            closing: b.closing_loc(),
        })
    }
}
pub(super) fn report(
    cop: &dyn Cop,
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
    message: &str,
) {
    let owner = if node.as_block_node().is_some() {
        cx.parent().unwrap_or(node)
    } else {
        node
    };
    let (line, col) = file.line_col(owner.location().start_offset());
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message: message.into(),
        line,
        col,
        end_line: file.last_line_of(node.location()),
        correction: None,
    });
}
/// Is this read a descendant, not the body root? Prism's StatementsNode is
/// transparent; explicit parentheses retain RuboCop's begin wrapper.
pub(super) fn body_root(body: Option<Node<'_>>, node: &Node<'_>) -> bool {
    let Some(body) = body else {
        return false;
    };
    let root = body
        .as_statements_node()
        .and_then(|s| (s.body().len() == 1).then(|| s.body().first()).flatten())
        .unwrap_or(body);
    std::mem::discriminant(&root) == std::mem::discriminant(node)
        && root.location().start_offset() == node.location().start_offset()
        && root.location().end_offset() == node.location().end_offset()
}
