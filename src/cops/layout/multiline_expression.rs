//! Port of RuboCop's `MultilineExpressionIndentation` mixin for the fixed
//! `aligned` style, landed with `Layout/MultilineMethodCallIndentation` and
//! reused by `Layout/MultilineOperationIndentation`.
//!
//! Upstream climbs parser-gem ancestors. [`parser_ancestors`] maps a Prism
//! ancestor list onto that shape: argument lists, one-statement bodies and
//! `else` clauses are transparent; a call that owns a literal block stands for
//! both the parser `send` (when reached from its receiver or arguments) and
//! the enclosing `block` (when reached from the block's body); op-assignments
//! of an attribute contain the parser `send` of that attribute; and bracket
//! calls are parser `index`/`indexasgn` nodes, not `send`s.
use crate::source::SourceFile;
use ruby_prism::{Location, Node};
use std::ops::Range;

pub(super) type Span = Range<usize>;

pub(super) fn span(location: &Location<'_>) -> Span {
    location.start_offset()..location.end_offset()
}

pub(super) fn node_span(node: &Node<'_>) -> Span {
    span(&node.location())
}

pub(super) fn within(inner: &Span, outer: &Span) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

/// The parser-gem node type of a method dispatch. RuboCop's builders keep
/// the legacy AST (no `emit_index`, `emit_lambda` or `emit_kwargs`), so
/// bracket calls are `send :[]`/`:[]=`, lambdas are blocks of a `lambda`
/// send and keyword arguments are a `hash`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Dispatch {
    Send,
    Csend,
}

/// The dispatch kind of a Prism node that is (or contains, for attribute
/// and index op-assignments) a parser `send` or `csend`.
pub(super) fn dispatch(node: &Node<'_>) -> Option<Dispatch> {
    let safe = |flag: bool| {
        if flag {
            Dispatch::Csend
        } else {
            Dispatch::Send
        }
    };
    Some(match node {
        Node::CallNode { .. } => safe(node.as_call_node().unwrap().is_safe_navigation()),
        Node::CallOperatorWriteNode { .. } => safe(
            node.as_call_operator_write_node()
                .unwrap()
                .is_safe_navigation(),
        ),
        Node::CallOrWriteNode { .. } => {
            safe(node.as_call_or_write_node().unwrap().is_safe_navigation())
        }
        Node::CallAndWriteNode { .. } => {
            safe(node.as_call_and_write_node().unwrap().is_safe_navigation())
        }
        Node::CallTargetNode { .. } => {
            safe(node.as_call_target_node().unwrap().is_safe_navigation())
        }
        Node::IndexOperatorWriteNode { .. } => safe(
            node.as_index_operator_write_node()
                .unwrap()
                .is_safe_navigation(),
        ),
        Node::IndexOrWriteNode { .. } => {
            safe(node.as_index_or_write_node().unwrap().is_safe_navigation())
        }
        Node::IndexAndWriteNode { .. } => {
            safe(node.as_index_and_write_node().unwrap().is_safe_navigation())
        }
        Node::IndexTargetNode { .. } => Dispatch::Send,
        _ => return None,
    })
}

/// A parser `send`/`csend` (`call_type?`).
pub(super) fn call_type(node: &Node<'_>) -> bool {
    dispatch(node).is_some()
}

pub(super) fn receiver<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().receiver(),
        Node::CallOperatorWriteNode { .. } => {
            node.as_call_operator_write_node().unwrap().receiver()
        }
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node().unwrap().receiver(),
        Node::CallAndWriteNode { .. } => node.as_call_and_write_node().unwrap().receiver(),
        Node::CallTargetNode { .. } => Some(node.as_call_target_node().unwrap().receiver()),
        Node::IndexOperatorWriteNode { .. } => {
            node.as_index_operator_write_node().unwrap().receiver()
        }
        Node::IndexOrWriteNode { .. } => node.as_index_or_write_node().unwrap().receiver(),
        Node::IndexAndWriteNode { .. } => node.as_index_and_write_node().unwrap().receiver(),
        Node::IndexTargetNode { .. } => Some(node.as_index_target_node().unwrap().receiver()),
        _ => None,
    }
}

/// `loc.dot`: `.`, `&.` or `::`.
pub(super) fn dot<'pr>(node: &Node<'pr>) -> Option<Location<'pr>> {
    match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().call_operator_loc(),
        Node::CallOperatorWriteNode { .. } => node
            .as_call_operator_write_node()
            .unwrap()
            .call_operator_loc(),
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node().unwrap().call_operator_loc(),
        Node::CallAndWriteNode { .. } => node.as_call_and_write_node().unwrap().call_operator_loc(),
        Node::CallTargetNode { .. } => {
            Some(node.as_call_target_node().unwrap().call_operator_loc())
        }
        _ => None,
    }
}

/// `loc.selector` of a `send`/`csend` (the brackets of an index call are
/// not one Prism location; no consumer needs them).
pub(super) fn selector<'pr>(node: &Node<'pr>) -> Option<Location<'pr>> {
    match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().message_loc(),
        Node::CallOperatorWriteNode { .. } => {
            node.as_call_operator_write_node().unwrap().message_loc()
        }
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node().unwrap().message_loc(),
        Node::CallAndWriteNode { .. } => node.as_call_and_write_node().unwrap().message_loc(),
        Node::CallTargetNode { .. } => Some(node.as_call_target_node().unwrap().message_loc()),
        _ => None,
    }
}

/// The parser method name (`b` for the send inside `a.b += 1`).
pub(super) fn method_name<'pr>(node: &Node<'pr>) -> &'pr [u8] {
    match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().name().as_slice(),
        Node::CallOperatorWriteNode { .. } => node
            .as_call_operator_write_node()
            .unwrap()
            .read_name()
            .as_slice(),
        Node::CallOrWriteNode { .. } => {
            node.as_call_or_write_node().unwrap().read_name().as_slice()
        }
        Node::CallAndWriteNode { .. } => node
            .as_call_and_write_node()
            .unwrap()
            .read_name()
            .as_slice(),
        Node::CallTargetNode { .. } => node.as_call_target_node().unwrap().name().as_slice(),
        _ => b"",
    }
}

/// The parser arguments of a dispatch, a block argument included.
pub(super) fn arguments<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    let Some(call) = node.as_call_node() else {
        return Vec::new();
    };
    let mut out: Vec<Node<'pr>> = call
        .arguments()
        .map(|a| a.arguments().iter().collect())
        .unwrap_or_default();
    if let Some(block) = call
        .block()
        .filter(|b| b.as_block_argument_node().is_some())
    {
        out.push(block);
    }
    out
}

/// The `(` and `)` of a parenthesized `send`/`csend`.
pub(super) fn parentheses(node: &Node<'_>) -> Option<(usize, usize)> {
    let call = node.as_call_node()?;
    let open = call.opening_loc()?;
    let close = call.closing_loc()?;
    (open.as_slice() == b"(" && call_type(node)).then(|| (open.start_offset(), close.end_offset()))
}

/// A literal `do`/`{` block owned by a call or `super`.
pub(super) fn literal_block<'pr>(node: &Node<'pr>) -> Option<ruby_prism::BlockNode<'pr>> {
    let block = match node {
        Node::CallNode { .. } => node.as_call_node().unwrap().block(),
        Node::SuperNode { .. } => node.as_super_node().unwrap().block(),
        Node::ForwardingSuperNode { .. } => node
            .as_forwarding_super_node()
            .unwrap()
            .block()
            .map(|b| b.as_node()),
        _ => None,
    };
    block.and_then(|b| b.as_block_node())
}

/// A parser `block`/`numblock`/`itblock` node: a call or `super` with a
/// literal block, or a lambda.
pub(super) fn any_block(node: &Node<'_>) -> bool {
    node.as_lambda_node().is_some() || literal_block(node).is_some()
}

/// A parser `block` (not `numblock`/`itblock`).
pub(super) fn plain_block(node: &Node<'_>) -> bool {
    let parameters = if let Some(lambda) = node.as_lambda_node() {
        lambda.parameters()
    } else if let Some(block) = literal_block(node) {
        block.parameters()
    } else {
        return false;
    };
    !matches!(
        parameters,
        Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
    )
}

/// The opening and closing delimiters of a parser block: rubocop-ast's
/// `BlockNode#single_line?` compares their lines, not the whole expression.
pub(super) fn block_delimiters(node: &Node<'_>) -> Option<(usize, usize)> {
    if let Some(lambda) = node.as_lambda_node() {
        return Some((
            lambda.opening_loc().start_offset(),
            lambda.closing_loc().start_offset(),
        ));
    }
    let block = literal_block(node)?;
    Some((
        block.opening_loc().start_offset(),
        block.closing_loc().start_offset(),
    ))
}

/// The body of a parser block.
pub(super) fn block_body<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    if let Some(lambda) = node.as_lambda_node() {
        lambda.body()
    } else {
        literal_block(node)?.body()
    }
}

/// The parser source range of a dispatch: a literal block is not part of a
/// `send`, nor is the operator and value of an attribute op-assignment.
pub(super) fn parser_span(node: &Node<'_>) -> Span {
    match node {
        Node::CallNode { .. } if literal_block(node).is_some() => {
            let call = node.as_call_node().unwrap();
            node.location().start_offset()..crate::cops::bundler::ordered_gem_node::send_end(&call)
        }
        Node::CallOperatorWriteNode { .. }
        | Node::CallOrWriteNode { .. }
        | Node::CallAndWriteNode { .. } => {
            let start = node.location().start_offset();
            start..selector(node).map_or(start, |s| s.end_offset())
        }
        Node::IndexOperatorWriteNode { .. } => {
            let n = node.as_index_operator_write_node().unwrap();
            node.location().start_offset()..n.closing_loc().end_offset()
        }
        Node::IndexOrWriteNode { .. } => {
            let n = node.as_index_or_write_node().unwrap();
            node.location().start_offset()..n.closing_loc().end_offset()
        }
        Node::IndexAndWriteNode { .. } => {
            let n = node.as_index_and_write_node().unwrap();
            node.location().start_offset()..n.closing_loc().end_offset()
        }
        _ => node_span(node),
    }
}

/// `assignment_method?`: a name ending in `=` that is not a comparison.
pub(super) fn assignment_method(name: &[u8]) -> bool {
    name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"<=" | b">=" | b"===")
}

/// RuboCop AST's `OPERATOR_METHODS`.
pub(super) fn operator_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"|" | b"^"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b">"
            | b">="
            | b"<"
            | b"<="
            | b"<<"
            | b">>"
            | b"+"
            | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"**"
            | b"~"
            | b"+@"
            | b"-@"
            | b"!@"
            | b"~@"
            | b"[]"
            | b"[]="
            | b"!"
            | b"!="
            | b"!~"
            | b"`"
    )
}

/// One parser-gem ancestor of a node.
#[derive(Clone, Copy)]
pub(super) enum Anc<'b, 'pr> {
    /// A dispatch (`send`/`csend`/`index`) reached from its receiver or
    /// arguments; for an attribute op-assignment, its inner `send`.
    Call(&'b Node<'pr>),
    /// A parser block, reached from its body (`body: true`) or its `send`.
    Block {
        node: &'b Node<'pr>,
        body: bool,
    },
    Other(&'b Node<'pr>),
}

impl<'b, 'pr> Anc<'b, 'pr> {
    pub(super) fn node(&self) -> &'b Node<'pr> {
        match self {
            Anc::Call(n) | Anc::Block { node: n, .. } | Anc::Other(n) => n,
        }
    }

    /// The parser source range.
    pub(super) fn span(&self) -> Span {
        match self {
            Anc::Call(n) => parser_span(n),
            _ => node_span(self.node()),
        }
    }

    pub(super) fn send(&self) -> bool {
        matches!(self, Anc::Call(n) if dispatch(n) == Some(Dispatch::Send))
    }
}

/// The parser-gem ancestors of `node`, innermost first, from its Prism
/// ancestors (innermost first). `node` itself counts as a parser `send` when
/// it is one: a literal block it owns is its first ancestor.
pub(super) fn parser_ancestors<'b, 'pr>(
    node: &'b Node<'pr>,
    ancestors: &[&'b Node<'pr>],
) -> Vec<Anc<'b, 'pr>> {
    let mut out = Vec::new();
    if dispatch(node).is_some() && literal_block(node).is_some() {
        out.push(Anc::Block { node, body: false });
    }
    // The parser `send` inside an attribute op-assignment is its child.
    if matches!(
        node,
        Node::CallOperatorWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
    ) {
        out.push(Anc::Other(node));
    }
    let mut child = node_span(node);
    let mut from_block = false;
    for &ancestor in ancestors {
        match ancestor {
            Node::ProgramNode { .. } | Node::ArgumentsNode { .. } | Node::ElseNode { .. } => {}
            Node::StatementsNode { .. } => {
                if ancestor.as_statements_node().unwrap().body().len() >= 2 {
                    out.push(Anc::Other(ancestor));
                }
            }
            Node::BlockNode { .. } => from_block = true,
            Node::CallNode { .. } | Node::SuperNode { .. } | Node::ForwardingSuperNode { .. }
                if from_block =>
            {
                from_block = false;
                out.push(Anc::Block {
                    node: ancestor,
                    body: true,
                });
            }
            Node::LambdaNode { .. } => out.push(Anc::Block {
                node: ancestor,
                body: true,
            }),
            Node::CallOperatorWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexAndWriteNode { .. } => {
                if !within(
                    &child,
                    &assignment_value(ancestor).map_or(0..0, |v| node_span(&v)),
                ) {
                    out.push(Anc::Call(ancestor));
                }
                out.push(Anc::Other(ancestor));
            }
            _ if dispatch(ancestor).is_some() => {
                out.push(Anc::Call(ancestor));
                if literal_block(ancestor).is_some() {
                    out.push(Anc::Block {
                        node: ancestor,
                        body: false,
                    });
                }
            }
            Node::SuperNode { .. } | Node::ForwardingSuperNode { .. } => {
                out.push(Anc::Other(ancestor));
                if literal_block(ancestor).is_some() {
                    out.push(Anc::Block {
                        node: ancestor,
                        body: false,
                    });
                }
            }
            _ => out.push(Anc::Other(ancestor)),
        }
        child = node_span(ancestor);
    }
    out
}

/// The value of an assignment (`children.last`, `rhs`, `expression`).
pub(super) fn assignment_value<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    Some(match node {
        Node::LocalVariableWriteNode { .. } => node.as_local_variable_write_node()?.value(),
        Node::InstanceVariableWriteNode { .. } => node.as_instance_variable_write_node()?.value(),
        Node::ClassVariableWriteNode { .. } => node.as_class_variable_write_node()?.value(),
        Node::GlobalVariableWriteNode { .. } => node.as_global_variable_write_node()?.value(),
        Node::ConstantWriteNode { .. } => node.as_constant_write_node()?.value(),
        Node::ConstantPathWriteNode { .. } => node.as_constant_path_write_node()?.value(),
        Node::MultiWriteNode { .. } => node.as_multi_write_node()?.value(),
        Node::LocalVariableOperatorWriteNode { .. } => {
            node.as_local_variable_operator_write_node()?.value()
        }
        Node::LocalVariableOrWriteNode { .. } => node.as_local_variable_or_write_node()?.value(),
        Node::LocalVariableAndWriteNode { .. } => node.as_local_variable_and_write_node()?.value(),
        Node::InstanceVariableOperatorWriteNode { .. } => {
            node.as_instance_variable_operator_write_node()?.value()
        }
        Node::InstanceVariableOrWriteNode { .. } => {
            node.as_instance_variable_or_write_node()?.value()
        }
        Node::InstanceVariableAndWriteNode { .. } => {
            node.as_instance_variable_and_write_node()?.value()
        }
        Node::ClassVariableOperatorWriteNode { .. } => {
            node.as_class_variable_operator_write_node()?.value()
        }
        Node::ClassVariableOrWriteNode { .. } => node.as_class_variable_or_write_node()?.value(),
        Node::ClassVariableAndWriteNode { .. } => node.as_class_variable_and_write_node()?.value(),
        Node::GlobalVariableOperatorWriteNode { .. } => {
            node.as_global_variable_operator_write_node()?.value()
        }
        Node::GlobalVariableOrWriteNode { .. } => node.as_global_variable_or_write_node()?.value(),
        Node::GlobalVariableAndWriteNode { .. } => {
            node.as_global_variable_and_write_node()?.value()
        }
        Node::ConstantOperatorWriteNode { .. } => node.as_constant_operator_write_node()?.value(),
        Node::ConstantOrWriteNode { .. } => node.as_constant_or_write_node()?.value(),
        Node::ConstantAndWriteNode { .. } => node.as_constant_and_write_node()?.value(),
        Node::ConstantPathOperatorWriteNode { .. } => {
            node.as_constant_path_operator_write_node()?.value()
        }
        Node::ConstantPathOrWriteNode { .. } => node.as_constant_path_or_write_node()?.value(),
        Node::ConstantPathAndWriteNode { .. } => node.as_constant_path_and_write_node()?.value(),
        Node::CallOperatorWriteNode { .. } => node.as_call_operator_write_node()?.value(),
        Node::CallOrWriteNode { .. } => node.as_call_or_write_node()?.value(),
        Node::CallAndWriteNode { .. } => node.as_call_and_write_node()?.value(),
        Node::IndexOperatorWriteNode { .. } => node.as_index_operator_write_node()?.value(),
        Node::IndexOrWriteNode { .. } => node.as_index_or_write_node()?.value(),
        Node::IndexAndWriteNode { .. } => node.as_index_and_write_node()?.value(),
        _ => return None,
    })
}

/// A parser `if` (including `unless`, modifiers and ternaries).
fn if_type(node: &Node<'_>) -> bool {
    matches!(node, Node::IfNode { .. } | Node::UnlessNode { .. })
}

pub(super) fn ternary(node: &Node<'_>) -> bool {
    node.as_if_node()
        .is_some_and(|n| n.if_keyword_loc().is_none())
}

/// `while`/`until`, not the `begin ... end while` post-loop.
fn loop_type(node: &Node<'_>) -> bool {
    node.as_while_node().is_some_and(|n| !n.is_begin_modifier())
        || node.as_until_node().is_some_and(|n| !n.is_begin_modifier())
}

pub(super) fn kwbegin(node: &Node<'_>) -> bool {
    node.as_begin_node()
        .is_some_and(|n| n.begin_keyword_loc().is_some())
}

/// `grouped_expression?`: a parser `begin` with a `(` or `#{` delimiter.
pub(super) fn grouped_expression(anc: &Anc<'_, '_>) -> bool {
    matches!(
        anc,
        Anc::Other(
            Node::ParenthesesNode { .. }
                | Node::EmbeddedStatementsNode { .. }
                | Node::PinnedExpressionNode { .. }
        )
    )
}

/// `inside_arg_list_parentheses?`: `node` lies strictly inside the argument
/// parentheses of a parser `send`.
pub(super) fn inside_arg_list_parentheses(node: &Span, anc: &Anc<'_, '_>) -> bool {
    if !anc.send() {
        return false;
    }
    parentheses(anc.node()).is_some_and(|(open, close)| node.start > open && node.end < close)
}

/// `not_for_this_cop?`.
pub(super) fn not_for_this_cop(node: &Span, ancestors: &[Anc<'_, '_>]) -> bool {
    ancestors
        .iter()
        .any(|a| grouped_expression(a) || inside_arg_list_parentheses(node, a))
}

/// The keyword expression of a `for`/`if`/`while`/`until`/`return`.
fn indented_keyword_expression<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    match node {
        Node::ForNode { .. } => Some(node.as_for_node()?.collection()),
        Node::IfNode { .. } => Some(node.as_if_node()?.predicate()),
        Node::UnlessNode { .. } => Some(node.as_unless_node()?.predicate()),
        Node::WhileNode { .. } => Some(node.as_while_node()?.predicate()),
        Node::UntilNode { .. } => Some(node.as_until_node()?.predicate()),
        Node::ReturnNode { .. } => node
            .as_return_node()?
            .arguments()?
            .arguments()
            .iter()
            .next(),
        _ => None,
    }
}

/// `kw_node_with_special_indentation`: the first `for`/`if`/`while`/`until`/
/// `return` ancestor whose keyword expression contains `node`, and that
/// expression's range.
pub(super) fn keyword_node<'b, 'pr>(
    node: &Span,
    ancestors: &[Anc<'b, 'pr>],
) -> Option<(&'b Node<'pr>, Span)> {
    ancestors.iter().find_map(|a| {
        let n = a.node();
        let Anc::Other(_) = a else { return None };
        if !(if_type(n) && !ternary(n)
            || loop_type(n)
            || n.as_for_node().is_some()
            || n.as_return_node().is_some())
        {
            return None;
        }
        let expression = node_span(&indented_keyword_expression(n)?);
        within(node, &expression).then_some((n, expression))
    })
}

/// `postfix_conditional?`: a modifier `if`/`unless`.
fn postfix_conditional(node: &Node<'_>) -> bool {
    match node {
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            n.if_keyword_loc().is_some() && n.end_keyword_loc().is_none()
        }
        Node::UnlessNode { .. } => node.as_unless_node().unwrap().end_keyword_loc().is_none(),
        _ => false,
    }
}

/// `correct_indentation`: one step, or two for a prefix keyword expression.
pub(super) fn correct_indentation(node: &Span, ancestors: &[Anc<'_, '_>]) -> usize {
    match keyword_node(node, ancestors) {
        Some((keyword, _)) if !postfix_conditional(keyword) => 4,
        _ => 2,
    }
}

/// `keyword_message_tail`.
fn keyword_message_tail(node: &Node<'_>) -> String {
    let keyword = match node {
        Node::IfNode { .. } => node.as_if_node().unwrap().if_keyword_loc(),
        Node::UnlessNode { .. } => Some(node.as_unless_node().unwrap().keyword_loc()),
        Node::WhileNode { .. } => Some(node.as_while_node().unwrap().keyword_loc()),
        Node::UntilNode { .. } => Some(node.as_until_node().unwrap().keyword_loc()),
        Node::ForNode { .. } => Some(node.as_for_node().unwrap().for_keyword_loc()),
        Node::ReturnNode { .. } => Some(node.as_return_node().unwrap().keyword_loc()),
        _ => None,
    };
    let keyword = keyword.map_or_else(String::new, |k| {
        String::from_utf8_lossy(k.as_slice()).into_owned()
    });
    let kind = if keyword == "for" {
        "collection"
    } else {
        "condition"
    };
    let article = if keyword.starts_with(['i', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("a {kind} in {article} `{keyword}` statement")
}

/// `operation_description`.
pub(super) fn operation_description(node: &Span, rhs: &Span, ancestors: &[Anc<'_, '_>]) -> String {
    if let Some((keyword, _)) = keyword_node(node, ancestors) {
        return keyword_message_tail(keyword);
    }
    if part_of_assignment_rhs(Some(rhs), ancestors).is_some() {
        return "an expression in an assignment".to_owned();
    }
    "an expression".to_owned()
}

/// Which parentheses `argument_in_method_call` requires.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Parens {
    Required,
    Any,
}

/// `argument_in_method_call`: the first parser `send` ancestor (not a setter,
/// parenthesized when required) holding `node` in an argument; `None` at a
/// parser `block` ancestor.
pub(super) fn argument_in_method_call<'b, 'pr>(
    node: &Span,
    kind: Parens,
    ancestors: &[Anc<'b, 'pr>],
) -> Option<&'b Node<'pr>> {
    for a in ancestors {
        match a {
            Anc::Block { node: b, .. } if plain_block(b) => return None,
            Anc::Call(call) if a.send() => {
                if setter(call) || kind == Parens::Required && parentheses(call).is_none() {
                    continue;
                }
                if arguments(call)
                    .iter()
                    .any(|arg| within(node, &node_span(arg)))
                {
                    return Some(call);
                }
            }
            _ => {}
        }
    }
    None
}

/// `setter_method?`: `loc.operator`, i.e. an attribute or index assignment.
pub(super) fn setter(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|c| c.is_attribute_write())
}

/// The value assigned by a parser assignment or setter ancestor.
fn assignment_rhs<'pr>(anc: &Anc<'_, 'pr>) -> Option<Node<'pr>> {
    match anc {
        Anc::Call(node) if setter(node) => arguments(node).pop(),
        Anc::Other(node) => assignment_value(node),
        _ => None,
    }
}

/// `part_of_assignment_rhs`: the first assignment ancestor whose value holds
/// `candidate` (any assignment when `None`), stopping at unaligned
/// constructs and at a parser block whose body holds it. Returns the value.
pub(super) fn part_of_assignment_rhs<'pr>(
    candidate: Option<&Span>,
    ancestors: &[Anc<'_, 'pr>],
) -> Option<Node<'pr>> {
    for a in ancestors {
        let n = a.node();
        let disqualified = match a {
            Anc::Other(n) => {
                if_type(n)
                    || loop_type(n)
                    || n.as_for_node().is_some()
                    || n.as_return_node().is_some()
                    || n.as_array_node().is_some()
                    || kwbegin(n)
            }
            Anc::Block { node: b, .. } => {
                plain_block(b)
                    && block_body(b)
                        .is_some_and(|body| candidate.is_none_or(|c| within(c, &node_span(&body))))
            }
            Anc::Call(_) => false,
        };
        if disqualified {
            return None;
        }
        let valid = match a {
            Anc::Call(call) if setter(call) => true,
            Anc::Other(_) => assignment_value(n).is_some(),
            _ => false,
        };
        if !valid {
            continue;
        }
        let Some(rhs) = assignment_rhs(a) else {
            continue;
        };
        if candidate.is_none_or(|c| within(c, &node_span(&rhs))) {
            return Some(rhs);
        }
    }
    None
}

/// `indentation(node)`: the character column of the first non-whitespace
/// character on the node's first line.
pub(super) fn line_indentation(file: &SourceFile, offset: usize) -> usize {
    let range = super::alignment::start_line_range(file, offset);
    super::alignment::effective_column(file, range.start)
}

/// The character column of an offset.
pub(super) fn column(file: &SourceFile, offset: usize) -> usize {
    super::alignment::effective_column(file, offset)
}
