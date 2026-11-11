//! Pinned Gemfile declaration facts shared by the Bundler cops: receiverless
//! sends found in the parser gem's pre-order, with the ancestors RuboCop's
//! AST gives them, and the parser `str` shape of their arguments. Nothing is
//! evaluated; this is the main parse only.
use std::collections::HashSet;

use ruby_prism::{CallNode, Node, StatementsNode};

use crate::{
    cops::{
        ast::{self, Fingerprint},
        lint::reference_basic_literal::is_basic_literal,
        walk::{NodeVisitor, walk},
    },
    source::SourceFile,
};

/// Receiverless calls named `name` (`(send nil? :name ...)`), in the parser
/// gem's pre-order (`def_node_search`), each mapped by `visit` from the call
/// and its Prism ancestors (innermost last). The only order difference from
/// Prism's field order is `unless`, whose pinned node stores its else body
/// before its body.
pub(super) fn receiverless_calls<'pr, R>(
    root: &Node<'pr>,
    name: &[u8],
    mut visit: impl FnMut(&CallNode<'pr>, &[Node<'pr>]) -> Option<R>,
) -> Vec<R> {
    parser_order_calls(root, |call, ancestors| {
        (call.receiver().is_none() && call.name().as_slice() == name)
            .then(|| visit(call, ancestors))
            .flatten()
    })
}

/// Every call mapped by `visit` from the call and its Prism ancestors
/// (innermost last), in the parser gem's pre-order: the generic search
/// behind [`receiverless_calls`].
pub(crate) fn parser_order_calls<'pr, R>(
    root: &Node<'pr>,
    mut visit: impl FnMut(&CallNode<'pr>, &[Node<'pr>]) -> Option<R>,
) -> Vec<R> {
    parser_order_nodes(root, |node, ancestors| {
        visit(&node.as_call_node()?, ancestors)
    })
}

/// Every node mapped by `visit` from the node and its Prism ancestors
/// (innermost last), in the parser gem's pre-order (`each_descendant`).
pub(crate) fn parser_order_nodes<'pr, R>(
    root: &Node<'pr>,
    visit: impl FnMut(&Node<'pr>, &[Node<'pr>]) -> Option<R>,
) -> Vec<R> {
    let mut collector = Collector {
        ancestors: Vec::new(),
        counters: Vec::new(),
        path: Vec::new(),
        found: Vec::new(),
        visit,
    };
    walk(&mut collector, root);
    let mut found = collector.found;
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().map(|(_, r)| r).collect()
}

struct Collector<'pr, F, R> {
    ancestors: Vec<Node<'pr>>,
    counters: Vec<u32>,
    path: Vec<u32>,
    found: Vec<(Vec<u32>, R)>,
    visit: F,
}

impl<'pr, F, R> NodeVisitor<'pr> for Collector<'pr, F, R>
where
    F: FnMut(&Node<'pr>, &[Node<'pr>]) -> Option<R>,
{
    fn enter(&mut self, node: Node<'pr>) {
        let rank = match (self.ancestors.last(), &node) {
            (Some(Node::UnlessNode { .. }), Node::ElseNode { .. }) => 1,
            (Some(Node::UnlessNode { .. }), Node::StatementsNode { .. }) => 2,
            (Some(Node::UnlessNode { .. }), _) => 0,
            _ => self.counters.last_mut().map_or(0, |count| {
                *count += 1;
                *count
            }),
        };
        self.path.push(rank);
        if let Some(found) = (self.visit)(&node, &self.ancestors) {
            self.found.push((self.path.clone(), found));
        }
        self.ancestors.push(node);
        self.counters.push(0);
    }

    fn leave(&mut self) {
        self.ancestors.pop();
        self.counters.pop();
        self.path.pop();
    }
}

/// `(send nil? :gem str ...)`: the declared name's str value.
pub(super) fn gem_name(call: &CallNode<'_>, file: &SourceFile) -> Option<Vec<u8>> {
    if call.name().as_slice() != b"gem" || call.receiver().is_some() {
        return None;
    }
    str_value(&call.arguments()?.arguments().iter().next()?, file)
}

/// The value of a node the pinned AST builds as `str` (not `dstr`).
pub(crate) fn str_value(node: &Node<'_>, file: &SourceFile) -> Option<Vec<u8>> {
    if !is_basic_literal(node, file) {
        return None;
    }
    if let Some(string) = node.as_string_node() {
        return Some(string.unescaped().to_vec());
    }
    // A static squiggly heredoc the adapter folds into one str child.
    let string = node.as_interpolated_string_node()?;
    let mut value = Vec::new();
    for part in string.parts().iter() {
        value.extend_from_slice(part.as_string_node()?.unescaped());
    }
    Some(value)
}

/// The pinned `send` node's arguments (a block-pass included, a literal
/// block excluded), compared structurally like RuboCop's `Node#==`.
pub(super) type SendKey = Vec<Fingerprint<'static>>;

pub(super) fn send_key(call: &CallNode<'_>) -> SendKey {
    let mut key: SendKey = call
        .arguments()
        .iter()
        .flat_map(|arguments| arguments.arguments().iter())
        .map(|argument| ast::fingerprint(&argument).into_owned())
        .collect();
    if let Some(block) = call.block()
        && block.as_block_argument_node().is_some()
    {
        key.push(ast::fingerprint(&block).into_owned());
    }
    key
}

/// The receiverless `gem` send a node is in the pinned AST, if any (a call
/// with a literal block is a `block` node there).
fn blockless_gem(node: &Node<'_>) -> Option<SendKey> {
    let call = node.as_call_node()?;
    (call.receiver().is_none()
        && call.name().as_slice() == b"gem"
        && call.block().is_none_or(|b| b.as_block_node().is_none()))
    .then(|| send_key(&call))
}

/// The conditional whose branches decide `conditional_declaration?`: the
/// nearest ancestor that is not a parser `begin` must be an `if` (the root)
/// or a `when` (its `case` is the root).
pub(super) fn conditional_root<'a, 'pr>(ancestors: &'a [Node<'pr>]) -> Option<&'a Node<'pr>> {
    let mut outward = ancestors.iter().rev();
    let parent = outward.find(|node| {
        !matches!(
            node,
            Node::StatementsNode { .. } | Node::ParenthesesNode { .. } | Node::ElseNode { .. }
        )
    })?;
    match parent {
        Node::IfNode { .. } | Node::UnlessNode { .. } => Some(parent),
        Node::WhenNode { .. } => outward.next(),
        _ => None,
    }
}

/// A node as the pinned AST has it: a Prism node, a multi-statement `begin`,
/// or the `send` child of a `block` node.
enum Parser<'pr> {
    Node(Node<'pr>),
    Begin(Vec<Node<'pr>>),
    Send(SendKey),
}

fn statements(statements: Option<StatementsNode<'_>>) -> Option<Parser<'_>> {
    let mut body: Vec<_> = statements?.body().iter().collect();
    match body.len() {
        0 => None,
        1 => body.pop().map(Parser::Node),
        _ => Some(Parser::Begin(body)),
    }
}

fn body(node: Option<Node<'_>>) -> Option<Parser<'_>> {
    let node = node?;
    match node.as_statements_node() {
        Some(list) => statements(Some(list)),
        None => Some(Parser::Node(node)),
    }
}

/// `IfNode#branches` / `CaseNode#branches`, compacted.
fn branches<'pr>(root: &Node<'pr>) -> Vec<Parser<'pr>> {
    let mut out = Vec::new();
    if let Some(node) = root.as_if_node() {
        out.extend(statements(node.statements()));
        match node.subsequent() {
            Some(Node::IfNode { .. }) => out.extend(branches(&node.subsequent().unwrap())),
            Some(other) => {
                out.extend(
                    other
                        .as_else_node()
                        .and_then(|e| statements(e.statements())),
                );
            }
            None => {}
        }
    } else if let Some(node) = root.as_unless_node() {
        out.extend(statements(node.statements()));
        out.extend(node.else_clause().and_then(|e| statements(e.statements())));
    } else if let Some(node) = root.as_case_node() {
        for when in node.conditions().iter() {
            out.extend(when.as_when_node().and_then(|w| statements(w.statements())));
        }
        out.extend(node.else_clause().and_then(|e| statements(e.statements())));
    }
    out
}

/// `child_nodes` of a pinned node, where it can matter for a `gem` send.
fn children(node: Parser<'_>) -> Vec<Parser<'_>> {
    let node = match node {
        Parser::Begin(list) => return list.into_iter().map(Parser::Node).collect(),
        Parser::Send(_) => return Vec::new(),
        Parser::Node(node) => node,
    };
    let node = &node;
    let mut out = Vec::new();
    if let Some(parentheses) = node.as_parentheses_node() {
        match parentheses.body() {
            Some(Node::StatementsNode { .. }) => {
                let list = parentheses.body().unwrap();
                out.extend(
                    list.as_statements_node()
                        .unwrap()
                        .body()
                        .iter()
                        .map(Parser::Node),
                );
            }
            Some(other) => out.push(Parser::Node(other)),
            None => {}
        }
    } else if let Some(call) = node.as_call_node()
        && let Some(block) = call.block().and_then(|b| b.as_block_node())
    {
        if let Some(key) = blockless_send_part(&call) {
            out.push(Parser::Send(key));
        }
        out.extend(body(block.body()));
    } else if let Some(node) = node.as_if_node() {
        out.push(Parser::Node(node.predicate()));
        out.extend(statements(node.statements()));
        match node.subsequent() {
            Some(Node::ElseNode { .. }) => out.extend(
                node.subsequent()
                    .unwrap()
                    .as_else_node()
                    .and_then(|e| statements(e.statements())),
            ),
            Some(elsif) => out.push(Parser::Node(elsif)),
            None => {}
        }
    } else if let Some(node) = node.as_unless_node() {
        out.push(Parser::Node(node.predicate()));
        out.extend(statements(node.statements()));
        out.extend(node.else_clause().and_then(|e| statements(e.statements())));
    } else if let Some(begin) = node.as_begin_node()
        && begin.rescue_clause().is_none()
        && begin.else_clause().is_none()
        && begin.ensure_clause().is_none()
    {
        // kwbegin lists its statements directly.
        out.extend(
            begin
                .statements()
                .iter()
                .flat_map(|s| s.body().iter())
                .map(Parser::Node),
        );
    } else {
        let mut direct = Direct {
            depth: 0,
            out: Vec::new(),
        };
        walk(&mut direct, node);
        for child in direct.out {
            if let Some(arguments) = child.as_arguments_node() {
                out.extend(arguments.arguments().iter().map(Parser::Node));
            } else if let Some(list) = child.as_statements_node() {
                out.extend(statements(Some(list)));
            } else if let Some(other) = child.as_else_node() {
                out.extend(statements(other.statements()));
            } else {
                out.push(Parser::Node(child));
            }
        }
    }
    out
}

/// The `send` child of a receiverless `gem` call that carries a block.
fn blockless_send_part(call: &CallNode<'_>) -> Option<SendKey> {
    (call.receiver().is_none() && call.name().as_slice() == b"gem").then(|| send_key(call))
}

/// The direct Prism children of a node.
struct Direct<'pr> {
    depth: usize,
    out: Vec<Node<'pr>>,
}

impl<'pr> NodeVisitor<'pr> for Direct<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if self.depth == 1 {
            self.out.push(node);
        }
        self.depth += 1;
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }
}

/// The send keys `IfNode#branches`/`CaseNode#branches` contain as a branch
/// or as a branch's direct child (`within_conditional?`).
pub(super) fn branch_keys(root: &Node<'_>) -> HashSet<SendKey> {
    let mut keys = HashSet::new();
    let mut insert = |candidate: &Parser<'_>| match candidate {
        Parser::Node(node) => keys.extend(blockless_gem(node)),
        Parser::Send(key) => {
            keys.insert(key.clone());
        }
        Parser::Begin(_) => {}
    };
    for branch in branches(root) {
        insert(&branch);
        for child in children(branch) {
            insert(&child);
        }
    }
    keys
}

/// Message text: valid UTF-8 as is, other bytes escaped.
pub(super) fn text(bytes: &[u8]) -> String {
    bytes
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().to_owned();
            for b in chunk.invalid() {
                text.push_str(&format!("\\x{b:02X}"));
            }
            text
        })
        .collect()
}

/// Source text as the pinned source buffer has it (CRLF folded).
pub(super) fn source_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut iter = bytes.iter().peekable();
    while let Some(&b) = iter.next() {
        if b == b'\r' && iter.peek() == Some(&&b'\n') {
            continue;
        }
        out.push(b);
    }
    out
}
