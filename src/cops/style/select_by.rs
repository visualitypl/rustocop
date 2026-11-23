//! Helpers shared by `Style/SelectByKind`, `Style/SelectByRange`,
//! `Style/SelectByRegexp` and `Style/PredicateWithKind`: the same receiver,
//! block-parameter and call shapes their RuboCop originals match.
use ruby_prism::{BlockNode, CallNode, Node};

use crate::cops::ast;

pub(super) fn plain_call(call: &CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call
            .block()
            .is_none_or(|n| n.as_block_argument_node().is_some())
}

pub(super) fn one_parenthesis(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(paren) = node.as_parentheses_node() {
        let body = paren.body()?.as_statements_node()?;
        if body.body().len() != 1 {
            return None;
        }
        body.body().iter().next()
    } else {
        Some(node)
    }
}

pub(super) fn hash_receiver(node: &Node<'_>) -> bool {
    if node.as_hash_node().is_some() || ast::is_global_const(node, &["ENV"]) {
        return true;
    }
    let Some(call) = node.as_call_node() else {
        return false;
    };
    let literal_block = call.block().is_some_and(|n| n.as_block_node().is_some());
    if matches!(call.name().as_slice(), b"to_h" | b"to_hash") {
        return !literal_block;
    }
    let ordinary_block = call
        .block()
        .and_then(|n| n.as_block_node())
        .is_some_and(|b| {
            b.parameters().is_none_or(|p| {
                p.as_numbered_parameters_node().is_none() && p.as_it_parameters_node().is_none()
            })
        });
    if !matches!(call.name().as_slice(), b"new" | b"[]")
        || (literal_block && (call.name().as_slice() != b"new" || !ordinary_block))
    {
        return false;
    }
    call.receiver().is_some_and(|n| {
        n.as_constant_read_node()
            .is_some_and(|n| n.name().as_slice() == b"Hash")
            || n.as_constant_path_node()
                .is_some_and(|n| n.name().is_some_and(|n| n.as_slice() == b"Hash"))
    })
}

pub(super) fn uses_parameter(block: &BlockNode<'_>, node: &Node<'_>) -> bool {
    let Some(parameters) = block.parameters() else {
        return false;
    };
    if let Some(numbered) = parameters.as_numbered_parameters_node() {
        return numbered.maximum() == 1
            && node
                .as_local_variable_read_node()
                .is_some_and(|n| n.name().as_slice() == b"_1");
    }
    if parameters.as_it_parameters_node().is_some() {
        return node.as_it_local_variable_read_node().is_some();
    }
    let Some(parameters) = parameters.as_block_parameters_node() else {
        return false;
    };
    if !parameters.locals().is_empty() {
        return false;
    }
    let Some(parameters) = parameters.parameters() else {
        return false;
    };
    if parameters.requireds().len() != 1
        || !parameters.optionals().is_empty()
        || parameters
            .rest()
            .is_some_and(|n| n.as_implicit_rest_node().is_none())
        || !parameters.posts().is_empty()
        || !parameters.keywords().is_empty()
        || parameters.keyword_rest().is_some()
        || parameters.block().is_some()
    {
        return false;
    }
    let Some(parameter) = parameters
        .requireds()
        .iter()
        .next()
        .and_then(|n| n.as_required_parameter_node())
    else {
        return false;
    };
    node.as_local_variable_read_node()
        .is_some_and(|n| n.name().as_slice() == parameter.name().as_slice())
}
