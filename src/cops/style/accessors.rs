//! Shared accessor sibling matching and lazy, closed-source correction proof.
use crate::{
    cops::{
        ast,
        walk::{NodeVisitor, walk},
    },
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::collections::HashSet;

pub(super) fn children<'pr>(node: &Node<'pr>) -> Option<Vec<Node<'pr>>> {
    let body = if let Some(class) = node.as_class_node() {
        class.body()
    } else if let Some(module) = node.as_module_node() {
        module.body()
    } else {
        node.as_singleton_class_node()?.body()
    };
    if let Some(statements) = body.as_ref().and_then(Node::as_statements_node)
        && statements.body().len() == 1
        && let Some(begin) = statements
            .body()
            .iter()
            .next()
            .and_then(|n| n.as_begin_node())
        && begin.rescue_clause().is_none()
        && begin.ensure_clause().is_none()
    {
        return Some(
            begin
                .statements()
                .map(|s| s.body().iter().collect())
                .unwrap_or_default(),
        );
    }
    if let Some(statements) = body.as_ref().and_then(Node::as_statements_node)
        && statements.body().len() == 1
        && let Some(only) = statements.body().first()
    {
        // A sole logical expression is the reference class body itself, not
        // a begin/statement list. Its immediate operands are the sibling sends
        // selected by each_child_node(:send). Do not flatten nested operators
        // or expressions beside other statements: those have different parents.
        if let Some(and) = only.as_and_node() {
            return Some(vec![and.left(), and.right()]);
        }
        if let Some(or) = only.as_or_node() {
            return Some(vec![or.left(), or.right()]);
        }
    }
    Some(super::module_function::body_elements(body))
}
pub(super) fn literal_block(call: &CallNode<'_>) -> bool {
    call.block().is_some_and(|b| b.as_block_node().is_some())
}
pub(super) fn family(call: &CallNode<'_>) -> bool {
    call.receiver().is_none()
        && !literal_block(call)
        && matches!(
            call.name().as_slice(),
            b"attr" | b"attr_reader" | b"attr_writer" | b"attr_accessor"
        )
        && !ast::call_arguments(call).is_empty()
}
pub(super) fn visibility<'pr>(call: &CallNode<'pr>) -> Option<&'pr [u8]> {
    (call.receiver().is_none()
        && !literal_block(call)
        && ast::call_arguments(call).is_empty()
        && matches!(
            call.name().as_slice(),
            b"private" | b"protected" | b"public"
        ))
    .then(|| call.name().as_slice())
}
pub(super) fn groupable(child: &Node<'_>, previous: Option<&Node<'_>>, file: &SourceFile) -> bool {
    let line = file.line_of(child.location().start_offset());
    let annotations = file.accessor_annotations();
    if line > 1 && annotations.comment_lines.contains(&(line - 1)) {
        return false;
    }
    let Some(previous_node) = previous else {
        return true;
    };
    let first_line = file.line_of(previous_node.location().start_offset());
    // Parser represents an arrow lambda as a block with a synthetic `lambda`
    // send whose range is just `->`, not the parameters or literal body.
    if previous_node.as_lambda_node().is_some() {
        return !annotations.rbs_lines.contains(&first_line) && line > first_line + 1;
    }
    let Some(previous) = previous_node
        .as_call_node()
        .filter(|c| !c.is_safe_navigation())
    else {
        return true;
    };
    let prev_line = file.line_of(ast::source_end(previous_node).saturating_sub(1));
    if annotations.rbs_lines.contains(&first_line) {
        return false;
    }
    (previous.receiver().is_none()
        && matches!(
            previous.name().as_slice(),
            b"attr" | b"attr_reader" | b"attr_writer" | b"attr_accessor"
        )
        && !ast::call_arguments(&previous).is_empty())
        || (previous.receiver().is_none()
            && matches!(
                previous.name().as_slice(),
                b"public" | b"private" | b"protected" | b"module_function"
            ))
        || line > prev_line + 1
}

/// Physical comment-only lines (including text in literals, as upstream does)
/// and actual RBS annotation starts. Cache once: many siblings can share a very
/// long preceding physical line, so trimming/scanning it per accessor is unbounded.
#[derive(Default)]
pub(crate) struct Annotations {
    comment_lines: HashSet<u32>,
    rbs_lines: HashSet<u32>,
    #[cfg(test)]
    pub(crate) visited_lines: usize,
}
impl Annotations {
    pub(crate) fn build(file: &SourceFile) -> Self {
        let mut result = Self::default();
        for line in 1..=file.code_line_count() as u32 {
            #[cfg(test)]
            {
                result.visited_lines += 1;
            }
            let text = file.line_text(line);
            let start = text
                .iter()
                .position(|b| !super::is_ruby_whitespace(*b))
                .unwrap_or(text.len());
            if text[start..].starts_with(b"#") {
                result.comment_lines.insert(line);
            }
        }
        for comment in file.comments() {
            if file.bytes[comment.clone()].starts_with(b"#:") {
                result.rbs_lines.insert(file.line_of(comment.start));
            }
        }
        result
    }
}

#[derive(Default)]
pub(crate) struct CustomAttrs {
    scopes: HashSet<usize>,
    #[cfg(test)]
    pub(crate) visited: usize,
}
impl CustomAttrs {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut index = Self::default();
        struct Visitor<'a> {
            index: &'a mut CustomAttrs,
            scopes: Vec<usize>,
            stack: Vec<bool>,
        }
        impl<'pr> NodeVisitor<'pr> for Visitor<'_> {
            fn enter(&mut self, node: Node<'pr>) {
                self.stack.push(matches!(
                    node,
                    Node::ClassNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
                ));
                #[cfg(test)]
                {
                    self.index.visited += 1;
                }
                if matches!(
                    node,
                    Node::ClassNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
                ) {
                    self.scopes.push(node.location().start_offset());
                }
                if node
                    .as_def_node()
                    .is_some_and(|d| d.receiver().is_none() && d.name().as_slice() == b"attr")
                {
                    self.index.scopes.extend(self.scopes.iter().copied());
                }
            }
            fn leave(&mut self) {
                if self.stack.pop() == Some(true) {
                    self.scopes.pop();
                }
            }
        }
        walk(
            &mut Visitor {
                index: &mut index,
                scopes: Vec::new(),
                stack: Vec::new(),
            },
            root,
        );
        index
    }
    pub(super) fn contains(&self, node: &Node<'_>) -> bool {
        self.scopes.contains(&node.location().start_offset())
    }
}
