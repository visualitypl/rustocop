//! `Layout/BlockAlignment` — align a block's closing `end`/`}` with the start
//! of the expression or the start of the line holding `do`/`{` (fixed
//! `EnforcedStyleAlignWith: either`).
//!
//! RuboCop walks parser-gem nodes. Here the lineage is projected onto Prism:
//! a parser `block` is the Prism call (or lambda/super) owning the literal
//! block; `ArgumentsNode` and single-statement `StatementsNode` wrappers do not
//! exist in the parser tree, while a multi-statement body is its `begin` node.

use std::ops::Range;

use ruby_prism::{Location, Node};

use super::alignment::end_indentation;
use crate::{
    cops::{Cop, NodeContext, walk},
    offense::Offense,
    source::SourceFile,
};

pub struct BlockAlignment;

impl Cop for BlockAlignment {
    fn name(&self) -> &'static str {
        "Layout/BlockAlignment"
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
        let Some(block) = Block::of(node) else {
            return;
        };
        let end = block.close.start_offset();
        if !begins_its_line(file, end) {
            return;
        }
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let (depth, target) = align_target(file, node, &ancestors);
        let start = lhs(target);
        let (start_line, start_col) = file.line_col(start.start);
        let (end_line, end_col) = file.line_col(end);
        if start_col == end_col {
            return;
        }
        let do_anchor = do_line_anchor(file, node, &block);
        let (do_line, do_col) = file.line_col(first_non_blank(file, do_anchor));
        let (_, open_line_col) = file.line_col(first_non_blank(file, block.open.start_offset()));
        if end_col == do_col || end_col == open_line_col {
            return;
        }
        let alternative = if (start_line, start_col) == (do_line, do_col) {
            String::new()
        } else {
            format!(
                " or `{}` at {do_line}, {}",
                rest_of_line(file, first_non_blank(file, do_anchor)),
                do_col - 1
            )
        };
        let message = format!(
            "`{}` at {end_line}, {} is not aligned with `{}` at {start_line}, {}{alternative}.",
            String::from_utf8_lossy(block.close.as_slice()),
            end_col - 1,
            first_line(file, &start),
            start_col - 1,
        );
        let correction = cx
            .corrections_requested()
            .then(|| {
                let column = file
                    .line_col(line_anchor(file, target, &ancestors[depth..]))
                    .1;
                let line_start = end - (end_col as usize - 1);
                let prefix = &file.bytes[line_start..end];
                let indentation = if column > end_col {
                    let mut s = String::from_utf8_lossy(prefix).into_owned();
                    s.push_str(&" ".repeat((column - end_col) as usize));
                    s
                } else {
                    let keep = prefix.len() - (end_col - column) as usize;
                    String::from_utf8_lossy(&prefix[..keep]).into_owned()
                };
                let root = ancestors.last().copied().unwrap_or(node);
                end_indentation(file, root, end, &indentation)
            })
            .flatten();
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message,
            line: end_line,
            end_line,
            col: end_col,
        });
    }
}

/// A parser-gem `block`, `numblock` or `itblock` node.
struct Block<'pr> {
    open: Location<'pr>,
    close: Location<'pr>,
    /// The send node's selector, or the send node itself without one.
    selector: usize,
    /// The send node's arguments and the block's own parameters.
    arguments: Vec<Range<usize>>,
}

impl<'pr> Block<'pr> {
    fn of(node: &Node<'pr>) -> Option<Self> {
        let range = |n: Node<'pr>| n.location().start_offset()..n.location().end_offset();
        let mut arguments = Vec::new();
        let (block, selector) = match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                let block = call.block()?.as_block_node()?;
                arguments.extend(
                    call.arguments()
                        .into_iter()
                        .flat_map(|a| a.arguments().iter().map(range)),
                );
                let selector = call
                    .message_loc()
                    .map_or(node.location().start_offset(), |l| l.start_offset());
                (block, selector)
            }
            Node::SuperNode { .. } => {
                let call = node.as_super_node().unwrap();
                let block = call.block()?.as_block_node()?;
                arguments.extend(
                    call.arguments()
                        .into_iter()
                        .flat_map(|a| a.arguments().iter().map(range)),
                );
                (block, call.keyword_loc().start_offset())
            }
            Node::ForwardingSuperNode { .. } => {
                let call = node.as_forwarding_super_node().unwrap();
                (call.block()?, node.location().start_offset())
            }
            Node::LambdaNode { .. } => {
                let lambda = node.as_lambda_node().unwrap();
                parameters(lambda.parameters(), &mut arguments);
                return Some(Self {
                    open: lambda.opening_loc(),
                    close: lambda.closing_loc(),
                    selector: lambda.operator_loc().start_offset(),
                    arguments,
                });
            }
            _ => return None,
        };
        parameters(block.parameters(), &mut arguments);
        Some(Self {
            open: block.opening_loc(),
            close: block.closing_loc(),
            selector,
            arguments,
        })
    }
}

/// The children of the parser gem's `args` node for explicit parameters.
fn parameters(node: Option<Node<'_>>, out: &mut Vec<Range<usize>>) {
    let Some(params) = node.and_then(|n| n.as_block_parameters_node()) else {
        return;
    };
    let range = |n: Node<'_>| n.location().start_offset()..n.location().end_offset();
    if let Some(p) = params.parameters() {
        out.extend(p.requireds().iter().map(range));
        out.extend(p.optionals().iter().map(range));
        out.extend(p.rest().map(range));
        out.extend(p.posts().iter().map(range));
        out.extend(p.keywords().iter().map(range));
        out.extend(p.keyword_rest().map(range));
        out.extend(p.block().map(|b| range(b.as_node())));
    }
    out.extend(params.locals().iter().map(range));
}

/// `begins_its_line?`: only Ruby whitespace precedes the offset on its line.
fn begins_its_line(file: &SourceFile, offset: usize) -> bool {
    let start = line_start(file, offset);
    file.bytes[start..offset]
        .iter()
        .all(|b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r'))
}

fn line_start(file: &SourceFile, offset: usize) -> usize {
    let line = file.line_of(offset);
    let start = file.line_start(line);
    if line == 1 && file.bytes.starts_with(b"\xef\xbb\xbf") {
        start.max(3)
    } else {
        start
    }
}

/// The first non-whitespace byte on the line holding `offset`.
fn first_non_blank(file: &SourceFile, offset: usize) -> usize {
    let start = line_start(file, offset);
    start
        + file.bytes[start..offset]
            .iter()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r'))
            .count()
}

/// Ruby's `/\S.*/` match from a first non-blank character.
fn rest_of_line(file: &SourceFile, offset: usize) -> String {
    let line = file.line_text(file.line_of(offset));
    let skip = offset - file.line_start(file.line_of(offset));
    String::from_utf8_lossy(&line[skip.min(line.len())..]).into_owned()
}

/// `loc.source.lines.first.chomp`.
fn first_line(file: &SourceFile, range: &Range<usize>) -> String {
    let text = &file.bytes[range.clone()];
    let text = text.split(|b| *b == b'\n').next().unwrap_or_default();
    String::from_utf8_lossy(text.strip_suffix(b"\r").unwrap_or(text)).into_owned()
}

fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    let (a, b) = (a.location(), b.location());
    (a.start_offset(), a.end_offset()) == (b.start_offset(), b.end_offset())
}

/// `block_end_align_target`: climb while the parent starts on the same line
/// (or is a multiple assignment) and is an assignment, a definition, a splat,
/// `&&`/`||`, a `<<` send or a send on the current node (other than `[]`).
/// Returns the number of ancestors consumed and the target node.
fn align_target<'a, 'pr>(
    file: &SourceFile,
    node: &'a Node<'pr>,
    ancestors: &[&'a Node<'pr>],
) -> (usize, &'a Node<'pr>) {
    let mut current = node;
    let mut depth = 0;
    let mut i = 0;
    loop {
        let parent = loop {
            let Some(parent) = ancestors.get(i) else {
                return (depth, current);
            };
            i += 1;
            match parent {
                Node::ArgumentsNode { .. } => {}
                Node::StatementsNode { .. }
                    if parent.as_statements_node().unwrap().body().iter().count() == 1 => {}
                _ => break *parent,
            }
        };
        if parent.as_statements_node().is_some() || parent.as_program_node().is_some() {
            return (depth, current);
        }
        let masgn = parent.as_multi_write_node().is_some();
        if !masgn
            && file.line_of(parent.location().start_offset())
                != file.line_of(current.location().start_offset())
        {
            return (depth, current);
        }
        if !masgn && !is_target(parent, current) {
            return (depth, current);
        }
        current = parent;
        depth = i;
        // The parser gem wraps a send with a literal block in a `block` node,
        // which is never a target itself: the send is the target.
        if parent
            .as_call_node()
            .and_then(|c| c.block())
            .is_some_and(|b| b.as_block_node().is_some())
        {
            return (depth, current);
        }
    }
}

fn is_target(parent: &Node<'_>, current: &Node<'_>) -> bool {
    match parent {
        Node::LocalVariableWriteNode { .. }
        | Node::InstanceVariableWriteNode { .. }
        | Node::ClassVariableWriteNode { .. }
        | Node::GlobalVariableWriteNode { .. }
        | Node::ConstantWriteNode { .. }
        | Node::ConstantPathWriteNode { .. }
        | Node::MultiWriteNode { .. }
        | Node::LocalVariableOperatorWriteNode { .. }
        | Node::InstanceVariableOperatorWriteNode { .. }
        | Node::ClassVariableOperatorWriteNode { .. }
        | Node::GlobalVariableOperatorWriteNode { .. }
        | Node::ConstantOperatorWriteNode { .. }
        | Node::ConstantPathOperatorWriteNode { .. }
        | Node::CallOperatorWriteNode { .. }
        | Node::IndexOperatorWriteNode { .. }
        | Node::LocalVariableOrWriteNode { .. }
        | Node::InstanceVariableOrWriteNode { .. }
        | Node::ClassVariableOrWriteNode { .. }
        | Node::GlobalVariableOrWriteNode { .. }
        | Node::ConstantOrWriteNode { .. }
        | Node::ConstantPathOrWriteNode { .. }
        | Node::CallOrWriteNode { .. }
        | Node::IndexOrWriteNode { .. }
        | Node::LocalVariableAndWriteNode { .. }
        | Node::InstanceVariableAndWriteNode { .. }
        | Node::ClassVariableAndWriteNode { .. }
        | Node::GlobalVariableAndWriteNode { .. }
        | Node::ConstantAndWriteNode { .. }
        | Node::ConstantPathAndWriteNode { .. }
        | Node::CallAndWriteNode { .. }
        | Node::IndexAndWriteNode { .. }
        | Node::DefNode { .. }
        | Node::SplatNode { .. }
        | Node::AndNode { .. }
        | Node::OrNode { .. } => true,
        Node::CallNode { .. } => {
            let call = parent.as_call_node().unwrap();
            // A setter (`a.b = x`, `a[k] = x`, also `&.`) is an assignment.
            if call.is_attribute_write() {
                return true;
            }
            if call.is_safe_navigation() {
                return false;
            }
            let name = call.name().as_slice();
            name == b"<<" || (name != b"[]" && call.receiver().is_some_and(|r| same(&r, current)))
        }
        _ => false,
    }
}

/// `find_lhs_node`: the left-hand side of an operator or multiple
/// assignment, otherwise the node itself.
fn lhs(node: &Node<'_>) -> Range<usize> {
    let loc = |l: Location<'_>| l.start_offset()..l.end_offset();
    let node_start = node.location().start_offset();
    match node {
        Node::LocalVariableOperatorWriteNode { .. } => loc(node
            .as_local_variable_operator_write_node()
            .unwrap()
            .name_loc()),
        Node::InstanceVariableOperatorWriteNode { .. } => loc(node
            .as_instance_variable_operator_write_node()
            .unwrap()
            .name_loc()),
        Node::ClassVariableOperatorWriteNode { .. } => loc(node
            .as_class_variable_operator_write_node()
            .unwrap()
            .name_loc()),
        Node::GlobalVariableOperatorWriteNode { .. } => loc(node
            .as_global_variable_operator_write_node()
            .unwrap()
            .name_loc()),
        Node::ConstantOperatorWriteNode { .. } => {
            loc(node.as_constant_operator_write_node().unwrap().name_loc())
        }
        Node::ConstantPathOperatorWriteNode { .. } => loc(node
            .as_constant_path_operator_write_node()
            .unwrap()
            .target()
            .location()),
        Node::CallOperatorWriteNode { .. } => {
            let n = node.as_call_operator_write_node().unwrap();
            node_start..n.message_loc().map_or(node_start, |l| l.end_offset())
        }
        Node::IndexOperatorWriteNode { .. } => {
            let n = node.as_index_operator_write_node().unwrap();
            node_start..n.closing_loc().end_offset()
        }
        Node::MultiWriteNode { .. } => {
            let n = node.as_multi_write_node().unwrap();
            if let (Some(open), Some(close)) = (n.lparen_loc(), n.rparen_loc()) {
                return open.start_offset()..close.end_offset();
            }
            let end = n
                .rights()
                .iter()
                .last()
                .map(|r| r.location().end_offset())
                .or_else(|| n.rest().map(|r| r.location().end_offset()))
                .or_else(|| n.lefts().iter().last().map(|l| l.location().end_offset()))
                .unwrap_or(node_start);
            node_start..end
        }
        _ => loc(node.location()),
    }
}

/// Whether a Prism ancestor stands for a parser-gem node (`ancestors[0]` is
/// the candidate, `ancestors[1]` its parent).
fn parser_node(ancestors: &[&Node<'_>]) -> bool {
    match ancestors[0] {
        Node::ArgumentsNode { .. }
        | Node::BlockNode { .. }
        | Node::ElseNode { .. }
        | Node::EnsureNode { .. }
        | Node::ProgramNode { .. } => false,
        Node::StatementsNode { .. } => {
            let statements = ancestors[0].as_statements_node().unwrap();
            if statements.body().iter().count() < 2 {
                return false;
            }
            match ancestors.get(1) {
                Some(Node::ParenthesesNode { .. }) => false,
                Some(parent @ Node::BeginNode { .. }) => {
                    let begin = parent.as_begin_node().unwrap();
                    begin.begin_keyword_loc().is_none()
                        || begin.rescue_clause().is_some()
                        || begin.ensure_clause().is_some()
                }
                _ => true,
            }
        }
        _ => true,
    }
}

/// `start_for_line_node`: the outermost ancestor of the target that starts on
/// the target's line, else the target itself; the column is its start.
fn line_anchor(file: &SourceFile, target: &Node<'_>, ancestors: &[&Node<'_>]) -> usize {
    let start = target.location().start_offset();
    let line = file.line_of(start);
    (0..ancestors.len())
        .rev()
        .filter(|&i| parser_node(&ancestors[i..]))
        .map(|i| ancestors[i].location().start_offset())
        .find(|&offset| file.line_of(offset) == line)
        .unwrap_or(start)
}

/// `do_line_anchor_loc`: the `do`/`{`, unless its line begins inside the
/// parentheses or brackets of an argument (or lambda parameter), where the
/// send's selector is the meaningful anchor.
fn do_line_anchor(file: &SourceFile, node: &Node<'_>, block: &Block<'_>) -> usize {
    let open = block.open.start_offset();
    let first = first_non_blank(file, open);
    let start = node.location().start_offset();
    if first > start
        && inside_parentheses(node, start..first)
        && block
            .arguments
            .iter()
            .any(|a| a.start <= first && first < a.end)
    {
        block.selector
    } else {
        open
    }
}

/// `inside_parentheses?`: more `(`/`[` than `)`/`]` tokens between the node's
/// start and `range.end`, from the paired delimiters of the node's subtree.
fn inside_parentheses(node: &Node<'_>, range: Range<usize>) -> bool {
    struct Pairs {
        range: Range<usize>,
        open: bool,
    }
    impl<'pr> walk::NodeVisitor<'pr> for Pairs {
        fn enter(&mut self, node: Node<'pr>) {
            let pair = match &node {
                Node::CallNode { .. } => {
                    let n = node.as_call_node().unwrap();
                    n.opening_loc().zip(n.closing_loc())
                }
                Node::ParenthesesNode { .. } => {
                    let n = node.as_parentheses_node().unwrap();
                    Some((n.opening_loc(), n.closing_loc()))
                }
                Node::ArrayNode { .. } => {
                    let n = node.as_array_node().unwrap();
                    n.opening_loc().zip(n.closing_loc())
                }
                Node::DefNode { .. } => {
                    let n = node.as_def_node().unwrap();
                    n.lparen_loc().zip(n.rparen_loc())
                }
                Node::BlockParametersNode { .. } => {
                    let n = node.as_block_parameters_node().unwrap();
                    n.opening_loc().zip(n.closing_loc())
                }
                Node::MultiTargetNode { .. } => {
                    let n = node.as_multi_target_node().unwrap();
                    n.lparen_loc().zip(n.rparen_loc())
                }
                Node::MultiWriteNode { .. } => {
                    let n = node.as_multi_write_node().unwrap();
                    n.lparen_loc().zip(n.rparen_loc())
                }
                Node::SuperNode { .. } => {
                    let n = node.as_super_node().unwrap();
                    n.lparen_loc().zip(n.rparen_loc())
                }
                Node::YieldNode { .. } => {
                    let n = node.as_yield_node().unwrap();
                    n.lparen_loc().zip(n.rparen_loc())
                }
                Node::DefinedNode { .. } => {
                    let n = node.as_defined_node().unwrap();
                    n.lparen_loc().zip(n.rparen_loc())
                }
                _ => None,
            };
            if let Some((open, close)) = pair
                && matches!(open.as_slice(), b"(" | b"[")
                && self.range.contains(&open.start_offset())
                && close.start_offset() >= self.range.end
            {
                self.open = true;
            }
        }
    }
    let mut pairs = Pairs { range, open: false };
    walk::walk(&mut pairs, node);
    pairs.open
}

#[cfg(test)]
mod tests;
