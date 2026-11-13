//! `Lint/UnmodifiedReduceAccumulator` — pinned syntactic reduce/inject returns.
//!
//! Deliberately not data flow: upstream compares argument/variable names, even
//! inside nested scopes, and only recognizes a few direct accumulator shapes.

use crate::{
    cops::{
        Cop, NodeContext, ast, variables,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct UnmodifiedReduceAccumulator;
impl Cop for UnmodifiedReduceAccumulator {
    fn name(&self) -> &'static str {
        "Lint/UnmodifiedReduceAccumulator"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_block_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(block) = node.as_block_node() else {
            return;
        };
        let Some(call) = cx.parent().and_then(Node::as_call_node) else {
            return;
        };
        if !matches!(call.name().as_slice(), b"reduce" | b"inject") {
            return;
        }
        let Some(body) = block.body() else { return };
        let names = argument_names(block.parameters());
        let [acc, el, ..] = names.as_slice() else {
            return;
        };
        let Some(body) = single_statement(body) else {
            return;
        };
        let syntax = SyntacticNames::new(node, cx, file, *acc, *el);
        let mut returns = Returns {
            values: vec![last_value(single_statement(block.body().unwrap()).unwrap())],
            boundaries: Vec::new(),
            depth: 0,
            first: true,
        };
        walk::walk(&mut returns, &body);
        let method = String::from_utf8_lossy(call.name().as_slice());
        let index = returns
            .values
            .iter()
            .find(|n| accumulator_index(n, *acc, *el, &syntax));
        if let Some(index) = index {
            report(
                self,
                index,
                format!("Do not return an element of the accumulator in `{method}`."),
                file,
                out,
            );
            return;
        }
        let mut mutation = ElementMutation {
            syntax: &syntax,
            name: *el,
            found: false,
            frames: Vec::new(),
            containing: std::collections::HashSet::new(),
        };
        walk::walk(&mut mutation, &body);
        if mutation.found || returns.values.iter().any(|n| lvar_used(n, *acc, &syntax)) {
            return;
        }
        for value in returns.values {
            let mut values = ExpressionValues {
                syntax: &syntax,
                element: *el,
                found: false,
                other: false,
                patterns: Vec::new(),
            };
            walk::walk(&mut values, &value);
            if values.found && !values.other {
                let acc = acc.map_or_else(String::new, |s| display_name(s, file));
                report(
                    self,
                    &value,
                    format!("Ensure the accumulator `{acc}` will be modified by `{method}`."),
                    file,
                    out,
                );
            }
        }
    }
}

fn display_name(bytes: &[u8], file: &SourceFile) -> String {
    let projected = file.match_pattern_source(bytes);
    match std::str::from_utf8(&projected) {
        Ok(text) => text.into(),
        Err(_) => projected
            .iter()
            .map(|b| {
                if b.is_ascii() {
                    char::from(*b).to_string()
                } else {
                    format!("\\x{b:02X}")
                }
            })
            .collect(),
    }
}

fn report(
    cop: &UnmodifiedReduceAccumulator,
    node: &Node<'_>,
    message: String,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let mut start = node.location().start_offset();
    // A returned literal block is the whole block expression, not just its
    // send (the shared helper deliberately excludes a call's block highlight).
    let mut end = if node
        .as_call_node()
        .is_some_and(|n| n.block().is_some_and(|b| b.as_block_node().is_some()))
    {
        node.location().end_offset()
    } else {
        ast::source_end(node)
    };
    if let Some(begin) = node
        .as_begin_node()
        .filter(|n| n.begin_keyword_loc().is_none())
    {
        start = begin
            .statements()
            .map(|n| n.location().start_offset())
            .or_else(|| {
                begin
                    .rescue_clause()
                    .map(|n| n.keyword_loc().start_offset())
            })
            .or_else(|| {
                begin
                    .ensure_clause()
                    .map(|n| n.ensure_keyword_loc().start_offset())
            })
            .unwrap_or(start);
        end = begin
            .ensure_clause()
            .map(|n| {
                n.statements()
                    .map_or(n.ensure_keyword_loc().end_offset(), |s| {
                        s.location().end_offset()
                    })
            })
            .or_else(|| {
                begin.else_clause().map(|n| {
                    n.statements()
                        .map_or(n.else_keyword_loc().end_offset(), |s| {
                            s.location().end_offset()
                        })
                })
            })
            .or_else(|| begin.rescue_clause().map(|n| n.location().end_offset()))
            .unwrap_or(end);
    }
    let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        correction: None,
        message,
        line,
        col,
        end_line: file.line_of(end.saturating_sub(1)),
    });
}

// ArgsNode#argument_list flattens destructuring and includes shadow arguments
// and explicit argument nodes in defaults. Anonymous/forwarding arguments retain
// their nil name. Only the inspected block synthesizes numbered/it arguments:
// nested numblock/itblock nodes have no argument descendants in the upstream AST.
fn argument_names(parameters: Option<Node<'_>>) -> Vec<Option<&[u8]>> {
    let Some(parameters) = parameters else {
        return Vec::new();
    };
    if let Some(n) = parameters.as_numbered_parameters_node() {
        return [
            b"_1".as_slice(),
            b"_2",
            b"_3",
            b"_4",
            b"_5",
            b"_6",
            b"_7",
            b"_8",
            b"_9",
        ]
        .into_iter()
        .take(n.maximum() as usize)
        .map(Some)
        .collect();
    }
    if parameters.as_it_parameters_node().is_some() {
        return vec![Some(b"it")];
    }
    struct Names<'pr>(Vec<Option<&'pr [u8]>>);
    impl<'pr> NodeVisitor<'pr> for Names<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            if let Some(name) = parameter_name(&node) {
                self.0.push(name);
            }
        }
    }
    let mut names = Names(Vec::new());
    walk::walk(&mut names, &parameters);
    names.0
}

fn single_statement(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(n) = node.as_statements_node() {
        let nodes = n.body();
        if nodes.is_empty() {
            return None;
        }
        if nodes.len() == 1 {
            return nodes.iter().next();
        }
    }
    Some(node)
}
fn last_value(node: Node<'_>) -> Node<'_> {
    if let Some(n) = node.as_statements_node() {
        return n.body().iter().last().unwrap_or(node);
    }
    if let Some(n) = node.as_parentheses_node()
        && let Some(body) = n.body()
    {
        return body
            .as_statements_node()
            .and_then(|n| n.body().iter().last())
            .unwrap_or(body);
    }
    node
}
fn block_boundary(node: &Node<'_>) -> bool {
    node.as_block_node().is_some()
        || node.as_lambda_node().is_some()
        || node
            .as_call_node()
            .is_some_and(|n| n.block().is_some_and(|b| b.as_block_node().is_some()))
        || node
            .as_super_node()
            .is_some_and(|n| n.block().is_some_and(|b| b.as_block_node().is_some()))
        || node
            .as_forwarding_super_node()
            .is_some_and(|n| n.block().is_some())
}
struct Returns<'pr> {
    values: Vec<Node<'pr>>,
    boundaries: Vec<bool>,
    depth: usize,
    first: bool,
}
impl<'pr> NodeVisitor<'pr> for Returns<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let first = std::mem::take(&mut self.first);
        let boundary = block_boundary(&node);
        self.boundaries.push(boundary);
        self.depth += usize::from(boundary);
        // each_descendant excludes the body root, and only next/break count.
        if first || self.depth != 0 {
            return;
        }
        let args = node
            .as_next_node()
            .and_then(|n| n.arguments())
            .or_else(|| node.as_break_node().and_then(|n| n.arguments()));
        if let Some(value) = args.and_then(|a| a.arguments().iter().next()) {
            self.values.push(value);
        }
    }
    fn leave(&mut self) {
        self.depth -= usize::from(self.boundaries.pop().unwrap());
    }
}
/// Reference-buffer names, private to this cop. Prism parses original bytes;
/// UTF8-MAC composition can turn its bare send into a reference-buffer local.
/// Query cached VariableForce declarations with ORIGINAL names and real ancestor
/// edges; never normalize shared bindings, reparse, or infer variable values.
struct SyntacticNames<'a> {
    file: &'a SourceFile,
    reads: std::collections::HashSet<(usize, usize)>,
}
impl<'a> SyntacticNames<'a> {
    fn new<'pr>(
        block: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &'a SourceFile,
        acc: Option<&'pr [u8]>,
        el: Option<&'pr [u8]>,
    ) -> Self {
        let mut syntax = Self {
            file,
            reads: Default::default(),
        };
        if [acc, el].into_iter().flatten().all(<[u8]>::is_ascii) {
            return syntax;
        }
        let Some(root) = cx.ancestors().last() else {
            return syntax;
        };
        struct Reads<'a, 'pr> {
            syntax: &'a SyntacticNames<'a>,
            ancestors: Vec<Node<'pr>>,
            cx: NodeContext<'a, 'pr>,
            block: &'a Node<'pr>,
            root: &'a Node<'pr>,
            reads: std::collections::HashSet<(usize, usize)>,
            acc: Option<&'pr [u8]>,
            el: Option<&'pr [u8]>,
            order: HeredocOrder<'a>,
        }
        impl<'pr> NodeVisitor<'pr> for Reads<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                if let Some(call) = node.as_call_node()
                    && call.is_variable_call()
                    && call.receiver().is_none()
                    && call.arguments().is_none()
                    && call.block().is_none()
                    && (self.syntax.same(Some(call.name().as_slice()), self.acc)
                        || self.syntax.same(Some(call.name().as_slice()), self.el))
                {
                    let context = NodeContext {
                        ancestors: &self.ancestors,
                        ..self.cx
                    };
                    // The internal stack ends at this block. Query the supplied
                    // outer context only if no owned closed scope intervenes.
                    let closed = self
                        .ancestors
                        .iter()
                        .rev()
                        .zip(std::iter::once(&node).chain(self.ancestors.iter().rev()))
                        .any(|(parent, child)| {
                            matches!(
                                parent,
                                Node::DefNode { .. }
                                    | Node::ClassNode { .. }
                                    | Node::ModuleNode { .. }
                                    | Node::SingletonClassNode { .. }
                            ) && variables::scope_owns_child(parent, child)
                        });
                    let locals = self.syntax.file.visible_locals(self.root);
                    let offset = node.location().start_offset();
                    let position = self.order.key(offset);
                    let before = |start| self.order.key(start) <= position;
                    if locals.declaration_names().any(|alias| {
                        alias != call.name().as_slice()
                            && self.syntax.same(Some(alias), Some(call.name().as_slice()))
                            && (locals.declared_before_with(alias, &node, &context, before)
                                || (!closed
                                    && locals
                                        .declared_before_with(alias, self.block, &self.cx, before)))
                    }) {
                        self.reads.insert((offset, node.location().end_offset()));
                    }
                }
                self.ancestors.push(node);
            }
            fn leave(&mut self) {
                self.ancestors.pop();
            }
        }
        let mut reads = Reads {
            syntax: &syntax,
            ancestors: Vec::new(),
            cx: *cx,
            block,
            root,
            reads: Default::default(),
            acc,
            el,
            order: HeredocOrder(file.heredocs(root)),
        };
        walk::walk(&mut reads, block);
        syntax.reads = reads.reads;
        syntax
    }
    fn same(&self, left: Option<&[u8]>, right: Option<&[u8]>) -> bool {
        if left == right {
            return true;
        }
        match (left, right) {
            (Some(left), Some(right)) => {
                self.file.match_pattern_source(left) == self.file.match_pattern_source(right)
            }
            _ => left == right,
        }
    }
    fn local_name<'pr>(&self, node: &Node<'pr>) -> Option<&'pr [u8]> {
        if let Some(n) = node.as_local_variable_read_node() {
            return Some(n.name().as_slice());
        }
        if node.as_it_local_variable_read_node().is_some() {
            return Some(b"it");
        }
        if self
            .reads
            .contains(&(node.location().start_offset(), node.location().end_offset()))
        {
            return node.as_call_node().map(|n| n.name().as_slice());
        }
        None
    }
}
/// Ruby lexes each heredoc body at its opener, then resumes the opener line.
/// Keep byte positions intact; only this cop's projected-read query needs these
/// ordering keys. Nested/multiple heredocs use the same main-parse excursions.
struct HeredocOrder<'a>(&'a [crate::source::heredoc_boundaries::Heredoc]);
impl HeredocOrder<'_> {
    fn key(&self, mut offset: usize) -> Vec<usize> {
        let mut key = vec![usize::MAX];
        while let Some(h) = self
            .0
            .iter()
            .filter(|h| (h.body.start..h.closing.end).contains(&offset))
            .min_by_key(|h| h.closing.end - h.body.start)
        {
            key.push(offset);
            offset = h.opening.end;
        }
        key.push(offset);
        key.reverse();
        key
    }
}
fn local(node: &Node<'_>, name: Option<&[u8]>, syntax: &SyntacticNames<'_>) -> bool {
    syntax
        .local_name(node)
        .is_some_and(|n| syntax.same(Some(n), name))
}
fn parameter_name<'pr>(node: &Node<'pr>) -> Option<Option<&'pr [u8]>> {
    macro_rules! named { ($($access:ident),*) => { $(if let Some(n) = node.$access() { return Some(Some(n.name().as_slice())); })* }; }
    named!(
        as_required_parameter_node,
        as_optional_parameter_node,
        as_required_keyword_parameter_node,
        as_optional_keyword_parameter_node,
        as_block_local_variable_node
    );
    macro_rules! optional { ($($access:ident),*) => { $(if let Some(n) = node.$access() { return Some(n.name().map(|n| n.as_slice())); })* }; }
    optional!(
        as_rest_parameter_node,
        as_keyword_rest_parameter_node,
        as_block_parameter_node
    );
    node.as_forwarding_parameter_node().map(|_| None)
}
fn local_assignment_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    if !variables::is_local_assignment(node) {
        return None;
    }
    if let Some(n) = node.as_local_variable_write_node() {
        return Some(n.name().as_slice());
    }
    if let Some(n) = node.as_local_variable_target_node() {
        return Some(n.name().as_slice());
    }
    if let Some(n) = node.as_local_variable_and_write_node() {
        return Some(n.name().as_slice());
    }
    if let Some(n) = node.as_local_variable_or_write_node() {
        return Some(n.name().as_slice());
    }
    node.as_local_variable_operator_write_node()
        .map(|n| n.name().as_slice())
}
fn single_interpolation<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let n = node.as_interpolated_string_node()?;
    let parts: Vec<_> = n.parts().iter().collect();
    let [part] = parts.as_slice() else {
        return None;
    };
    single_statement(part.as_embedded_statements_node()?.statements()?.as_node())
}
fn lvar_used(node: &Node<'_>, name: Option<&[u8]>, syntax: &SyntacticNames<'_>) -> bool {
    if local(node, name, syntax) {
        return true;
    }
    if node
        .as_local_variable_write_node()
        .is_some_and(|n| syntax.same(Some(n.name().as_slice()), name))
    {
        return true;
    }
    if let Some(call) = node.as_call_node()
        && !call.is_safe_navigation()
        && call.block().is_none_or(|n| n.as_block_node().is_none())
        && call.name().as_slice() == b"<<"
        && call.receiver().is_some_and(|n| local(&n, name, syntax))
    {
        return true;
    }
    single_interpolation(node).is_some_and(|n| local(&n, name, syntax))
}
fn accumulator_index(
    node: &Node<'_>,
    acc: Option<&[u8]>,
    el: Option<&[u8]>,
    syntax: &SyntacticNames<'_>,
) -> bool {
    let Some(n) = node.as_call_node() else {
        return false;
    };
    if n.is_safe_navigation()
        || n.block().is_some_and(|b| b.as_block_node().is_some())
        || !n.receiver().is_some_and(|r| local(&r, acc, syntax))
    {
        return false;
    }
    match n.name().as_slice() {
        b"[]=" => true,
        b"[]" => ast::call_arguments(&n)
            .iter()
            .all(|a| !lvar_used(a, el, syntax)),
        _ => false,
    }
}
fn plain_send(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|n| {
        !n.is_safe_navigation() && !n.block().is_some_and(|b| b.as_block_node().is_some())
    })
}
fn variable_name<'pr>(node: &Node<'pr>, syntax: &SyntacticNames<'_>) -> Option<&'pr [u8]> {
    if let Some(name) = syntax.local_name(node) {
        return Some(name);
    }
    macro_rules! named { ($($access:ident),*) => { $(if let Some(n) = node.$access() { return Some(n.name().as_slice()) })* }; }
    named!(
        as_local_variable_read_node,
        as_instance_variable_read_node,
        as_global_variable_read_node,
        as_class_variable_read_node
    );
    if node.as_it_local_variable_read_node().is_some() {
        return Some(b"it");
    }
    None
}
struct ElementMutation<'a, 'pr> {
    syntax: &'a SyntacticNames<'a>,
    name: Option<&'a [u8]>,
    found: bool,
    frames: Vec<(Node<'pr>, bool)>,
    containing: std::collections::HashSet<(usize, usize)>,
}
impl<'pr> NodeVisitor<'pr> for ElementMutation<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let contains = local(&node, self.name, self.syntax);
        self.frames.push((node, contains));
    }
    fn leave(&mut self) {
        let (node, contains) = self.frames.pop().unwrap();
        if contains {
            self.containing
                .insert((node.location().start_offset(), node.location().end_offset()));
            if let Some((_, parent)) = self.frames.last_mut() {
                *parent = true;
            }
        }
        if node.as_local_variable_target_node().is_none()
            && local_assignment_name(&node).is_some_and(|n| self.syntax.same(Some(n), self.name))
        {
            self.found = true;
        }
        // Index shorthand/multiple targets contain a synthetic send getter.
        macro_rules! index_getter { ($($access:ident),*) => { $(if let Some(n) = node.$access() {
            if n.receiver().is_some_and(|r| local(&r, self.name, self.syntax)) && n.arguments().is_some_and(|a| a.arguments().iter().any(|arg| variable_name(&arg, self.syntax).is_some() || plain_send(&arg))) { self.found = true; }
        })* }; }
        index_getter!(
            as_index_and_write_node,
            as_index_or_write_node,
            as_index_operator_write_node
        );
        if let Some(n) = node.as_index_target_node()
            && local(&n.receiver(), self.name, self.syntax)
            && n.arguments().is_some_and(|a| {
                a.arguments()
                    .iter()
                    .any(|arg| variable_name(&arg, self.syntax).is_some() || plain_send(&arg))
            })
        {
            self.found = true;
        }
        let Some(call) = node.as_call_node() else {
            return;
        };
        // The send under a literal block is still visited by each_node.
        if call.is_safe_navigation() {
            return;
        }
        let args = ast::call_arguments(&call);
        if !matches!(call.name().as_slice(), b"[]" | b"[]=") && args.len() >= 2 {
            self.found |= args.iter().any(|a| {
                self.containing
                    .contains(&(a.location().start_offset(), a.location().end_offset()))
            });
        }
        if call
            .receiver()
            .is_some_and(|n| local(&n, self.name, self.syntax))
            && args
                .iter()
                .any(|a| variable_name(a, self.syntax).is_some() || plain_send(a))
        {
            self.found = true;
        }
    }
}
struct ExpressionValues<'a> {
    syntax: &'a SyntacticNames<'a>,
    element: Option<&'a [u8]>,
    found: bool,
    other: bool,
    patterns: Vec<bool>,
}
impl ExpressionValues<'_> {
    fn add(&mut self, name: Option<&[u8]>) {
        if self.syntax.same(name, self.element) {
            self.found = true
        } else {
            self.other = true
        }
    }
}
impl NodeVisitor<'_> for ExpressionValues<'_> {
    fn enter(&mut self, node: Node<'_>) {
        let pattern = if matches!(
            node,
            Node::PinnedExpressionNode { .. }
                | Node::MultiWriteNode { .. }
                | Node::ForNode { .. }
                | Node::RescueNode { .. }
        ) {
            false
        } else {
            self.patterns.last().copied().unwrap_or(false)
                || matches!(
                    node,
                    Node::MatchRequiredNode { .. }
                        | Node::MatchPredicateNode { .. }
                        | Node::InNode { .. }
                )
        };
        self.patterns.push(pattern);
        if let Some(name) = variable_name(&node, self.syntax).or_else(|| {
            if pattern && node.as_local_variable_target_node().is_some() {
                None
            } else {
                local_assignment_name(&node)
            }
        }) {
            self.add(Some(name));
        }
        macro_rules! writes { ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.add(Some(n.name().as_slice())); })* }; }
        writes!(
            as_instance_variable_write_node,
            as_global_variable_write_node,
            as_class_variable_write_node,
            as_instance_variable_target_node,
            as_global_variable_target_node,
            as_class_variable_target_node,
            as_instance_variable_and_write_node,
            as_instance_variable_or_write_node,
            as_instance_variable_operator_write_node,
            as_global_variable_and_write_node,
            as_global_variable_or_write_node,
            as_global_variable_operator_write_node,
            as_class_variable_and_write_node,
            as_class_variable_or_write_node,
            as_class_variable_operator_write_node
        );
        if matches!(
            node,
            Node::MultiWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
                | Node::ConstantPathAndWriteNode { .. }
                | Node::ConstantPathOrWriteNode { .. }
                | Node::ConstantPathOperatorWriteNode { .. }
        ) {
            self.other = true;
        }
        // $(send _ _) captures the NODE, not its method name: any zero-arg
        // send is uncertainty, even el.foo. Literal blocks still have a send.
        if node.as_lambda_node().is_some() {
            self.other = true;
        }
        macro_rules! call_getter { ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.other |= !n.is_safe_navigation(); })* }; }
        call_getter!(
            as_call_and_write_node,
            as_call_or_write_node,
            as_call_operator_write_node
        );
        macro_rules! index_getter { ($($access:ident),*) => { $(if let Some(n) = node.$access() { self.other |= n.arguments().is_none_or(|a| a.arguments().is_empty()); })* }; }
        index_getter!(
            as_index_and_write_node,
            as_index_or_write_node,
            as_index_operator_write_node
        );
        if let Some(call) = node.as_call_node()
            && !call.is_safe_navigation()
            && ast::call_arguments(&call).is_empty()
            && self.syntax.local_name(&node).is_none()
        {
            self.other = true;
        }
    }
    fn leave(&mut self) {
        self.patterns.pop();
    }
}

#[cfg(test)]
mod tests {
    //! Ported RuboCop unmodified_reduce_accumulator_spec.rb examples (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov); all examples also run as fixtures.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnmodifiedReduceAccumulator")
            .collect()
    }
    #[test]
    fn explicit_and_numbered_returns_and_index_priority() {
        for method in ["reduce", "inject"] {
            let found = offenses(&format!("xs.{method}(0) do |acc, el|\n  el + 2\nend\n"));
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                format!("Ensure the accumulator `acc` will be modified by `{method}`.")
            );
            assert!(found[0].correction.is_none());
            assert_eq!(offenses(&format!("xs&.{method} {{ _2 }}")).len(), 1);
            let found = offenses(&format!("xs.{method} {{ |acc, el| next acc[0]; acc[1] }}"));
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].message,
                format!("Do not return an element of the accumulator in `{method}`.")
            );
        }
    }
    #[test]
    fn defaults_flatten_explicit_arguments_but_not_nested_implicit_arguments() {
        for (source, col) in [
            ("xs.reduce { |acc = foo { it }, el = 0| el }", 40),
            ("xs.reduce { |acc = foo { _2 }, el = 0| el }", 40),
            ("xs.reduce { |acc = -> { it }, el = 0| el }", 39),
            ("xs.reduce { |acc = -> { _2 }, el = 0| el }", 39),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(
                found[0].message,
                "Ensure the accumulator `acc` will be modified by `reduce`."
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "xs.reduce { |acc = foo { it }, el = 0| acc[el] }",
            "xs.reduce { |acc = foo { _2 }, el = 0| acc[el] }",
            "xs.reduce { |acc = (def f(...); end), el = 0| el }",
            "xs.reduce { |acc = foo { |nested| nested }, el = 0| el }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("xs.reduce { |acc = (def f(...); end), el = 0| acc[el] }");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 47, 1));
        assert_eq!(
            found[0].message,
            "Do not return an element of the accumulator in `reduce`."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn original_byte_mac_identifiers_compose_but_utf8_identifiers_stay_distinct() {
        for (param, reference) in [("e\u{301}", "é"), ("é", "e\u{301}")] {
            let source =
                format!("# coding: UTF8-MAC\nxs.reduce {{ |acc, {param}| {reference} }}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 22, 2));
            assert_eq!(
                found[0].message,
                "Ensure the accumulator `acc` will be modified by `reduce`."
            );
            assert!(found[0].correction.is_none());
            let source =
                format!("# coding: UTF8-MAC\nxs.reduce {{ |{param}, el| {reference}[0] }}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!((found[0].line, found[0].col), (2, 21));
            assert_eq!(
                found[0].message,
                "Do not return an element of the accumulator in `reduce`."
            );
            for body in [
                format!("acc[{reference}]"),
                format!("{reference} = 1; {param}"),
            ] {
                let source = format!("# coding: UTF8-MAC\nxs.reduce {{ |acc, {param}| {body} }}\n");
                assert!(offenses(&source).is_empty(), "{source:?}");
            }
            assert!(
                offenses(&format!(
                    "# coding: UTF-8\nxs.reduce {{ |acc, {param}| {reference} }}\n"
                ))
                .is_empty()
            );
            assert_eq!(
                offenses(&format!(
                    "# coding: UTF-8\nxs.reduce {{ |acc, {param}| acc[{reference}] }}\n"
                ))
                .len(),
                1
            );
        }
    }
    #[test]
    fn projected_local_declarations_follow_heredoc_lexing_not_byte_order() {
        for (name, alias) in [("é", "e\u{301}"), ("e\u{301}", "é")] {
            let source = format!(
                "# coding: UTF8-MAC\nxs.reduce do |acc = foo {{ |{name}| 1 }}, z = 0|\n  acc[<<~TEXT, {alias}]\n#{{{name} = 1}}\nTEXT\nend\n"
            );
            assert!(offenses(&source).is_empty(), "{source:?}");
            let source = format!(
                "# coding: UTF8-MAC\nxs.reduce do |acc = foo {{ |{name}| 1 }}, z = 0|\n  bar(1) do |x = <<~TEXT, {alias} = 0|\n#{{{name}}}\nTEXT\n    {alias}\n  end\nend\n"
            );
            assert!(offenses(&source).is_empty(), "{source:?}");
        }
    }
    #[test]
    fn no_offense_lookalikes_and_direct_accumulator_branches() {
        for body in [
            "acc",
            "acc + el",
            "next acc if test; el",
            "el << acc; el",
            "acc[el]",
            "x + el",
            "foo { next acc }; acc",
        ] {
            assert!(
                offenses(&format!("xs.reduce {{ |acc, el| {body} }}")).is_empty(),
                "{body}"
            );
        }
        assert!(offenses("xs.reduce { it }").is_empty());
        assert!(offenses("xs.map { |acc, el| el }").is_empty());
    }
}
