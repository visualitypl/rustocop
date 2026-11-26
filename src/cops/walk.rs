//! A depth-first walk over a Prism tree that calls back exactly once per
//! node, typed children included.
//!
//! ruby-prism's `Visit::visit` calls the enter hook, then the node's typed
//! `visit_*_node` method, then the leave hook; but where a field has a
//! concrete type (a body's `StatementsNode`, a call's `ArgumentsNode`, a
//! `begin`'s `RescueNode`, …) the generated code calls the typed method
//! directly, skipping both hooks. A walker built on the hooks alone never
//! sees those nodes; one that also overrides the typed methods sees some of
//! them twice. [`walk`] fixes that once: implement [`NodeVisitor`] instead of
//! `Visit` whenever every node (or a complete ancestor stack) matters.
//!
//! A cop does not need its own walk to see a node's ancestors: the cop walker
//! is built on this one and hands them to `check_node`
//! ([`NodeContext`](super::NodeContext)).
//!
//! The walk always visits every child, in ruby-prism's field order. A walker
//! that skips subtrees or reorders children still implements `Visit` and
//! overrides the typed methods it needs; mind the typed children there.
//!
//! ```
//! use ruby_prism::Node;
//! use rustocop::cops::walk::{NodeVisitor, walk};
//!
//! /// The depth of the deepest node in a tree.
//! #[derive(Default)]
//! struct Deepest<'pr> {
//!     ancestors: Vec<Node<'pr>>,
//!     deepest: usize,
//! }
//!
//! impl<'pr> NodeVisitor<'pr> for Deepest<'pr> {
//!     fn enter(&mut self, node: Node<'pr>) {
//!         // `ancestors` holds every ancestor of `node`, typed children too.
//!         self.ancestors.push(node);
//!         self.deepest = self.deepest.max(self.ancestors.len());
//!     }
//!
//!     fn leave(&mut self) {
//!         self.ancestors.pop();
//!     }
//! }
//!
//! let result = ruby_prism::parse(b"foo(1)");
//! let mut deepest = Deepest::default();
//! walk(&mut deepest, &result.node());
//! // Program > Statements > Call > Arguments > Integer
//! assert_eq!(deepest.deepest, 5);
//! ```

use ruby_prism::{Node, Visit};

/// Callbacks for [`walk`]: ruby-prism's enter and leave hooks, fired exactly
/// once per node, parents before children and children in field order.
pub trait NodeVisitor<'pr> {
    /// A node that can have children (ruby-prism's "branch" kinds), before
    /// its children.
    fn enter(&mut self, node: Node<'pr>);

    /// Leaves the node most recently entered and not yet left, after its
    /// children. (ruby-prism nodes are not `Clone`: keep the node from
    /// `enter` if `leave` needs it.)
    fn leave(&mut self) {}

    /// A node that cannot have children (ruby-prism's "leaf" kinds). By
    /// default it is entered and left like any other node.
    fn leaf(&mut self, node: Node<'pr>) {
        self.enter(node);
        self.leave();
    }
}

/// Walk `node` and everything below it, calling `visitor` once per node.
///
/// # Panics
///
/// Panics beyond [`super::depth::MAX_AST_DEPTH`] instead of overflowing a
/// [`super::depth::STACK_SIZE`] stack. The lint pipeline rejects such files
/// before invoking any visitor.
pub fn walk<'pr, V: NodeVisitor<'pr> + ?Sized>(visitor: &mut V, node: &Node<'pr>) {
    let mut adapter = Adapter {
        visitor,
        open: Vec::new(),
        depth: 0,
    };
    adapter.visit(node);
}

/// The `Visit` implementation behind [`walk`].
struct Adapter<'v, V: ?Sized> {
    visitor: &'v mut V,
    /// One entry per branch node entered through ruby-prism's enter hook,
    /// innermost last: whether that hook entered it in the visitor, so the
    /// matching leave hook (which gets no node) knows whether to leave it.
    open: Vec<bool>,
    depth: usize,
}

impl<'pr, V: NodeVisitor<'pr> + ?Sized> Adapter<'_, V> {
    fn check_depth(&self) {
        assert!(
            self.depth < super::depth::MAX_AST_DEPTH,
            "AST exceeds maximum depth of {}",
            super::depth::MAX_AST_DEPTH
        );
    }

    fn enter(&mut self, node: Node<'pr>) {
        self.check_depth();
        self.depth += 1;
        self.visitor.enter(node);
    }

    fn leave(&mut self) {
        self.visitor.leave();
        self.depth -= 1;
    }

    fn leaf(&mut self, node: Node<'pr>) {
        self.check_depth();
        self.visitor.leaf(node);
    }
}

/// Generates the adapter's `Visit` impl from the node kinds that ruby-prism
/// also visits as typed children, through their `visit_*_node` method,
/// skipping the hooks. Those kinds are entered and left from that method,
/// which runs on both paths; every other kind from the hooks.
///
/// The lists mirror ruby-prism 1.9's generated visitor: the kinds it calls as
/// `visitor.visit_*_node(..)` from another node's visit function, split by
/// whether its `visit` treats them as branches or leaves. Recheck them on a
/// ruby-prism upgrade; the tests compare the walk with every node of the
/// tree, and each kind's leafness with ruby-prism's, so a missing or
/// misfiled kind fails them. (`BlockArgumentNode` is only a typed child under
/// index writes, which Prism rejects as syntax errors; its test walks the
/// error-recovered tree.)
macro_rules! typed_child_visits {
    (
        branches { $($branch_method:ident: $branch:ident,)* }
        leaves { $($leaf_method:ident: $leaf:ident,)* }
    ) => {
        /// Whether a branch node is entered from its typed visit.
        fn is_typed_branch(node: &Node<'_>) -> bool {
            matches!(node, $(Node::$branch { .. })|*)
        }

        /// Whether a leaf node is visited from its typed visit.
        fn is_typed_leaf(node: &Node<'_>) -> bool {
            matches!(node, $(Node::$leaf { .. })|*)
        }

        impl<'pr, V: NodeVisitor<'pr> + ?Sized> Visit<'pr> for Adapter<'_, V> {
            fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
                let typed = is_typed_branch(&node);
                if !typed {
                    self.enter(node);
                }
                self.open.push(!typed);
            }

            fn visit_branch_node_leave(&mut self) {
                if self.open.pop() == Some(true) {
                    self.leave();
                }
            }

            fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
                if !is_typed_leaf(&node) {
                    self.leaf(node);
                }
            }

            $(
                fn $branch_method(&mut self, node: &ruby_prism::$branch<'pr>) {
                    self.enter(node.as_node());
                    ruby_prism::$branch_method(self, node);
                    self.leave();
                }
            )*

            $(
                fn $leaf_method(&mut self, node: &ruby_prism::$leaf<'pr>) {
                    self.leaf(node.as_node());
                }
            )*
        }
    };
}

typed_child_visits! {
    branches {
        visit_arguments_node: ArgumentsNode,
        visit_block_argument_node: BlockArgumentNode,
        visit_block_node: BlockNode,
        visit_call_node: CallNode,
        visit_constant_path_node: ConstantPathNode,
        visit_else_node: ElseNode,
        visit_ensure_node: EnsureNode,
        visit_parameters_node: ParametersNode,
        visit_rescue_node: RescueNode,
        visit_splat_node: SplatNode,
        visit_statements_node: StatementsNode,
    }
    leaves {
        visit_block_parameter_node: BlockParameterNode,
        visit_local_variable_target_node: LocalVariableTargetNode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::testing::{
        ERROR_TREE_TYPED_CHILDREN, FULL_TREE, Step, TYPED_CHILD_CASES, tree_steps,
    };

    /// A node's kind (`CallNode`), from ruby-prism's `Debug` dump.
    fn kind_name(node: &Node<'_>) -> String {
        let dump = format!("{node:?}");
        dump[..dump.find('(').unwrap_or(dump.len())].to_string()
    }

    /// Whether ruby-prism's own `visit` treats `node` as a leaf.
    fn prism_says_leaf(node: &Node<'_>) -> bool {
        struct FirstHook(Option<bool>);
        impl<'pr> Visit<'pr> for FirstHook {
            fn visit_branch_node_enter(&mut self, _node: Node<'pr>) {
                self.0.get_or_insert(false);
            }
            fn visit_leaf_node_enter(&mut self, _node: Node<'pr>) {
                self.0.get_or_insert(true);
            }
        }
        let mut hook = FirstHook(None);
        hook.visit(node);
        hook.0.unwrap()
    }

    /// A node as recorded: kind and byte range.
    type Id = (String, usize, usize);

    /// Records every callback, checking each against ruby-prism's leafness.
    #[derive(Default)]
    struct Recorder {
        steps: Vec<Step>,
        /// Every node entered (leaves included), in order.
        entered: Vec<Id>,
        /// Nodes entered and not yet left.
        open: Vec<Id>,
        /// Nodes passed to the wrong callback for their leafness.
        misclassified: Vec<Id>,
    }

    impl Recorder {
        fn record_enter(&mut self, node: &Node<'_>, as_leaf: bool) {
            let location = node.location();
            let id = (
                kind_name(node),
                location.start_offset(),
                location.end_offset(),
            );
            if prism_says_leaf(node) != as_leaf {
                self.misclassified.push(id.clone());
            }
            self.steps.push(Step::Enter(id.0.clone()));
            self.entered.push(id.clone());
            self.open.push(id);
        }
    }

    impl<'pr> NodeVisitor<'pr> for Recorder {
        fn enter(&mut self, node: Node<'pr>) {
            self.record_enter(&node, false);
        }

        fn leave(&mut self) {
            let (kind, ..) = self.open.pop().expect("leave without enter");
            self.steps.push(Step::Leave(kind));
        }

        fn leaf(&mut self, node: Node<'pr>) {
            self.record_enter(&node, true);
            self.leave();
        }
    }

    fn record(source: &str) -> Recorder {
        let result = ruby_prism::parse(source.as_bytes());
        let mut recorder = Recorder::default();
        walk(&mut recorder, &result.node());
        assert!(recorder.open.is_empty(), "every node is left: {source:?}");
        recorder
    }

    fn assert_parses(source: &str) {
        let result = ruby_prism::parse(source.as_bytes());
        let errors = result.errors().count();
        assert_eq!(errors, 0, "fixture must parse: {source:?}");
    }

    #[test]
    fn enters_and_leaves_every_node_of_the_full_tree_once_in_depth_first_order() {
        assert_parses(FULL_TREE);
        let recorder = record(FULL_TREE);
        assert_eq!(recorder.steps, tree_steps(FULL_TREE));
        let mut unique = recorder.entered.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), recorder.entered.len(), "no node twice");
        assert_eq!(recorder.misclassified, [] as [Id; 0]);
    }

    #[test]
    fn enters_and_leaves_every_typed_child() {
        for (source, kind) in TYPED_CHILD_CASES {
            assert_parses(source);
            let recorder = record(source);
            let expected = tree_steps(source);
            assert!(
                expected.contains(&Step::Enter(kind.to_string())),
                "oracle sees {kind} in {source:?}"
            );
            assert_eq!(recorder.steps, expected, "steps for {source:?}");
            assert_eq!(recorder.misclassified, [] as [Id; 0], "{source:?}");
        }
    }

    #[test]
    fn enters_and_leaves_typed_children_of_error_trees() {
        for source in ERROR_TREE_TYPED_CHILDREN {
            let recorder = record(source);
            let block_arguments = recorder
                .entered
                .iter()
                .filter(|(kind, ..)| kind == "BlockArgumentNode")
                .count();
            assert_eq!(block_arguments, 1, "{source:?}");
            assert_eq!(recorder.steps, tree_steps(source), "steps for {source:?}");
            assert_eq!(recorder.misclassified, [] as [Id; 0], "{source:?}");
        }
    }

    #[test]
    fn walks_a_subtree_rooted_at_a_typed_child_kind() {
        let source = "def m(a)\n  a\n  b\nend\n";
        let result = ruby_prism::parse(source.as_bytes());
        let program = result.node();
        let def = program.as_program_node().unwrap().statements().body();
        let def = def.iter().next().unwrap();
        let body = def.as_def_node().unwrap().body().unwrap();
        let mut recorder = Recorder::default();
        walk(&mut recorder, &body);
        let enter = |kind: &str| Step::Enter(kind.to_string());
        let leave = |kind: &str| Step::Leave(kind.to_string());
        assert_eq!(
            recorder.steps,
            [
                enter("StatementsNode"),
                enter("LocalVariableReadNode"),
                leave("LocalVariableReadNode"),
                enter("CallNode"),
                leave("CallNode"),
                leave("StatementsNode"),
            ]
        );
    }

    #[test]
    #[should_panic(expected = "AST exceeds maximum depth of 4096")]
    fn standalone_walk_rejects_deep_typed_child_chains() {
        struct Ignore;
        impl<'pr> NodeVisitor<'pr> for Ignore {
            fn enter(&mut self, _node: Node<'pr>) {}
        }
        // A receiver chain traverses typed CallNode children directly, not
        // through Visit::visit or its branch hooks.
        let source = format!("root{}", ".foo".repeat(5000));
        let result = ruby_prism::parse(source.as_bytes());
        assert_eq!(result.errors().count(), 0);
        stacker::grow(super::super::depth::STACK_SIZE, || {
            walk(&mut Ignore, &result.node())
        });
    }

    #[test]
    fn leaf_defaults_to_enter_then_leave() {
        #[derive(Default)]
        struct Counter {
            entered: usize,
            left: usize,
        }
        impl<'pr> NodeVisitor<'pr> for Counter {
            fn enter(&mut self, _node: Node<'pr>) {
                self.entered += 1;
            }
            fn leave(&mut self) {
                self.left += 1;
            }
        }
        // ProgramNode, StatementsNode, CallNode, ArgumentsNode, IntegerNode,
        // and the typed leaf LocalVariableTargetNode under a pattern.
        let source = "foo(1)\ncase x; in Integer => n; end\n";
        let result = ruby_prism::parse(source.as_bytes());
        let mut counter = Counter::default();
        walk(&mut counter, &result.node());
        let nodes = crate::cops::testing::preorder_kinds(source).len();
        assert_eq!((counter.entered, counter.left), (nodes, nodes));
    }
}
