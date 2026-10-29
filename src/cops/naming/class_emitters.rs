//! Pinned ConfigurableFormatting class-emitter exemption. NodeContext owns
//! ancestry; this lazy index only avoids repeated scans of sibling classes.
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    cops::{
        NodeContext,
        walk::{NodeVisitor, walk},
    },
    source::SourceFile,
};
use ruby_prism::Node;

// BeginNode can translate to nested kwbegin/rescue/ensure nodes. Keep the
// synthetic parents distinct, even though they share one Prism location.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    Node,
    Statements,
    Rescue,
    Ensure,
}
type Parent = (usize, usize, Layer);

#[derive(Default)]
pub(crate) struct Siblings(BTreeMap<Parent, BTreeSet<Vec<u8>>>);

impl Siblings {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut builder = Builder {
            index: Self::default(),
            ancestors: Vec::new(),
        };
        walk(&mut builder, root);
        builder.index
    }
}

struct Builder<'pr> {
    index: Siblings,
    ancestors: Vec<Node<'pr>>,
}

impl<'pr> NodeVisitor<'pr> for Builder<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        if let Some(class) = node.as_class_node()
            && let Some(parent) = reference_parent(self.ancestors.iter().rev(), false)
        {
            self.index
                .0
                .entry(parent)
                .or_default()
                .insert(class.constant_path().location().as_slice().to_vec());
        }
        self.ancestors.push(node);
    }

    fn leave(&mut self) {
        self.ancestors.pop();
    }
}

/// The parent seen by ConfigurableFormatting's each_child_node(:class).
/// Singleton statement lists and else/argument wrappers have no reference-AST
/// node. Multi-expression lists do, so never search across their boundary.
/// Only methods climb enclosing singleton defs; indexed classes do not.
fn reference_parent<'a, 'pr: 'a>(
    ancestors: impl Iterator<Item = &'a Node<'pr>>,
    climb_singleton_defs: bool,
) -> Option<Parent> {
    let mut ensure_body = false;
    for node in ancestors {
        let layer = if let Some(statements) = node.as_statements_node() {
            if statements.body().iter().nth(1).is_none() {
                continue;
            }
            Layer::Statements
        } else if node.as_else_node().is_some() || node.as_arguments_node().is_some() {
            continue;
        } else if node.as_ensure_node().is_some() {
            ensure_body = true;
            continue;
        } else if let Some(begin) = node.as_begin_node() {
            if ensure_body {
                Layer::Ensure
            } else if begin.rescue_clause().is_some() {
                Layer::Rescue
            } else if begin.ensure_clause().is_some() {
                Layer::Ensure
            } else if begin.begin_keyword_loc().is_some() {
                Layer::Node
            } else {
                continue;
            }
        } else if climb_singleton_defs
            && node
                .as_def_node()
                .is_some_and(|def| def.receiver().is_some())
        {
            continue;
        } else if node.as_program_node().is_some() {
            return None;
        } else {
            Layer::Node
        };
        let loc = node.location();
        return Some((loc.start_offset(), loc.end_offset(), layer));
    }
    None
}

pub(super) fn is_class_emitter(name: &[u8], cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    let Some(parent) = reference_parent(cx.ancestors(), true) else {
        return false;
    };
    let Some(root) = cx.ancestors().next_back() else {
        return false;
    };
    file.class_emitter_siblings(root)
        .0
        .get(&parent)
        .is_some_and(|names| names.contains(name))
}
