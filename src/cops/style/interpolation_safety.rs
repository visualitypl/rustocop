//! Lazy, bounded main-AST facts for the two safe interpolation cops.
//! Unknown earlier calls can change core dispatch; deferred declarations and
//! lexical lookup changes invalidate it globally. No Ruby value-flow is assumed.
use crate::cops::{
    ast,
    walk::{self, NodeVisitor},
};
use ruby_prism::Node;
use std::collections::{HashMap, HashSet};
#[derive(Clone, Copy, Default)]
pub(crate) struct Shape {
    pub(crate) closed: bool,
    pub(crate) movable: bool,
    pub(crate) full_conditional: bool,
}
#[derive(Default)]
pub(crate) struct Facts {
    shapes: HashMap<(usize, usize), Shape>,
    runtime_strings: HashSet<(usize, usize)>,
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
fn key(n: &Node<'_>) -> (usize, usize) {
    (n.location().start_offset(), n.location().end_offset())
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        struct Index {
            facts: Facts,
            stack: Vec<(Shape, (usize, usize), bool)>,
        }
        impl<'pr> NodeVisitor<'pr> for Index {
            fn enter(&mut self, n: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                if proven_mutable_interpolation(&n) {
                    self.facts.runtime_strings.insert(key(&n));
                }
                let atom = matches!(
                    n,
                    Node::StringNode { .. }
                        | Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::NilNode { .. }
                        | Node::TrueNode { .. }
                        | Node::FalseNode { .. }
                );
                let container = matches!(
                    n,
                    Node::StatementsNode { .. }
                        | Node::EmbeddedStatementsNode { .. }
                        | Node::InterpolatedStringNode { .. }
                        | Node::ArgumentsNode { .. }
                        | Node::IfNode { .. }
                        | Node::UnlessNode { .. }
                        | Node::ElseNode { .. }
                        | Node::ParenthesesNode { .. }
                );
                let read = matches!(
                    n,
                    Node::LocalVariableReadNode { .. }
                        | Node::InstanceVariableReadNode { .. }
                        | Node::ClassVariableReadNode { .. }
                        | Node::GlobalVariableReadNode { .. }
                        | Node::ConstantReadNode { .. }
                        | Node::EmbeddedVariableNode { .. }
                );
                let call = n.as_call_node();
                let wrapper = unfreeze(&n).is_some();
                if let Some(call) = &call
                    && !wrapper
                {
                    let at = call.location().end_offset();
                    self.facts.first_effect =
                        Some(self.facts.first_effect.map_or(at, |old| old.min(at)));
                }
                // Anything outside this deliberately closed vocabulary can declare
                // bindings, aliases, refinements, hooks, or deferred executable code.
                self.facts.changed |= !atom
                    && !container
                    && !read
                    && call.is_none()
                    && !matches!(
                        n,
                        Node::ProgramNode { .. }
                            | Node::ArrayNode { .. }
                            | Node::LocalVariableWriteNode { .. }
                            | Node::SourceLineNode { .. }
                            | Node::SourceFileNode { .. }
                            | Node::SourceEncodingNode { .. }
                    );
                self.stack.push((
                    Shape {
                        closed: atom || container || wrapper,
                        movable: atom || container || read || call.is_some(),
                        full_conditional: n.as_if_node().is_some_and(|c| {
                            c.end_keyword_loc().is_some() || c.then_keyword_loc().is_some()
                        }) || n
                            .as_unless_node()
                            .is_some_and(|c| c.end_keyword_loc().is_some()),
                    },
                    key(&n),
                    n.as_interpolated_string_node().is_some(),
                ));
            }
            fn leave(&mut self) {
                let (s, key, interpolated) = self.stack.pop().unwrap();
                if interpolated && !s.closed {
                    self.facts.first_effect =
                        Some(self.facts.first_effect.map_or(key.1, |old| old.min(key.1)));
                }
                self.facts.shapes.insert(key, s);
                if let Some((parent, _, _)) = self.stack.last_mut() {
                    parent.closed &= s.closed;
                    parent.movable &= s.movable;
                    parent.full_conditional |= s.full_conditional;
                }
            }
        }
        let mut i = Index {
            facts: Self::default(),
            stack: Vec::new(),
        };
        walk::walk(&mut i, root);
        i.facts
    }
    pub(crate) fn shape(&self, n: &Node<'_>) -> Shape {
        self.shapes.get(&key(n)).copied().unwrap_or_default()
    }
    pub(crate) fn runtime_string(&self, n: &Node<'_>) -> bool {
        n.as_interpolated_string_node().is_some() && self.runtime_strings.contains(&key(n))
    }
    pub(crate) fn core_before(&self, end: usize) -> bool {
        !self.changed && self.first_effect.is_none_or(|at| at >= end)
    }
}
/// A primitive sibling alone does not prove mutability: a later empty String
/// interpolation can make even `"#{1}#{''}"` frozen. Only accept a whole string
/// whose interpolated values are non-String primitive literals. Do not infer
/// the compiler's folding of String branches/nested interpolations or evaluate Ruby.
/// Cached separately from shapes (transparent containers can share ranges).
/// Computed once per string, not per wrapper candidate. Total immediate-part
/// work is linear; the shared facts index also serves conditional diagnostics.
fn proven_mutable_interpolation(n: &Node<'_>) -> bool {
    let Some(s) = n.as_interpolated_string_node() else {
        return false;
    };
    if s.is_frozen() {
        return false;
    }
    let mut primitive = false;
    for part in s.parts().iter() {
        if part.as_string_node().is_some() {
            continue;
        }
        let Some(statements) = part
            .as_embedded_statements_node()
            .and_then(|e| e.statements())
        else {
            return false;
        };
        if statements.body().len() != 1
            || !statements.body().iter().next().is_some_and(|value| {
                matches!(
                    value,
                    Node::IntegerNode { .. }
                        | Node::FloatNode { .. }
                        | Node::TrueNode { .. }
                        | Node::FalseNode { .. }
                        | Node::NilNode { .. }
                )
            })
        {
            return false;
        }
        primitive = true;
    }
    primitive
}
/// Exact pinned matcher, intentionally excluding ::String and argument-bearing dup.
pub(super) fn unfreeze<'pr>(n: &Node<'pr>) -> Option<(Node<'pr>, bool)> {
    let call = n.as_call_node()?;
    if call.is_safe_navigation() {
        return None;
    }
    let receiver = call.receiver()?;
    let mut args = ast::call_arguments(&call);
    let new = call.name().as_slice() == b"new"
        && receiver
            .as_constant_read_node()
            .is_some_and(|c| c.name().as_slice() == b"String");
    let string = if new {
        if args.len() != 1 {
            return None;
        }
        args.pop()?
    } else {
        if !matches!(call.name().as_slice(), b"+@" | b"dup") || !args.is_empty() {
            return None;
        }
        receiver
    };
    let s = string.as_interpolated_string_node()?;
    // Implicit concatenation alone is not interpolation.
    if !s.parts().iter().any(|p| {
        p.as_embedded_statements_node().is_some()
            || p.as_embedded_variable_node().is_some_and(|e| {
                matches!(
                    e.variable(),
                    Node::InstanceVariableReadNode { .. }
                        | Node::ClassVariableReadNode { .. }
                        | Node::GlobalVariableReadNode { .. }
                )
            })
            || p.as_interpolated_string_node().is_some()
    }) {
        return None;
    }
    Some((string, new))
}

pub(crate) static INTERPOLATION_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn interpolation_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        INTERPOLATION_FACTS.get(self, || Facts::build(root))
    }
}
