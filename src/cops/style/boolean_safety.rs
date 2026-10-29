//! Lazy closed-source proofs for the boolean predicate cops, never type inference
//! for arbitrary locals/calls or an assumption that ===/is_a? are interchangeable.
use crate::cops::{
    ast,
    walk::{self, NodeVisitor},
};
use ruby_prism::Node;
use std::collections::HashMap;
#[derive(Default)]
pub(crate) struct Safety {
    pub(super) core_changed: bool,
    pub(super) constants_changed: bool,
    first_effect: Option<usize>,
    constants: HashMap<String, usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut builder = Builder {
            facts: Self::default(),
            scope: Vec::new(),
            lengths: Vec::new(),
            kinds: Vec::new(),
            defined_depth: 0,
            guarded_depth: 0,
        };
        walk::walk(&mut builder, root);
        builder.facts
    }
    pub(super) fn before_effects(&self, start: usize) -> bool {
        self.first_effect.is_none_or(|end| end > start)
    }
    pub(super) fn known_constant(&self, node: &Node<'_>, start: usize) -> bool {
        ast::const_name(node).is_some_and(|name| {
            self.constants
                .get(name.trim_start_matches("::"))
                .is_some_and(|&end| end <= start)
        })
    }
}
struct Builder {
    facts: Safety,
    scope: Vec<String>,
    lengths: Vec<usize>,
    kinds: Vec<(bool, bool, bool)>,
    defined_depth: usize,
    guarded_depth: usize,
}
impl Builder {
    fn declare(&mut self, name: &str, end: usize) {
        let qualified = self
            .scope
            .iter()
            .map(String::as_str)
            .chain([name])
            .collect::<Vec<_>>()
            .join("::");
        if self.facts.constants.insert(qualified, end).is_some() {
            self.facts.constants_changed = true;
        }
    }
}
impl<'pr> NodeVisitor<'pr> for Builder {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.facts.visits += 1;
        }
        self.lengths.push(self.scope.len());
        let module_header = self.kinds.last().is_some_and(|&(_, module, _)| module);
        let is_defined = node.as_defined_node().is_some();
        if let Some(defined) = node.as_defined_node() {
            // Resolving a namespace inside defined? can still autoload it.
            // Do not let an earlier unresolved probe mutate our later proof.
            self.facts.core_changed = true;
            let value = defined.value();
            if value.as_constant_path_node().is_some()
                && !self
                    .facts
                    .known_constant(&value, node.location().start_offset())
            {
                let end = node.location().end_offset();
                self.facts.first_effect =
                    Some(self.facts.first_effect.map_or(end, |old| old.min(end)));
            }
            if !ast::is_const(&value) && value.as_call_node().is_none() {
                self.facts.constants_changed = true;
            }
        }
        self.defined_depth += usize::from(is_defined);
        let guarded = is_defined || node.as_and_node().is_some() || node.as_or_node().is_some();
        self.guarded_depth += usize::from(guarded);
        self.kinds
            .push((is_defined, node.as_module_node().is_some(), guarded));
        if self.guarded_depth > 0
            && (node.as_module_node().is_some() || node.as_constant_write_node().is_some())
        {
            // Syntactically present declarations need not execute under a
            // short-circuit operator or defined? probe.
            self.facts.constants_changed = true;
        }
        if ast::is_const(&node)
            && self.defined_depth == 0
            && !module_header
            && !super::case_equality::core_class(&node)
            && !self
                .facts
                .known_constant(&node, node.location().start_offset())
        {
            let end = node.location().end_offset();
            self.facts.first_effect = Some(self.facts.first_effect.map_or(end, |old| old.min(end)));
        }
        if let Some(module) = node.as_module_node() {
            self.facts.core_changed = true;
            if let Some(name) = module.constant_path().as_constant_read_node() {
                let name = String::from_utf8_lossy(name.name().as_slice()).into_owned();
                self.declare(&name, node.location().end_offset());
                self.scope.push(name);
            } else {
                self.facts.constants_changed = true;
            }
            return;
        }
        if let Some(write) = node.as_constant_write_node() {
            self.facts.core_changed = true;
            if super::case_equality::immediate(&write.value()) {
                let name = String::from_utf8_lossy(write.name().as_slice()).into_owned();
                self.declare(&name, node.location().end_offset());
            } else {
                self.facts.constants_changed = true;
            }
            return;
        }
        if let Some(call) = node.as_call_node() {
            if !super::case_equality::trusted_call(&call)
                && !super::nil_comparison::trusted_call(&call)
            {
                let end = call.location().end_offset();
                self.facts.first_effect =
                    Some(self.facts.first_effect.map_or(end, |old| old.min(end)));
                if call.block().is_some() {
                    self.facts.core_changed = true;
                    self.facts.constants_changed = true;
                }
            }
            return;
        }
        if !matches!(
            node,
            Node::ProgramNode { .. }
                | Node::StatementsNode { .. }
                | Node::ArgumentsNode { .. }
                | Node::IntegerNode { .. }
                | Node::SourceLineNode { .. }
                | Node::NilNode { .. }
                | Node::TrueNode { .. }
                | Node::FalseNode { .. }
                | Node::ArrayNode { .. }
                | Node::RangeNode { .. }
                | Node::ParenthesesNode { .. }
                | Node::LocalVariableReadNode { .. }
                | Node::LocalVariableWriteNode { .. }
                | Node::ConstantReadNode { .. }
                | Node::ConstantPathNode { .. }
                | Node::DefinedNode { .. }
                | Node::AndNode { .. }
                | Node::OrNode { .. }
        ) {
            self.facts.core_changed = true;
            self.facts.constants_changed = true;
        }
    }
    fn leave(&mut self) {
        self.scope.truncate(self.lengths.pop().unwrap());
        let (defined, _, guarded) = self.kinds.pop().unwrap();
        self.defined_depth -= usize::from(defined);
        self.guarded_depth -= usize::from(guarded);
    }
}
