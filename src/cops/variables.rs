//! Local variable scope analysis: the scopes of a file, the local variables
//! each declares, and every assignment and reference to them, including
//! whether an assignment's value can still be read.
//!
//! A port of RuboCop's `VariableForce` (MIT License, Copyright (c) 2012-26
//! Bozhidar Batsov) onto Prism's AST, for the cops RuboCop builds on it
//! (`Lint/UselessAssignment`, and later the unused-argument and shadowing
//! cops). [`ScopeAnalysis::of`] walks the tree once and records the result
//! as data; cops query it instead of walking scopes themselves.
//!
//! The model, as in RuboCop:
//!
//! - Scopes are the top level, `def`s, blocks and lambdas, `class`,
//!   `class << x` and `module` bodies. Blocks see their enclosing scope's
//!   variables; the others do not. A block's call, a class's name and
//!   superclass, a singleton class's object and a `def`'s receiver belong
//!   to the enclosing scope.
//! - A variable is declared by its first assignment, a parameter, a named
//!   regexp capture or a pattern variable (which declares but does not
//!   assign). Parallel-assignment locals are lexically declared before the
//!   RHS is visited, but their value writes remain after it. The declaration
//!   pass visits target shapes only, never receiver/index expressions.
//! - A reference marks the assignments it can read: the latest, and
//!   earlier ones in other branches (`if`/`case` bodies, `&&`/`||` and
//!   operator-assignment right-hand sides, loop bodies, `rescue`/`ensure`
//!   clauses) that may have run instead, unless the branches exclude each
//!   other.
//! - An assignment followed by another in the same branch is *reassigned*.
//!   A variable assigned or read from a block in an outer scope is
//!   *captured*: the block may run any time. A deferred read can see later
//!   assignments; an eager read only keeps earlier assignments live.
//! - Loops (and `begin`/`rescue` with `retry`) mark the last assignment in
//!   their body as referenced when the loop reads the variable, since the
//!   next iteration can see it; all of them when they sit in a branch.
//! - Zero-arity `super` reads the method's arguments, `binding` reads every
//!   visible variable.
//!
//! Retains RuboCop's flow model and quirks: e.g. loop marking compares
//! assignment nodes structurally, as Ruby's `Array#include?` does on its nodes.
//! Unlike upstream's VariableForce, RHS closures share the lexical identities
//! of parallel targets even without an earlier assignment. Binding
//! acquisitions accept valid ignored blocks and optimized reflective sends;
//! positional arity and ownership matter. Complete parameter headers are
//! declared before defaults. Separate mutation uncertainty protects correction
//! consumers without inventing liveness reads.

use std::ops::Range;

use ruby_prism::{
    BeginNode, BlockNode, CallNode, CaseMatchNode, CaseNode, ClassNode, DefNode, ForNode,
    ForwardingSuperNode, IfNode, InNode, LambdaNode, LocalVariableAndWriteNode,
    LocalVariableOperatorWriteNode, LocalVariableOrWriteNode, LocalVariableReadNode,
    LocalVariableTargetNode, LocalVariableWriteNode, Location, MatchPredicateNode,
    MatchRequiredNode, MatchWriteNode, ModuleNode, MultiWriteNode, Node, NodeList,
    PinnedExpressionNode, ProgramNode, RescueModifierNode, SingletonClassNode, UnlessNode,
    UntilNode, Visit, WhileNode,
};

use crate::cops::ast::{Fingerprint, fingerprint};

#[cfg(test)]
thread_local! {
    pub(crate) static CONDITION_BRANCH_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether this node is an explicit local assignment (including the synthetic
/// lvasgn targets of compound, multiple, rescue and for assignments). Pattern
/// bindings must be excluded by the caller's pattern context.
pub(crate) fn is_local_assignment(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableWriteNode { .. }
            | Node::LocalVariableTargetNode { .. }
            | Node::LocalVariableAndWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::LocalVariableOperatorWriteNode { .. }
    )
}

/// Whether a local assigned inside `condition` is explicitly read inside
/// `branch`. This is the syntactic descendant-name test used by GuardClause,
/// not value liveness: it deliberately includes nested scopes and excludes
/// the two roots, as RuboCop's `each_descendant(:lvasgn/:lvar)` does. Flow-based
/// rules should use `ScopeAnalysis` instead. Each subtree is walked once.
pub(crate) fn condition_assignment_used_in_branch(condition: &Node<'_>, branch: &Node<'_>) -> bool {
    use super::walk::{NodeVisitor, walk};
    use std::collections::HashSet;

    struct Names<'pr> {
        first: bool,
        writes: bool,
        names: HashSet<&'pr [u8]>,
        pattern_stack: Vec<bool>,
    }
    impl<'pr> NodeVisitor<'pr> for Names<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            #[cfg(test)]
            CONDITION_BRANCH_VISITS.with(|count| count.set(count.get() + 1));
            // A matching expression's value (or a pinned expression) can
            // still contain real multiple/for/rescue assignments. Their
            // targets are lvasgn, unlike the pattern/capture bindings.
            let assignment_targets = matches!(
                node,
                Node::MultiWriteNode { .. } | Node::ForNode { .. } | Node::RescueNode { .. }
            );
            let pattern = !assignment_targets
                && (self.pattern_stack.last().copied().unwrap_or(false)
                    || matches!(
                        node,
                        Node::MatchWriteNode { .. }
                            | Node::MatchRequiredNode { .. }
                            | Node::MatchPredicateNode { .. }
                            | Node::InNode { .. }
                    ));
            self.pattern_stack.push(pattern);
            let synthetic_target = matches!(
                node,
                Node::LocalVariableAndWriteNode { .. }
                    | Node::LocalVariableOrWriteNode { .. }
                    | Node::LocalVariableOperatorWriteNode { .. }
            );
            if std::mem::take(&mut self.first) && !(self.writes && synthetic_target) {
                return;
            }
            let name = if self.writes {
                match node {
                    Node::LocalVariableWriteNode { .. } => {
                        Some(node.as_local_variable_write_node().unwrap().name())
                    }
                    Node::LocalVariableTargetNode { .. } if !pattern => {
                        Some(node.as_local_variable_target_node().unwrap().name())
                    }
                    // RuboCop represents these as an op_asgn containing lvasgn.
                    Node::LocalVariableAndWriteNode { .. } => {
                        Some(node.as_local_variable_and_write_node().unwrap().name())
                    }
                    Node::LocalVariableOrWriteNode { .. } => {
                        Some(node.as_local_variable_or_write_node().unwrap().name())
                    }
                    Node::LocalVariableOperatorWriteNode { .. } => {
                        Some(node.as_local_variable_operator_write_node().unwrap().name())
                    }
                    _ => None,
                }
            } else {
                node.as_local_variable_read_node().map(|n| n.name())
            };
            if let Some(name) = name {
                self.names.insert(name.as_slice());
            }
        }
        fn leave(&mut self) {
            self.pattern_stack.pop();
        }
    }
    let mut assigned = Names {
        first: true,
        writes: true,
        names: HashSet::new(),
        pattern_stack: Vec::new(),
    };
    walk(&mut assigned, condition);
    if assigned.names.is_empty() {
        return false;
    }
    let mut used = Names {
        first: true,
        writes: false,
        names: HashSet::new(),
        pattern_stack: Vec::new(),
    };
    walk(&mut used, branch);
    assigned.names.iter().any(|name| used.names.contains(name))
}

/// Whether any assignment in `condition` has a variable name equal to the
/// *entire source* of `inner_condition` (SoleNestedConditional's exemption).
/// This is deliberately syntactic, not a read/liveness query: parentheses,
/// compound inner expressions, and constant/setter targets do not match.
/// Includes the root and nested scopes, and synthetic assignment targets,
/// but not named regexp captures or pattern bindings. Walks the condition once.
pub(crate) fn condition_assigns_inner_source(
    condition: &Node<'_>,
    inner_condition: &Node<'_>,
) -> bool {
    use super::walk::{NodeVisitor, walk};

    struct Assigned<'a> {
        source: &'a [u8],
        found: bool,
        patterns: Vec<bool>,
    }
    impl NodeVisitor<'_> for Assigned<'_> {
        fn enter(&mut self, node: Node<'_>) {
            #[cfg(test)]
            CONDITION_BRANCH_VISITS.with(|count| count.set(count.get() + 1));
            let assignment_targets = matches!(
                node,
                Node::MultiWriteNode { .. } | Node::ForNode { .. } | Node::RescueNode { .. }
            );
            let pattern = !assignment_targets
                && (self.patterns.last().copied().unwrap_or(false)
                    || matches!(
                        node,
                        Node::MatchWriteNode { .. }
                            | Node::MatchRequiredNode { .. }
                            | Node::MatchPredicateNode { .. }
                            | Node::InNode { .. }
                    ));
            self.patterns.push(pattern);
            // Compound writes have a synthetic variable assignment child in
            // RuboCop, and multi/for/rescue writes have synthetic targets.
            macro_rules! named {
                ($($access:ident),* $(,)?) => {$(
                    if let Some(n) = node.$access() {
                        self.found |= n.name().as_slice() == self.source;
                    }
                )*};
            }
            named!(
                as_local_variable_write_node,
                as_local_variable_and_write_node,
                as_local_variable_or_write_node,
                as_local_variable_operator_write_node,
                as_instance_variable_write_node,
                as_instance_variable_and_write_node,
                as_instance_variable_or_write_node,
                as_instance_variable_operator_write_node,
                as_class_variable_write_node,
                as_class_variable_and_write_node,
                as_class_variable_or_write_node,
                as_class_variable_operator_write_node,
                as_global_variable_write_node,
                as_global_variable_and_write_node,
                as_global_variable_or_write_node,
                as_global_variable_operator_write_node,
            );
            if !pattern {
                named!(
                    as_local_variable_target_node,
                    as_instance_variable_target_node,
                    as_class_variable_target_node,
                    as_global_variable_target_node,
                );
            }
        }
        fn leave(&mut self) {
            self.patterns.pop();
        }
    }
    let mut assigned = Assigned {
        source: inner_condition.location().as_slice(),
        found: false,
        patterns: Vec::new(),
    };
    walk(&mut assigned, condition);
    assigned.found
}

#[cfg(test)]
thread_local! {
    pub(crate) static VISIBLE_LOCAL_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static VISIBLE_LOCAL_EDGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Source-order declarations for calls that would become local-variable reads
/// after removing invocation parentheses. Built from the shared scope analysis,
/// not a second, subtly different implementation of Ruby variable binding.
#[derive(Default)]
pub(crate) struct VisibleLocals {
    multiple_targets: std::collections::HashMap<Vec<u8>, Vec<usize>>,
    block_declarations: std::collections::HashMap<Vec<u8>, Vec<usize>>,
    scopes:
        std::collections::HashMap<(usize, usize, u8), std::collections::HashMap<Vec<u8>, usize>>,
}

impl VisibleLocals {
    pub(crate) fn of(root: &Node<'_>) -> Self {
        #[cfg(test)]
        VISIBLE_LOCAL_BUILDS.with(|n| n.set(n.get() + 1));
        let mut result = Self::default();
        let Some(program) = root.as_program_node() else {
            return result;
        };
        let analysis = ScopeAnalysis::of(&program);
        for id in analysis.scopes_in_exit_order() {
            let scope = analysis.scope(id);
            let loc = scope.node.location();
            if scope.kind.is_block() {
                for id in scope.variables() {
                    let var = analysis.variable(id);
                    // Parameters and explicit `|; local|` declarations keep
                    // shadowing an outer local introduced before the block.
                    if !matches!(var.declaration, Declaration::Parameter(_)) {
                        result
                            .block_declarations
                            .entry(var.name.to_vec())
                            .or_default()
                            .push(var.declaration_range.start);
                    }
                }
            }
            let names = scope
                .variables()
                .map(|id| {
                    let var = analysis.variable(id);
                    (var.name.to_vec(), var.declaration_range.start)
                })
                .collect();
            result.scopes.insert(
                (
                    loc.start_offset(),
                    loc.end_offset(),
                    scope_tag(&scope.node).unwrap(),
                ),
                names,
            );
        }
        for assignment in &analysis.assignments {
            if assignment.kind == AssignmentKind::Multiple {
                let name = analysis.variable(assignment.variable).name;
                result
                    .multiple_targets
                    .entry(name.to_vec())
                    .or_default()
                    .push(assignment.name_range.start);
            }
        }
        for positions in result
            .multiple_targets
            .values_mut()
            .chain(result.block_declarations.values_mut())
        {
            positions.sort_unstable();
        }
        result
    }

    /// A block-owned declaration in source that an earlier outer declaration
    /// could turn into a capture. Includes lambdas, patterns and regexp captures;
    /// closed nested scopes may conservatively withhold an otherwise safe edit.
    pub(crate) fn block_declaration_in(&self, name: &[u8], range: Range<usize>) -> bool {
        let Some(positions) = self.block_declarations.get(name) else {
            return false;
        };
        let first = positions.partition_point(|p| *p < range.start);
        positions.get(first).is_some_and(|p| *p < range.end)
    }

    pub(crate) fn same_name_assignment(
        &self,
        call: &CallNode<'_>,
        cx: &crate::cops::NodeContext<'_, '_>,
    ) -> bool {
        cx.ancestors().any(|node| {
            let name = if let Some(n) = node.as_local_variable_write_node() {
                Some(n.name())
            } else if let Some(n) = node.as_local_variable_or_write_node() {
                Some(n.name())
            } else if let Some(n) = node.as_local_variable_and_write_node() {
                Some(n.name())
            } else {
                node.as_local_variable_operator_write_node()
                    .map(|n| n.name())
            };
            if name.is_some_and(|n| n.as_slice() == call.name().as_slice()) {
                return true;
            }
            let Some(multi) = node.as_multi_write_node() else {
                return false;
            };
            let Some(positions) = self.multiple_targets.get(call.name().as_slice()) else {
                return false;
            };
            let first = positions.partition_point(|p| *p < multi.location().start_offset());
            positions
                .get(first)
                .is_some_and(|p| *p < multi.value().location().start_offset())
        })
    }

    pub(crate) fn shadows(
        &self,
        call: &CallNode<'_>,
        cx: &crate::cops::NodeContext<'_, '_>,
    ) -> bool {
        self.declared_before(
            call.name().as_slice(),
            &call.as_node(),
            cx,
            call.location().start_offset(),
        )
    }

    /// Whether a name is already lexical at a proposed earlier source position.
    /// Used when a guard body moves before its predicate or sibling branch.
    pub(crate) fn declared_before(
        &self,
        name: &[u8],
        node: &Node<'_>,
        cx: &crate::cops::NodeContext<'_, '_>,
        offset: usize,
    ) -> bool {
        let children = std::iter::once(node).chain(cx.ancestors());
        for (parent, child) in cx.ancestors().zip(children) {
            #[cfg(test)]
            VISIBLE_LOCAL_EDGES.with(|n| n.set(n.get() + 1));
            if !scope_owns_child(parent, child) {
                continue;
            }
            let Some(tag) = scope_tag(parent) else {
                continue;
            };
            let loc = parent.location();
            let Some(names) = self
                .scopes
                .get(&(loc.start_offset(), loc.end_offset(), tag))
            else {
                continue;
            };
            if names.get(name).is_some_and(|start| *start <= offset) {
                return true;
            }
            if !matches!(parent, Node::BlockNode { .. } | Node::LambdaNode { .. }) {
                return false;
            }
        }
        false
    }
}

/// Class/module bodies and method parameters/bodies open local scopes; their
/// headers/receivers evaluate outside them. Follow the *direct child edge* in
/// the complete ancestor chain, never containment of the candidate's offset:
/// heredoc interpolation descendants can lie beyond their body's location (or
/// beyond the definition itself, for a heredoc in an endless method).
pub(crate) fn scope_owns_child(parent: &Node<'_>, child: &Node<'_>) -> bool {
    let body = match parent {
        Node::ClassNode { .. } => parent.as_class_node().unwrap().body(),
        Node::ModuleNode { .. } => parent.as_module_node().unwrap().body(),
        Node::SingletonClassNode { .. } => parent.as_singleton_class_node().unwrap().body(),
        Node::DefNode { .. } => {
            if child.as_parameters_node().is_some() {
                return true;
            }
            parent.as_def_node().unwrap().body()
        }
        _ => return true,
    };
    body.is_some_and(|body| {
        // Identify the declared direct child by kind and exact span, as for the
        // scope keys below. This is edge identity, NOT descendant containment
        // or structural/source-text equality between unrelated subtrees.
        std::mem::discriminant(&body) == std::mem::discriminant(child)
            && body.location().start_offset() == child.location().start_offset()
            && body.location().end_offset() == child.location().end_offset()
    })
}

fn scope_tag(node: &Node<'_>) -> Option<u8> {
    Some(match node {
        Node::ProgramNode { .. } => 0,
        Node::DefNode { .. } => 1,
        Node::BlockNode { .. } => 2,
        Node::LambdaNode { .. } => 3,
        Node::ClassNode { .. } => 4,
        Node::ModuleNode { .. } => 5,
        Node::SingletonClassNode { .. } => 6,
        _ => return None,
    })
}

/// Identifies a [`Scope`] in a [`ScopeAnalysis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(usize);

/// Identifies a [`Variable`] in a [`ScopeAnalysis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariableId(usize);

/// Identifies an [`Assignment`] in a [`ScopeAnalysis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssignmentId(usize);

/// Identifies a [`Reference`] in a [`ScopeAnalysis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceId(usize);

/// Identifies a branch of a conditional construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BranchId(usize);

/// What opens a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// The file.
    TopLevel,
    /// A `def`, singleton or not.
    Def,
    /// A block or a lambda: sees the enclosing scope's variables.
    Block,
    /// A `class` body.
    Class,
    /// A `class << object` body.
    SingletonClass,
    /// A `module` body.
    Module,
}

impl ScopeKind {
    /// Whether the scope sees its enclosing scope's variables.
    pub fn is_block(self) -> bool {
        self == ScopeKind::Block
    }
}

/// A scope: a context of local variable visibility.
pub struct Scope<'pr> {
    /// What opens it.
    pub kind: ScopeKind,
    /// The node opening it: `ProgramNode`, `DefNode`, `BlockNode`,
    /// `LambdaNode`, `ClassNode`, `SingletonClassNode` or `ModuleNode`.
    pub node: Node<'pr>,
    /// The enclosing scope.
    pub parent: Option<ScopeId>,
    /// The variables declared here, in declaration order; a redeclared
    /// name keeps its place and points to the newest variable.
    variables: Vec<(&'pr [u8], VariableId)>,
}

impl<'pr> Scope<'pr> {
    /// The variables declared in this scope, in declaration order.
    pub fn variables(&self) -> impl Iterator<Item = VariableId> + '_ {
        self.variables.iter().map(|&(_, id)| id)
    }

    /// The scope's body: the statements of the file, `def`, block or
    /// class, or the `BeginNode` of a body with `rescue`/`ensure`.
    pub fn body(&self) -> Option<Node<'pr>> {
        match &self.node {
            Node::ProgramNode { .. } => {
                Some(self.node.as_program_node().unwrap().statements().as_node())
            }
            Node::DefNode { .. } => self.node.as_def_node().unwrap().body(),
            Node::BlockNode { .. } => self.node.as_block_node().unwrap().body(),
            Node::LambdaNode { .. } => self.node.as_lambda_node().unwrap().body(),
            Node::ClassNode { .. } => self.node.as_class_node().unwrap().body(),
            Node::SingletonClassNode { .. } => self.node.as_singleton_class_node().unwrap().body(),
            Node::ModuleNode { .. } => self.node.as_module_node().unwrap().body(),
            _ => None,
        }
    }
}

/// How a variable came to be declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declaration {
    /// By its first assignment.
    Assignment,
    /// By a named capture of a regexp match (`/(?<name>.)/ =~ s`).
    NamedCapture,
    /// By a pattern (`in [name]`, `=> name`); pattern variables are never
    /// assignments.
    PatternMatch,
    /// As a method or block parameter.
    Parameter(ParameterKind),
}

/// The kinds of parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterKind {
    /// `a`, also inside `(a, b)` destructuring.
    Required,
    /// `a = 1`.
    Optional,
    /// `*a`.
    Rest,
    /// `a:`.
    RequiredKeyword,
    /// `a: 1`.
    OptionalKeyword,
    /// `**a`.
    KeywordRest,
    /// `&a`.
    Block,
    /// A block-local variable, `|x; a|`.
    BlockLocal,
}

/// A local variable.
pub struct Variable<'pr> {
    /// Its name.
    pub name: &'pr [u8],
    /// The scope it belongs to.
    pub scope: ScopeId,
    /// How it was declared.
    pub declaration: Declaration,
    /// Where it was declared.
    pub declaration_range: Range<usize>,
    /// Its assignments, in source (analysis) order.
    pub assignments: Vec<AssignmentId>,
    /// Whether a pattern binds this variable. These writes are tracked
    /// separately from RuboCop VariableForce-style assignments, so pattern
    /// bindings do not become candidates for useless-assignment checks.
    pub has_pattern_bindings: bool,
    /// Its references, in analysis order.
    pub references: Vec<ReferenceId>,
    /// Whether a block in an inner scope assigns or reads it.
    pub captured_by_block: bool,
    /// A read in an inner block can observe writes analyzed after it.
    referenced_from_block: bool,
    /// Global assignment cursor at the last read in the owning scope.
    /// Such a read cannot observe writes analyzed after it.
    last_eager_reference: usize,
    /// Correction-only uncertainty, never a read or an assignment. Reflective
    /// dispatch may mutate an escaped Binding without naming this local.
    uncertain_binding_mutation: bool,
}

impl Variable<'_> {
    /// Whether anything references it.
    pub fn is_referenced(&self) -> bool {
        !self.references.is_empty()
    }

    /// Whether it is declared unused by name (`_`, `_foo`).
    pub fn should_be_unused(&self) -> bool {
        self.name.starts_with(b"_")
    }

    /// Whether it is a method or block parameter.
    pub fn is_parameter(&self) -> bool {
        matches!(self.declaration, Declaration::Parameter(_))
    }
}

/// How an assignment is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentKind {
    /// `a = value`.
    Write,
    /// `a += value`, `a ||= value`, `a &&= value`.
    Operator,
    /// A target of a multiple assignment, `a, b = values`.
    Multiple,
    /// The index of a `for` loop.
    For,
    /// The exception variable of a `rescue` clause.
    Rescue,
    /// A named capture, `/(?<a>.)/ =~ s`.
    NamedCapture,
    /// Any other target without a value of its own.
    Target,
}

/// An assignment to a variable.
pub struct Assignment<'pr> {
    /// The variable assigned.
    pub variable: VariableId,
    /// The assigning node: `LocalVariableWriteNode`, a local variable
    /// operator write node, `LocalVariableTargetNode` or `MatchWriteNode`.
    pub node: Node<'pr>,
    /// How it is written.
    pub kind: AssignmentKind,
    /// The variable's name in it (a named capture's regexp).
    pub name_range: Range<usize>,
    /// Whether a reference can read the assigned value.
    pub referenced: bool,
    /// Whether another assignment in the same branch overwrites it before
    /// any reference.
    pub reassigned: bool,
    /// The condition of the nearest enclosing `while`/`until` loop, when
    /// that loop is nearer than any `for` loop or `def`.
    pub loop_condition: Option<Node<'pr>>,
    branch: Option<BranchId>,
    /// The `(conditional, condition)` ranges of the nearest modifier
    /// `if`/`unless`/`while`/`until` whose condition holds the assignment.
    modifier_conditional: Option<(Range<usize>, Range<usize>)>,
    /// Whether an `if`, `case`, `case … in` or `rescue` encloses it.
    in_branching_construct: bool,
}

/// The kinds of references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A read, `a`.
    Read,
    /// The implicit read of an operator assignment, `a += 1`.
    OperatorAssignment,
    /// A method parameter passed on by a zero-arity `super`.
    ZeroAritySuper,
    /// A variable visible to `binding`.
    Binding,
}

/// A reference to a variable.
pub struct Reference {
    /// The variable referenced.
    pub variable: VariableId,
    /// How it is referenced.
    pub kind: ReferenceKind,
    /// The referencing node's range.
    pub range: Range<usize>,
}

impl Reference {
    /// Whether the variable is named in the source (not read implicitly by
    /// `super` or `binding`).
    pub fn is_explicit(&self) -> bool {
        matches!(
            self.kind,
            ReferenceKind::Read | ReferenceKind::OperatorAssignment
        )
    }
}

/// A branch: one child of a conditional construct that may or may not run.
struct Branch {
    /// Identifies the construct.
    control: usize,
    /// Which of its children.
    slot: u32,
    /// The branch enclosing the construct, within the same scope.
    parent: Option<BranchId>,
    /// `Some` for the clauses of `rescue` and `ensure` constructs.
    handler: Option<Handler>,
    control_range: Range<usize>,
    child_range: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Handler {
    Rescue,
    Ensure,
}

/// The main body slot of every construct.
const MAIN: u32 = 0;
/// `else` slots.
const ELSE: u32 = u32::MAX;

impl Branch {
    /// The main body of `begin` with `rescue` or `ensure`: an exception can
    /// cut it short (RuboCop's `may_run_incompletely?` and
    /// `may_jump_to_other_branch?`).
    fn may_run_incompletely(&self) -> bool {
        self.handler.is_some() && self.slot == MAIN
    }
}

/// The scopes, variables, assignments and references of a file.
pub struct ScopeAnalysis<'pr> {
    scopes: Vec<Scope<'pr>>,
    exit_order: Vec<ScopeId>,
    variables: Vec<Variable<'pr>>,
    assignments: Vec<Assignment<'pr>>,
    references: Vec<Reference>,
    branches: Vec<Branch>,
    /// Aliases/acquired mutation methods can cross closed lexical scopes.
    /// Conservatively invalidate correction proofs, not ordinary liveness.
    uncertain_binding_alias: bool,
}

impl<'pr> ScopeAnalysis<'pr> {
    /// Analyze a parsed file.
    ///
    /// # Panics
    ///
    /// Panics on excessive AST depth, rather than overflowing the stack. The
    /// lint pipeline runs this same preflight before cops and reports a file
    /// error; standalone callers can use [`super::depth::check`] first.
    pub fn of(program: &ProgramNode<'pr>) -> Self {
        super::depth::check(program).unwrap_or_else(|error| panic!("{error}"));
        let mut analyzer = Analyzer {
            out: ScopeAnalysis {
                scopes: Vec::new(),
                exit_order: Vec::new(),
                variables: Vec::new(),
                assignments: Vec::new(),
                references: Vec::new(),
                branches: Vec::new(),
                uncertain_binding_alias: false,
            },
            scope_stack: Vec::new(),
            branch: None,
            controls: 0,
            modifiers: Vec::new(),
            branching_constructs: 0,
            loops: Vec::new(),
            pattern_depth: 0,
            open_loops: 0,
            loop_reads: Vec::new(),
            consumed: Vec::new(),
            consumed_stamp: 0,
            fingerprints: Vec::new(),
            parameter_defaults: None,
        };
        analyzer.visit_program_node(program);
        analyzer.out
    }

    /// Every scope, innermost first: the order in which RuboCop's
    /// `after_leaving_scope` hooks run.
    pub fn scopes_in_exit_order(&self) -> impl Iterator<Item = ScopeId> + '_ {
        self.exit_order.iter().copied()
    }

    /// A scope.
    pub fn scope(&self, id: ScopeId) -> &Scope<'pr> {
        &self.scopes[id.0]
    }

    /// A variable.
    pub fn variable(&self, id: VariableId) -> &Variable<'pr> {
        &self.variables[id.0]
    }

    /// An assignment.
    pub fn assignment(&self, id: AssignmentId) -> &Assignment<'pr> {
        &self.assignments[id.0]
    }

    /// A reference.
    pub fn reference(&self, id: ReferenceId) -> &Reference {
        &self.references[id.0]
    }

    /// Conservative evidence that this lexical identity might be reflectively
    /// written. This is deliberately stronger than liveness: unknown dispatch,
    /// saved method getters and aliases are uncertainty, not implicit reads.
    /// Definite Binding exposure is identity-local; aliases/getters may escape
    /// across closed scopes and conservatively invalidate the whole file.
    pub fn may_be_dynamically_mutated(&self, id: VariableId) -> bool {
        let variable = self.variable(id);
        self.uncertain_binding_alias
            || variable.uncertain_binding_mutation
            || variable
                .references
                .iter()
                .any(|r| self.reference(*r).kind == ReferenceKind::Binding)
    }

    /// Whether the assigned value can be read: it is referenced, or captured
    /// and not overwritten in its branch with a potentially observing read.
    /// Unlike RuboCop's broad `Assignment#used?` fallback, eager references
    /// before a write cannot keep it alive; deferred block reads still can.
    pub fn is_used(&self, id: AssignmentId) -> bool {
        let assignment = &self.assignments[id.0];
        let variable = &self.variables[assignment.variable.0];
        assignment.referenced
            || (!assignment.reassigned
                && variable.captured_by_block
                && (variable.referenced_from_block || id.0 < variable.last_eager_reference))
    }

    /// The names a misspelt variable may have meant in `scope`: its
    /// variables, and calls without receiver or arguments made in it (not
    /// in inner scopes).
    pub fn variable_like_names(&self, scope: ScopeId) -> Vec<&'pr [u8]> {
        let scope = &self.scopes[scope.0];
        let mut finder = CallNameFinder { names: Vec::new() };
        finder.scan_scope(&scope.node);
        let mut names = finder.names;
        for &(name, _) in &scope.variables {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names
    }
}

/// Collects the names of receiverless calls without arguments in a scope.
struct CallNameFinder<'pr> {
    names: Vec<&'pr [u8]>,
}

impl<'pr> CallNameFinder<'pr> {
    /// Visit the parts of a scope node that belong to the scope itself.
    fn scan_scope(&mut self, node: &Node<'pr>) {
        match node {
            Node::ProgramNode { .. } => {
                self.visit_statements_node(&node.as_program_node().unwrap().statements());
            }
            Node::DefNode { .. } => {
                let def = node.as_def_node().unwrap();
                if let Some(parameters) = def.parameters() {
                    self.visit_parameters_node(&parameters);
                }
                self.visit_optional(def.body());
            }
            Node::BlockNode { .. } => {
                let block = node.as_block_node().unwrap();
                self.visit_optional(block.parameters());
                self.visit_optional(block.body());
            }
            Node::LambdaNode { .. } => {
                let lambda = node.as_lambda_node().unwrap();
                self.visit_optional(lambda.parameters());
                self.visit_optional(lambda.body());
            }
            Node::ClassNode { .. } => self.visit_optional(node.as_class_node().unwrap().body()),
            Node::SingletonClassNode { .. } => {
                self.visit_optional(node.as_singleton_class_node().unwrap().body());
            }
            Node::ModuleNode { .. } => self.visit_optional(node.as_module_node().unwrap().body()),
            _ => {}
        }
    }

    fn visit_optional(&mut self, node: Option<Node<'pr>>) {
        if let Some(node) = node {
            self.visit(&node);
        }
    }
}

impl<'pr> Visit<'pr> for CallNameFinder<'pr> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        let variable_like = node.receiver().is_none()
            && node.arguments().is_none()
            && !node
                .block()
                .is_some_and(|block| block.as_block_argument_node().is_some());
        let name = node.name().as_slice();
        if variable_like && !self.names.contains(&name) {
            self.names.push(name);
        }
        ruby_prism::visit_call_node(self, node);
    }

    // Inner scopes: only the parts that belong to this scope.
    fn visit_def_node(&mut self, node: &DefNode<'pr>) {
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }
    }

    fn visit_block_node(&mut self, _node: &BlockNode<'pr>) {}

    fn visit_lambda_node(&mut self, _node: &LambdaNode<'pr>) {}

    fn visit_class_node(&mut self, node: &ClassNode<'pr>) {
        self.visit(&node.constant_path());
        if let Some(superclass) = node.superclass() {
            self.visit(&superclass);
        }
    }

    fn visit_singleton_class_node(&mut self, node: &SingletonClassNode<'pr>) {
        self.visit(&node.expression());
    }

    fn visit_module_node(&mut self, node: &ModuleNode<'pr>) {
        self.visit(&node.constant_path());
    }
}

/// The nearest enclosing loop or `def`, for [`Assignment::loop_condition`].
enum LoopFrame<'pr> {
    /// A `while` or `until` loop node.
    Conditional(Node<'pr>),
    /// A `for` loop or a `def`.
    Other,
}

/// Where a loop's reads and assignments start in the analysis.
struct LoopMark {
    assignments: usize,
    reads: usize,
}

/// The walk building a [`ScopeAnalysis`].
struct Analyzer<'pr> {
    out: ScopeAnalysis<'pr>,
    scope_stack: Vec<ScopeId>,
    /// The innermost branch enclosing the current node within its scope.
    branch: Option<BranchId>,
    /// Counts conditional constructs, to identify them.
    controls: usize,
    /// Modifier conditionals whose condition is being walked.
    modifiers: Vec<(Range<usize>, Range<usize>)>,
    /// How many `if`/`unless`/`case`/`case … in`/`rescue` constructs
    /// enclose the current node, in any scope.
    branching_constructs: usize,
    loops: Vec<LoopFrame<'pr>>,
    /// How many patterns enclose the current node.
    pattern_depth: usize,
    /// How many loops (or retried `rescue`s) are being walked.
    open_loops: usize,
    /// Names read (or operator-assigned) inside the open loops.
    loop_reads: Vec<&'pr [u8]>,
    /// Per branch, the stamp of the last reference that consumed it.
    consumed: Vec<u32>,
    consumed_stamp: u32,
    /// Cached fingerprints of `Write` assignments' values.
    fingerprints: Vec<Option<Fingerprint<'pr>>>,
    /// Declare the whole parameter header before evaluating defaults. A
    /// Binding in an optional value can already expose later &parameters and
    /// block locals; this is lexical declaration, not an early value write.
    parameter_defaults: Option<Vec<Node<'pr>>>,
}

fn range(location: &Location<'_>) -> Range<usize> {
    location.start_offset()..location.end_offset()
}

fn covers(container: &Range<usize>, node: &Range<usize>) -> bool {
    container.start <= node.start && node.end <= container.end
}

impl<'pr> Analyzer<'pr> {
    // --- scopes -----------------------------------------------------------

    fn current_scope(&self) -> ScopeId {
        *self.scope_stack.last().expect("inside the top-level scope")
    }

    /// Enter a scope's own part; returns the branch to restore on leaving.
    fn enter_scope(&mut self, kind: ScopeKind, node: Node<'pr>) -> Option<BranchId> {
        let id = ScopeId(self.out.scopes.len());
        self.out.scopes.push(Scope {
            kind,
            node,
            parent: self.scope_stack.last().copied(),
            variables: Vec::new(),
        });
        self.scope_stack.push(id);
        // Branches do not reach into an inner scope.
        self.branch.take()
    }

    fn leave_scope(&mut self, branch: Option<BranchId>) {
        let id = self.scope_stack.pop().expect("a scope to leave");
        self.out.exit_order.push(id);
        self.branch = branch;
    }

    fn visit_optional(&mut self, node: Option<Node<'pr>>) {
        if let Some(node) = node {
            self.visit(&node);
        }
    }

    // --- variables --------------------------------------------------------

    fn declare(
        &mut self,
        name: &'pr [u8],
        declaration: Declaration,
        at: Range<usize>,
    ) -> VariableId {
        let scope = self.current_scope();
        let id = VariableId(self.out.variables.len());
        self.out.variables.push(Variable {
            name,
            scope,
            declaration,
            declaration_range: at,
            assignments: Vec::new(),
            has_pattern_bindings: false,
            references: Vec::new(),
            captured_by_block: false,
            referenced_from_block: false,
            last_eager_reference: 0,
            uncertain_binding_mutation: false,
        });
        let variables = &mut self.out.scopes[scope.0].variables;
        match variables.iter_mut().find(|(known, _)| *known == name) {
            Some(entry) => entry.1 = id,
            None => variables.push((name, id)),
        }
        id
    }

    fn declare_unless_visible(
        &mut self,
        name: &'pr [u8],
        declaration: Declaration,
        at: Range<usize>,
    ) {
        if self.find_variable(name).is_none() {
            self.declare(name, declaration, at);
        }
    }

    /// A depth-zero local belongs to this scope, even if a parallel target
    /// predeclared an outer namesake. An earlier LHS receiver's closure was
    /// parsed before later targets: value-flow order must not make it capture
    /// those speculative declarations.
    fn declare_local(
        &mut self,
        name: &'pr [u8],
        depth: u32,
        declaration: Declaration,
        at: Range<usize>,
    ) {
        if depth == 0 {
            let scope = self.current_scope();
            if !self.out.scopes[scope.0]
                .variables
                .iter()
                .any(|&(known, _)| known == name)
            {
                self.declare(name, declaration, at);
            }
        } else {
            self.declare_unless_visible(name, declaration, at);
        }
    }

    /// The variable a name refers to from the current scope.
    fn find_variable(&self, name: &[u8]) -> Option<VariableId> {
        for &scope in self.scope_stack.iter().rev() {
            let scope = &self.out.scopes[scope.0];
            if let Some(&(_, id)) = scope.variables.iter().find(|(known, _)| *known == name) {
                return Some(id);
            }
            if !scope.kind.is_block() {
                return None;
            }
        }
        None
    }

    /// Every variable visible from the current scope.
    fn accessible_variables(&self) -> Vec<VariableId> {
        let mut variables = Vec::new();
        for &scope in self.scope_stack.iter().rev() {
            let scope = &self.out.scopes[scope.0];
            variables.extend(scope.variables.iter().map(|&(_, id)| id));
            if !scope.kind.is_block() {
                break;
            }
        }
        variables
    }

    fn mark_captured_if_in_block(&mut self, variable: VariableId) {
        let current = self.current_scope();
        if self.out.scopes[current.0].kind.is_block()
            && self.out.variables[variable.0].scope != current
        {
            self.out.variables[variable.0].captured_by_block = true;
        }
    }

    // --- assignments ------------------------------------------------------

    fn assign(
        &mut self,
        name: &'pr [u8],
        node: Node<'pr>,
        kind: AssignmentKind,
        name_range: Range<usize>,
    ) {
        let variable = match self.find_variable(name) {
            Some(variable) => variable,
            None => self.declare(name, Declaration::Assignment, name_range.clone()),
        };
        self.mark_captured_if_in_block(variable);
        let loop_condition = match self.loops.last() {
            Some(LoopFrame::Conditional(node)) => Some(match node {
                Node::WhileNode { .. } => node.as_while_node().unwrap().predicate(),
                _ => node.as_until_node().unwrap().predicate(),
            }),
            _ => None,
        };
        let id = AssignmentId(self.out.assignments.len());
        self.out.assignments.push(Assignment {
            variable,
            node,
            kind,
            name_range,
            referenced: false,
            reassigned: false,
            loop_condition,
            branch: self.branch,
            modifier_conditional: self.modifiers.last().cloned(),
            in_branching_construct: self.branching_constructs > 0,
        });
        let record = &self.out.variables[variable.0];
        if !record.captured_by_block
            && let Some(&last) = record.assignments.last()
        {
            let last = &mut self.out.assignments[last.0];
            if last.branch == self.branch && !last.referenced {
                last.reassigned = true;
            }
        }
        self.out.variables[variable.0].assignments.push(id);
    }

    /// Register lexical locals without evaluating receiver/index expressions or
    /// recording value writes. Prism parses parallel LHS bindings before its RHS,
    /// even when a target is nested or appears after a splat.
    fn declare_target(&mut self, target: &Node<'pr>) {
        match target {
            Node::LocalVariableTargetNode { .. } => {
                let local = target.as_local_variable_target_node().unwrap();
                self.declare_local(
                    local.name().as_slice(),
                    local.depth(),
                    Declaration::Assignment,
                    range(&local.location()),
                );
            }
            Node::MultiTargetNode { .. } => {
                let multi = target.as_multi_target_node().unwrap();
                for target in multi
                    .lefts()
                    .iter()
                    .chain(multi.rest())
                    .chain(multi.rights().iter())
                {
                    self.declare_target(&target);
                }
            }
            Node::SplatNode { .. } => {
                if let Some(expression) = target.as_splat_node().unwrap().expression() {
                    self.declare_target(&expression);
                }
            }
            _ => {}
        }
    }

    fn assign_target(&mut self, target: &LocalVariableTargetNode<'pr>, kind: AssignmentKind) {
        let at = range(&target.location());
        self.declare_local(
            target.name().as_slice(),
            target.depth(),
            Declaration::Assignment,
            at.clone(),
        );
        self.assign(target.name().as_slice(), target.as_node(), kind, at);
    }

    /// Visit the targets of a multiple assignment or `for` loop.
    fn visit_target(&mut self, target: &Node<'pr>, kind: AssignmentKind) {
        match target {
            Node::LocalVariableTargetNode { .. } => {
                self.assign_target(&target.as_local_variable_target_node().unwrap(), kind);
            }
            Node::MultiTargetNode { .. } => {
                let multi = target.as_multi_target_node().unwrap();
                self.visit_targets(&multi.lefts(), multi.rest(), &multi.rights(), kind);
            }
            Node::SplatNode { .. } => {
                if let Some(expression) = target.as_splat_node().unwrap().expression() {
                    self.visit_target(&expression, kind);
                }
            }
            _ => self.visit(target),
        }
    }

    fn visit_targets(
        &mut self,
        lefts: &NodeList<'pr>,
        rest: Option<Node<'pr>>,
        rights: &NodeList<'pr>,
        kind: AssignmentKind,
    ) {
        for target in lefts.iter() {
            self.visit_target(&target, kind);
        }
        if let Some(rest) = rest {
            self.visit_target(&rest, kind);
        }
        for target in rights.iter() {
            self.visit_target(&target, kind);
        }
    }

    /// `a op= value` (RuboCop's `process_variable_operator_assignment`).
    fn operator_write(
        &mut self,
        name: &'pr [u8],
        depth: u32,
        node: Node<'pr>,
        name_range: Range<usize>,
        value: Node<'pr>,
    ) {
        self.declare_local(name, depth, Declaration::Assignment, name_range.clone());
        // `a += a = 1` reads `a` before the right-hand side runs.
        let at = range(&node.location());
        self.read(name, at.clone(), ReferenceKind::OperatorAssignment);
        let control = self.new_control();
        self.in_branch(control, 1, None, at, range(&value.location()), |this| {
            this.visit(&value);
        });
        self.assign(name, node, AssignmentKind::Operator, name_range);
    }

    // --- references -------------------------------------------------------

    fn read(&mut self, name: &'pr [u8], at: Range<usize>, kind: ReferenceKind) {
        if self.open_loops > 0 {
            self.loop_reads.push(name);
        }
        let Some(variable) = self.find_variable(name) else {
            return;
        };
        self.mark_captured_if_in_block(variable);
        self.reference(variable, at, kind);
    }

    /// RuboCop's `Variable#reference!`.
    fn reference(&mut self, variable: VariableId, at: Range<usize>, kind: ReferenceKind) {
        let id = ReferenceId(self.out.references.len());
        self.out.references.push(Reference {
            variable,
            kind,
            range: at.clone(),
        });
        let current = self.current_scope();
        let record = &mut self.out.variables[variable.0];
        record.references.push(id);
        if self.out.scopes[current.0].kind.is_block() && record.scope != current {
            record.referenced_from_block = true;
        } else {
            record.last_eager_reference = self.out.assignments.len();
        }
        let reference_branch = self.branch;
        // Branches whose assignments this reference can no longer see: a
        // later assignment in them always runs first. Marked with a stamp
        // per reference, so the check stays constant-time on variables
        // assigned in hundreds of branches.
        self.consumed_stamp += 1;
        self.consumed.resize(self.out.branches.len(), 0);
        let count = self.out.variables[variable.0].assignments.len();
        for index in (0..count).rev() {
            let assignment = self.out.variables[variable.0].assignments[index];
            let branch = self.out.assignments[assignment.0].branch;
            if branch.is_some_and(|branch| self.consumed[branch.0] == self.consumed_stamp) {
                continue;
            }
            if !self.run_exclusively(branch, reference_branch) {
                self.out.assignments[assignment.0].referenced = true;
            }
            // An assignment in a modifier condition does not put the
            // variable in scope for the modifier's body: look further back.
            if let Some((conditional, condition)) =
                &self.out.assignments[assignment.0].modifier_conditional
                && covers(conditional, &at)
                && !covers(condition, &at)
            {
                continue;
            }
            let Some(branch) = branch else {
                break;
            };
            if Some(branch) == reference_branch {
                break;
            }
            if !(self.out.branches[branch.0].may_run_incompletely()
                || self.exception_may_reach(branch, &at))
            {
                self.consumed[branch.0] = self.consumed_stamp;
            }
        }
    }

    /// Whether two branches never both run (RuboCop's
    /// `run_exclusively_with?`).
    fn run_exclusively(&self, a: Option<BranchId>, b: Option<BranchId>) -> bool {
        let (Some(a), Some(b)) = (a, b) else {
            return false;
        };
        let mut current = Some(a);
        while let Some(branch) = current {
            let branch = &self.out.branches[branch.0];
            if branch.may_run_incompletely() {
                return false;
            }
            let mut other = Some(b);
            while let Some(candidate) = other {
                let candidate = &self.out.branches[candidate.0];
                if candidate.control == branch.control {
                    return candidate.slot != branch.slot;
                }
                other = candidate.parent;
            }
            current = branch.parent;
        }
        false
    }

    /// Whether an exception raised in a `begin` body enclosing `branch` can
    /// carry its assignment to the reference at `at`.
    fn exception_may_reach(&self, branch: BranchId, at: &Range<usize>) -> bool {
        let mut ancestor = self.out.branches[branch.0].parent;
        while let Some(id) = ancestor {
            let branch = &self.out.branches[id.0];
            if branch.may_run_incompletely()
                && !covers(&branch.child_range, at)
                && (branch.handler == Some(Handler::Rescue) || covers(&branch.control_range, at))
            {
                return true;
            }
            ancestor = branch.parent;
        }
        false
    }

    // --- branches and loops -----------------------------------------------

    fn new_control(&mut self) -> usize {
        self.controls += 1;
        self.controls
    }

    fn in_branch(
        &mut self,
        control: usize,
        slot: u32,
        handler: Option<Handler>,
        control_range: Range<usize>,
        child_range: Range<usize>,
        walk: impl FnOnce(&mut Self),
    ) {
        let id = BranchId(self.out.branches.len());
        self.out.branches.push(Branch {
            control,
            slot,
            parent: self.branch,
            handler,
            control_range,
            child_range,
        });
        let outer = self.branch.replace(id);
        walk(self);
        self.branch = outer;
    }

    fn begin_loop(&mut self) -> LoopMark {
        self.open_loops += 1;
        LoopMark {
            assignments: self.out.assignments.len(),
            reads: self.loop_reads.len(),
        }
    }

    /// RuboCop's `mark_assignments_as_referenced_in_loop`: a variable read
    /// in the loop can see the loop's last assignment to it on the next
    /// iteration, and every one in a branch.
    fn end_loop(&mut self, mark: LoopMark) {
        self.open_loops -= 1;
        let mut names: Vec<&'pr [u8]> = self.loop_reads[mark.reads..].to_vec();
        names.sort_unstable();
        names.dedup();
        // The loop's assignments by name, built when first needed.
        let mut in_loop: Option<Vec<(&'pr [u8], AssignmentId)>> = None;
        for name in names {
            let Some(variable) = self.find_variable(name) else {
                continue;
            };
            let count = self.out.variables[variable.0].assignments.len();
            // The variable's last assignment in the loop. RuboCop compares
            // nodes structurally (`Array#include?`), so an earlier
            // assignment equal to one in the loop counts as in it.
            let mut last = None;
            for index in (0..count).rev() {
                let id = self.out.variables[variable.0].assignments[index];
                if !self.is_lvasgn(id) {
                    continue;
                }
                if id.0 >= mark.assignments {
                    last = Some(id);
                    break;
                }
                let in_loop = in_loop.get_or_insert_with(|| self.loop_index(mark.assignments));
                if self.equals_one_in_loop(id, in_loop) {
                    last = Some(id);
                    break;
                }
            }
            let Some(last) = last else {
                continue;
            };
            for index in 0..count {
                let id = self.out.variables[variable.0].assignments[index];
                if !self.is_lvasgn(id) || !self.out.assignments[id.0].in_branching_construct {
                    continue;
                }
                let inside = id.0 >= mark.assignments || {
                    let in_loop = in_loop.get_or_insert_with(|| self.loop_index(mark.assignments));
                    self.equals_one_in_loop(id, in_loop)
                };
                if inside {
                    self.out.assignments[id.0].referenced = true;
                }
            }
            self.out.assignments[last.0].referenced = true;
        }
        if self.open_loops == 0 {
            self.loop_reads.clear();
        }
    }

    /// Whether RuboCop sees the assignment as an `lvasgn` node (named
    /// captures are `match_with_lvasgn`).
    fn is_lvasgn(&self, id: AssignmentId) -> bool {
        self.out.assignments[id.0].kind != AssignmentKind::NamedCapture
    }

    /// The `lvasgn` assignments from `start` on, sorted by name.
    fn loop_index(&self, start: usize) -> Vec<(&'pr [u8], AssignmentId)> {
        let mut index: Vec<(&'pr [u8], AssignmentId)> = (start..self.out.assignments.len())
            .map(AssignmentId)
            .filter(|&id| self.is_lvasgn(id))
            .map(|id| {
                (
                    self.out.variables[self.out.assignments[id.0].variable.0].name,
                    id,
                )
            })
            .collect();
        index.sort_unstable();
        index
    }

    /// Whether an assignment before a loop equals, as RuboCop compares
    /// nodes, one of the loop's (`in_loop`, from [`Self::loop_index`]).
    fn equals_one_in_loop(
        &mut self,
        id: AssignmentId,
        in_loop: &[(&'pr [u8], AssignmentId)],
    ) -> bool {
        let name = self.out.variables[self.out.assignments[id.0].variable.0].name;
        let has_value = self.out.assignments[id.0].kind == AssignmentKind::Write;
        let first = in_loop.partition_point(|&(other, _)| other < name);
        for &(other_name, other) in &in_loop[first..] {
            if other_name != name {
                break;
            }
            let other_has_value = self.out.assignments[other.0].kind == AssignmentKind::Write;
            match (has_value, other_has_value) {
                (false, false) => return true,
                (true, true) => {
                    self.cache_fingerprint(id);
                    self.cache_fingerprint(other);
                    if self.fingerprints[id.0] == self.fingerprints[other.0] {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Compute the structural fingerprint of a `Write` assignment's value
    /// once: large generated loops compare the same assignments many times.
    fn cache_fingerprint(&mut self, id: AssignmentId) {
        if self.fingerprints.len() <= id.0 {
            self.fingerprints.resize(self.out.assignments.len(), None);
        }
        if self.fingerprints[id.0].is_none() {
            self.fingerprints[id.0] =
                assignment_value(&self.out.assignments[id.0]).map(|value| fingerprint(&value));
        }
    }

    fn visit_statements(&mut self, statements: Option<ruby_prism::StatementsNode<'pr>>) {
        if let Some(statements) = statements {
            self.visit_statements_node(&statements);
        }
    }

    /// Visit a conditional body as a branch.
    fn visit_branch_statements(
        &mut self,
        control: usize,
        slot: u32,
        control_range: Range<usize>,
        statements: Option<ruby_prism::StatementsNode<'pr>>,
    ) {
        if let Some(statements) = statements {
            let child = range(&statements.location());
            self.in_branch(control, slot, None, control_range, child, |this| {
                this.visit_statements_node(&statements);
            });
        }
    }

    /// A `while`/`until` loop, `begin … end while` included.
    fn visit_loop(
        &mut self,
        node: Node<'pr>,
        predicate: Node<'pr>,
        statements: Option<ruby_prism::StatementsNode<'pr>>,
        post: bool,
        modifier: bool,
    ) {
        let at = range(&node.location());
        self.loops.push(LoopFrame::Conditional(node));
        let mark = self.begin_loop();
        let control = self.new_control();
        if post {
            self.visit_branch_statements(control, 1, at, statements);
            self.visit(&predicate);
        } else {
            if modifier {
                self.modifiers
                    .push((at.clone(), range(&predicate.location())));
                self.visit(&predicate);
                self.modifiers.pop();
            } else {
                self.visit(&predicate);
            }
            self.visit_branch_statements(control, 1, at, statements);
        }
        self.end_loop(mark);
        self.loops.pop();
    }

    /// A conditional's predicate, recording a modifier form.
    fn visit_predicate(&mut self, node_range: Range<usize>, predicate: &Node<'pr>, modifier: bool) {
        if modifier {
            self.modifiers
                .push((node_range, range(&predicate.location())));
            self.visit(predicate);
            self.modifiers.pop();
        } else {
            self.visit(predicate);
        }
    }

    /// The main body, `rescue` clauses and `else` of a `begin`.
    fn visit_rescued(&mut self, node: &BeginNode<'pr>) {
        let Some(first) = node.rescue_clause() else {
            self.visit_statements(node.statements());
            if let Some(else_clause) = node.else_clause() {
                self.visit_else_node(&else_clause);
            }
            return;
        };
        self.branching_constructs += 1;
        // `retry` makes the construct a loop.
        let mark = contains_retry(&first.as_node()).then(|| self.begin_loop());
        let control = self.new_control();
        let at = range(&node.location());
        if let Some(statements) = node.statements() {
            let child = range(&statements.location());
            self.in_branch(
                control,
                MAIN,
                Some(Handler::Rescue),
                at.clone(),
                child,
                |this| {
                    this.visit_statements_node(&statements);
                },
            );
        }
        let mut clause = Some(first);
        let mut slot = 1;
        while let Some(rescue) = clause {
            let child = range(&rescue.location());
            self.in_branch(
                control,
                slot,
                Some(Handler::Rescue),
                at.clone(),
                child,
                |this| {
                    for exception in rescue.exceptions().iter() {
                        this.visit(&exception);
                    }
                    if let Some(reference) = rescue.reference() {
                        match reference.as_local_variable_target_node() {
                            Some(target) => this.assign_target(&target, AssignmentKind::Rescue),
                            None => this.visit(&reference),
                        }
                    }
                    this.visit_statements(rescue.statements());
                },
            );
            clause = rescue.subsequent();
            slot += 1;
        }
        if let Some(else_clause) = node.else_clause() {
            let child = range(&else_clause.location());
            self.in_branch(control, ELSE, Some(Handler::Rescue), at, child, |this| {
                this.visit_else_node(&else_clause);
            });
        }
        if let Some(mark) = mark {
            self.end_loop(mark);
        }
        self.branching_constructs -= 1;
    }

    /// Visit a pattern: its local variable targets are pattern variables.
    fn visit_pattern(&mut self, pattern: &Node<'pr>) {
        self.pattern_depth += 1;
        self.visit(pattern);
        self.pattern_depth -= 1;
    }

    fn parameter_header(&mut self, visit: impl FnOnce(&mut Self)) {
        let owns_defaults = self.parameter_defaults.is_none();
        if owns_defaults {
            self.parameter_defaults = Some(Vec::new());
        }
        visit(self);
        if owns_defaults {
            for value in self.parameter_defaults.take().unwrap() {
                self.visit(&value);
            }
        }
    }

    fn parameter_default(&mut self, value: Node<'pr>) {
        if let Some(defaults) = &mut self.parameter_defaults {
            defaults.push(value);
        } else {
            self.visit(&value);
        }
    }

    fn mark_binding_mutation_uncertain(&mut self) {
        // No liveness references, writes, capture flags or assignment-flow
        // changes: this evidence is exclusively for correction proofs.
        for id in self.accessible_variables() {
            self.out.variables[id.0].uncertain_binding_mutation = true;
        }
    }

    fn reference_binding(&mut self, at: Range<usize>) {
        let mut names = std::collections::HashSet::new();
        for variable in self.accessible_variables() {
            // A Binding cannot access an outer namesake hidden by a block
            // parameter/local, unlike zero-arity super.
            if names.insert(self.out.variables[variable.0].name) {
                self.mark_captured_if_in_block(variable);
                self.reference(variable, at.clone(), ReferenceKind::Binding);
            }
        }
    }

    /// RuboCop's `process_zero_arity_super` and `binding` handling: an
    /// implicit reference to these variables.
    fn reference_all(&mut self, variables: Vec<VariableId>, at: Range<usize>, kind: ReferenceKind) {
        for variable in variables {
            self.reference(variable, at.clone(), kind);
        }
    }
}

/// The value of a `LocalVariableWriteNode` assignment.
fn assignment_value<'pr>(assignment: &Assignment<'pr>) -> Option<Node<'pr>> {
    assignment
        .node
        .as_local_variable_write_node()
        .map(|write| write.value())
}

/// Whether a `retry` occurs anywhere below `node`.
fn contains_retry(node: &Node<'_>) -> bool {
    struct Finder(bool);
    impl<'pr> Visit<'pr> for Finder {
        fn visit_retry_node(&mut self, _node: &ruby_prism::RetryNode<'pr>) {
            self.0 = true;
        }
    }
    let mut finder = Finder(false);
    finder.visit(node);
    finder.0
}

impl<'pr> Visit<'pr> for Analyzer<'pr> {
    // --- scopes -----------------------------------------------------------

    fn visit_program_node(&mut self, node: &ProgramNode<'pr>) {
        let outer = self.enter_scope(ScopeKind::TopLevel, node.as_node());
        self.visit_statements_node(&node.statements());
        self.leave_scope(outer);
    }

    fn visit_def_node(&mut self, node: &DefNode<'pr>) {
        self.visit_optional(node.receiver());
        self.loops.push(LoopFrame::Other);
        let outer = self.enter_scope(ScopeKind::Def, node.as_node());
        if let Some(parameters) = node.parameters() {
            self.visit_parameters_node(&parameters);
        }
        self.visit_optional(node.body());
        self.leave_scope(outer);
        self.loops.pop();
    }

    fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
        let outer = self.enter_scope(ScopeKind::Block, node.as_node());
        self.visit_optional(node.parameters());
        self.visit_optional(node.body());
        self.leave_scope(outer);
    }

    fn visit_lambda_node(&mut self, node: &LambdaNode<'pr>) {
        let outer = self.enter_scope(ScopeKind::Block, node.as_node());
        self.visit_optional(node.parameters());
        self.visit_optional(node.body());
        self.leave_scope(outer);
    }

    fn visit_class_node(&mut self, node: &ClassNode<'pr>) {
        self.visit(&node.constant_path());
        self.visit_optional(node.superclass());
        let outer = self.enter_scope(ScopeKind::Class, node.as_node());
        self.visit_optional(node.body());
        self.leave_scope(outer);
    }

    fn visit_singleton_class_node(&mut self, node: &SingletonClassNode<'pr>) {
        self.visit(&node.expression());
        let outer = self.enter_scope(ScopeKind::SingletonClass, node.as_node());
        self.visit_optional(node.body());
        self.leave_scope(outer);
    }

    fn visit_module_node(&mut self, node: &ModuleNode<'pr>) {
        self.visit(&node.constant_path());
        let outer = self.enter_scope(ScopeKind::Module, node.as_node());
        self.visit_optional(node.body());
        self.leave_scope(outer);
    }

    // --- declarations -----------------------------------------------------

    fn visit_numbered_parameters_node(&mut self, node: &ruby_prism::NumberedParametersNode<'pr>) {
        // Implicit parameters still have binding identities. Their synthetic
        // declaration location must never be used as a source rename range.
        const NAMES: [&[u8]; 9] = [
            b"_1", b"_2", b"_3", b"_4", b"_5", b"_6", b"_7", b"_8", b"_9",
        ];
        for &name in NAMES.iter().take(node.maximum() as usize) {
            self.declare(
                name,
                Declaration::Parameter(ParameterKind::Required),
                range(&node.location()),
            );
        }
    }

    fn visit_it_parameters_node(&mut self, node: &ruby_prism::ItParametersNode<'pr>) {
        self.declare(
            b"it",
            Declaration::Parameter(ParameterKind::Required),
            range(&node.location()),
        );
    }

    fn visit_parameters_node(&mut self, node: &ruby_prism::ParametersNode<'pr>) {
        self.parameter_header(|this| ruby_prism::visit_parameters_node(this, node));
    }

    fn visit_block_parameters_node(&mut self, node: &ruby_prism::BlockParametersNode<'pr>) {
        self.parameter_header(|this| ruby_prism::visit_block_parameters_node(this, node));
    }

    fn visit_required_parameter_node(&mut self, node: &ruby_prism::RequiredParameterNode<'pr>) {
        let kind = Declaration::Parameter(ParameterKind::Required);
        self.declare(node.name().as_slice(), kind, range(&node.location()));
    }

    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'pr>) {
        let kind = Declaration::Parameter(ParameterKind::Optional);
        self.declare(node.name().as_slice(), kind, range(&node.name_loc()));
        self.parameter_default(node.value());
    }

    fn visit_rest_parameter_node(&mut self, node: &ruby_prism::RestParameterNode<'pr>) {
        if let Some(name) = node.name() {
            let kind = Declaration::Parameter(ParameterKind::Rest);
            self.declare(name.as_slice(), kind, range(&node.location()));
        }
    }

    fn visit_required_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::RequiredKeywordParameterNode<'pr>,
    ) {
        let kind = Declaration::Parameter(ParameterKind::RequiredKeyword);
        self.declare(node.name().as_slice(), kind, range(&node.location()));
    }

    fn visit_optional_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::OptionalKeywordParameterNode<'pr>,
    ) {
        let kind = Declaration::Parameter(ParameterKind::OptionalKeyword);
        self.declare(node.name().as_slice(), kind, range(&node.name_loc()));
        self.parameter_default(node.value());
    }

    fn visit_keyword_rest_parameter_node(
        &mut self,
        node: &ruby_prism::KeywordRestParameterNode<'pr>,
    ) {
        if let Some(name) = node.name() {
            let kind = Declaration::Parameter(ParameterKind::KeywordRest);
            self.declare(name.as_slice(), kind, range(&node.location()));
        }
    }

    fn visit_block_parameter_node(&mut self, node: &ruby_prism::BlockParameterNode<'pr>) {
        if let Some(name) = node.name() {
            let kind = Declaration::Parameter(ParameterKind::Block);
            self.declare(name.as_slice(), kind, range(&node.location()));
        }
    }

    fn visit_block_local_variable_node(&mut self, node: &ruby_prism::BlockLocalVariableNode<'pr>) {
        let kind = Declaration::Parameter(ParameterKind::BlockLocal);
        self.declare(node.name().as_slice(), kind, range(&node.location()));
    }

    // --- assignments ------------------------------------------------------

    fn visit_local_variable_write_node(&mut self, node: &LocalVariableWriteNode<'pr>) {
        let name = node.name().as_slice();
        let name_range = range(&node.name_loc());
        self.declare_local(
            name,
            node.depth(),
            Declaration::Assignment,
            name_range.clone(),
        );
        // The right-hand side runs first: `a = a + 1` reads the old value.
        self.visit(&node.value());
        self.assign(name, node.as_node(), AssignmentKind::Write, name_range);
    }

    fn visit_local_variable_target_node(&mut self, node: &LocalVariableTargetNode<'pr>) {
        if self.pattern_depth > 0 {
            let at = range(&node.location());
            let name = node.name().as_slice();
            self.declare_local(name, node.depth(), Declaration::PatternMatch, at);
            let variable = self.find_variable(name).expect("declared pattern binding");
            self.out.variables[variable.0].has_pattern_bindings = true;
            self.mark_captured_if_in_block(variable);
        } else {
            self.assign_target(node, AssignmentKind::Target);
        }
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &LocalVariableOperatorWriteNode<'pr>,
    ) {
        let at = range(&node.name_loc());
        self.operator_write(
            node.name().as_slice(),
            node.depth(),
            node.as_node(),
            at,
            node.value(),
        );
    }

    fn visit_local_variable_or_write_node(&mut self, node: &LocalVariableOrWriteNode<'pr>) {
        let at = range(&node.name_loc());
        self.operator_write(
            node.name().as_slice(),
            node.depth(),
            node.as_node(),
            at,
            node.value(),
        );
    }

    fn visit_local_variable_and_write_node(&mut self, node: &LocalVariableAndWriteNode<'pr>) {
        let at = range(&node.name_loc());
        self.operator_write(
            node.name().as_slice(),
            node.depth(),
            node.as_node(),
            at,
            node.value(),
        );
    }

    fn visit_multi_write_node(&mut self, node: &MultiWriteNode<'pr>) {
        for target in node
            .lefts()
            .iter()
            .chain(node.rest())
            .chain(node.rights().iter())
        {
            self.declare_target(&target);
        }
        // Declaration is not assignment: values still run before target writes.
        self.visit(&node.value());
        self.visit_targets(
            &node.lefts(),
            node.rest(),
            &node.rights(),
            AssignmentKind::Multiple,
        );
    }

    fn visit_match_write_node(&mut self, node: &MatchWriteNode<'pr>) {
        let call = node.call();
        let regexp = call.receiver().map_or_else(
            || range(&node.location()),
            |regexp| range(&regexp.location()),
        );
        let mut names: Vec<&'pr [u8]> = Vec::new();
        for target in node.targets().iter() {
            if let Some(target) = target.as_local_variable_target_node() {
                let name = target.name().as_slice();
                if !names.contains(&name) {
                    self.declare_local(
                        name,
                        target.depth(),
                        Declaration::NamedCapture,
                        regexp.clone(),
                    );
                    names.push(name);
                }
            }
        }
        if let Some(arguments) = call.arguments() {
            self.visit_arguments_node(&arguments);
        }
        self.visit_optional(call.receiver());
        for name in names {
            self.assign(
                name,
                node.as_node(),
                AssignmentKind::NamedCapture,
                regexp.clone(),
            );
        }
    }

    // --- references -------------------------------------------------------

    fn visit_local_variable_read_node(&mut self, node: &LocalVariableReadNode<'pr>) {
        self.read(
            node.name().as_slice(),
            range(&node.location()),
            ReferenceKind::Read,
        );
    }

    fn visit_it_local_variable_read_node(
        &mut self,
        node: &ruby_prism::ItLocalVariableReadNode<'pr>,
    ) {
        self.read(b"it", range(&node.location()), ReferenceKind::Read);
    }

    fn visit_forwarding_super_node(&mut self, node: &ForwardingSuperNode<'pr>) {
        let arguments: Vec<VariableId> = self
            .accessible_variables()
            .into_iter()
            .filter(|&id| {
                let variable = &self.out.variables[id.0];
                variable.is_parameter() && self.out.scopes[variable.scope.0].kind == ScopeKind::Def
            })
            .collect();
        self.reference_all(
            arguments,
            range(&node.location()),
            ReferenceKind::ZeroAritySuper,
        );
        if let Some(block) = node.block() {
            self.visit_block_node(&block);
        }
    }

    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        // Receiver/argument evaluation precedes acquisition. In particular an
        // assignment in &expression is visible to the resulting Binding.
        ruby_prism::visit_call_node(self, node);
        if binding_mutation_call(node) {
            self.mark_binding_mutation_uncertain();
        }
        if matches!(
            node.name().as_slice(),
            b"method"
                | b"public_method"
                | b"singleton_method"
                | b"instance_method"
                | b"public_instance_method"
                | b"alias_method"
        ) {
            let index = usize::from(node.name().as_slice() == b"alias_method");
            self.out.uncertain_binding_alias |= node
                .arguments()
                .and_then(|a| a.arguments().iter().nth(index))
                .is_none_or(|n| static_selector_matches(n, binding_mutator).unwrap_or(true));
        } else if matches!(
            node.name().as_slice(),
            b"send" | b"__send__" | b"public_send"
        ) {
            // Unknown dispatch can install/acquire an alias for use in a
            // different method, not only mutate currently accessible locals.
            self.out.uncertain_binding_alias |= node
                .arguments()
                .and_then(|a| a.arguments().iter().next())
                .is_none_or(|n| static_selector_matches(n, binding_alias_dispatch).unwrap_or(true));
        }
        if binding_acquisition_call(node) {
            if node.name().as_slice() != b"binding"
                || binding_receiver_may_be_kernel(node.receiver())
            {
                self.reference_binding(range(&node.location()));
            } else {
                // A non-Kernel core receiver does not create a liveness read,
                // but unknown/custom dispatch is not a correction-safety proof.
                self.mark_binding_mutation_uncertain();
            }
        }
    }

    fn visit_alias_method_node(&mut self, node: &ruby_prism::AliasMethodNode<'pr>) {
        self.out.uncertain_binding_alias |=
            static_selector_matches(node.old_name(), binding_mutator).unwrap_or(true);
        ruby_prism::visit_alias_method_node(self, node);
    }

    // --- loops ------------------------------------------------------------

    fn visit_while_node(&mut self, node: &WhileNode<'pr>) {
        let post = node.is_begin_modifier();
        let modifier = !post && node.closing_loc().is_none();
        self.visit_loop(
            node.as_node(),
            node.predicate(),
            node.statements(),
            post,
            modifier,
        );
    }

    fn visit_until_node(&mut self, node: &UntilNode<'pr>) {
        let post = node.is_begin_modifier();
        let modifier = !post && node.closing_loc().is_none();
        self.visit_loop(
            node.as_node(),
            node.predicate(),
            node.statements(),
            post,
            modifier,
        );
    }

    fn visit_for_node(&mut self, node: &ForNode<'pr>) {
        self.loops.push(LoopFrame::Other);
        let mark = self.begin_loop();
        // `for item in items`: the collection is evaluated first.
        self.visit(&node.collection());
        self.visit_target(&node.index(), AssignmentKind::For);
        let control = self.new_control();
        self.visit_branch_statements(control, 2, range(&node.location()), node.statements());
        self.end_loop(mark);
        self.loops.pop();
    }

    // --- branches ---------------------------------------------------------

    fn visit_if_node(&mut self, node: &IfNode<'pr>) {
        self.branching_constructs += 1;
        let at = range(&node.location());
        let modifier = node.end_keyword_loc().is_none()
            && node
                .if_keyword_loc()
                .is_some_and(|keyword| keyword.as_slice() == b"if");
        self.visit_predicate(at.clone(), &node.predicate(), modifier);
        let control = self.new_control();
        self.visit_branch_statements(control, 1, at.clone(), node.statements());
        if let Some(subsequent) = node.subsequent() {
            let child = range(&subsequent.location());
            self.in_branch(control, 2, None, at, child, |this| this.visit(&subsequent));
        }
        self.branching_constructs -= 1;
    }

    fn visit_unless_node(&mut self, node: &UnlessNode<'pr>) {
        self.branching_constructs += 1;
        let at = range(&node.location());
        let modifier = node.end_keyword_loc().is_none();
        self.visit_predicate(at.clone(), &node.predicate(), modifier);
        let control = self.new_control();
        // RuboCop's AST puts the `else` body first.
        if let Some(else_clause) = node.else_clause() {
            let child = range(&else_clause.location());
            self.in_branch(control, 1, None, at.clone(), child, |this| {
                this.visit_else_node(&else_clause);
            });
        }
        self.visit_branch_statements(control, 2, at, node.statements());
        self.branching_constructs -= 1;
    }

    fn visit_case_node(&mut self, node: &CaseNode<'pr>) {
        self.branching_constructs += 1;
        let at = range(&node.location());
        self.visit_optional(node.predicate());
        let control = self.new_control();
        for (index, when) in node.conditions().iter().enumerate() {
            let child = range(&when.location());
            self.in_branch(control, 1 + index as u32, None, at.clone(), child, |this| {
                this.visit(&when);
            });
        }
        if let Some(else_clause) = node.else_clause() {
            let child = range(&else_clause.location());
            self.in_branch(control, ELSE, None, at, child, |this| {
                this.visit_else_node(&else_clause);
            });
        }
        self.branching_constructs -= 1;
    }

    fn visit_case_match_node(&mut self, node: &CaseMatchNode<'pr>) {
        self.branching_constructs += 1;
        let at = range(&node.location());
        self.visit_optional(node.predicate());
        let control = self.new_control();
        for (index, clause) in node.conditions().iter().enumerate() {
            let child = range(&clause.location());
            self.in_branch(control, 1 + index as u32, None, at.clone(), child, |this| {
                this.visit(&clause);
            });
        }
        if let Some(else_clause) = node.else_clause() {
            let child = range(&else_clause.location());
            self.in_branch(control, ELSE, None, at, child, |this| {
                this.visit_else_node(&else_clause);
            });
        }
        self.branching_constructs -= 1;
    }

    fn visit_in_node(&mut self, node: &InNode<'pr>) {
        // Prism wraps a guard (`in pattern if guard`) around the pattern;
        // RuboCop's AST has the pattern, then an `if_guard`, neither of
        // them a conditional.
        let pattern = node.pattern();
        match &pattern {
            Node::IfNode { .. } => {
                let guard = pattern.as_if_node().unwrap();
                self.pattern_depth += 1;
                self.visit_statements(guard.statements());
                self.pattern_depth -= 1;
                self.visit(&guard.predicate());
            }
            Node::UnlessNode { .. } => {
                let guard = pattern.as_unless_node().unwrap();
                self.pattern_depth += 1;
                self.visit_statements(guard.statements());
                self.pattern_depth -= 1;
                self.visit(&guard.predicate());
            }
            _ => self.visit_pattern(&pattern),
        }
        self.visit_statements(node.statements());
    }

    fn visit_match_predicate_node(&mut self, node: &MatchPredicateNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    fn visit_match_required_node(&mut self, node: &MatchRequiredNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    fn visit_pinned_expression_node(&mut self, node: &PinnedExpressionNode<'pr>) {
        let depth = std::mem::take(&mut self.pattern_depth);
        self.visit(&node.expression());
        self.pattern_depth = depth;
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        self.visit(&node.left());
        let right = node.right();
        let control = self.new_control();
        let (at, child) = (range(&node.location()), range(&right.location()));
        self.in_branch(control, 1, None, at, child, |this| this.visit(&right));
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        self.visit(&node.left());
        let right = node.right();
        let control = self.new_control();
        let (at, child) = (range(&node.location()), range(&right.location()));
        self.in_branch(control, 1, None, at, child, |this| this.visit(&right));
    }

    fn visit_begin_node(&mut self, node: &BeginNode<'pr>) {
        let Some(ensure) = node.ensure_clause() else {
            self.visit_rescued(node);
            return;
        };
        // `ensure` always runs; the rest may be cut short.
        let at = range(&node.location());
        let ensure_start = ensure.location().start_offset();
        let start = node
            .statements()
            .map(|statements| statements.location().start_offset())
            .or_else(|| {
                node.rescue_clause()
                    .map(|rescue| rescue.location().start_offset())
            })
            .unwrap_or(ensure_start);
        let control = self.new_control();
        self.in_branch(
            control,
            MAIN,
            Some(Handler::Ensure),
            at,
            start..ensure_start,
            |this| this.visit_rescued(node),
        );
        self.visit_ensure_node(&ensure);
    }

    fn visit_rescue_modifier_node(&mut self, node: &RescueModifierNode<'pr>) {
        self.branching_constructs += 1;
        let rescue = node.rescue_expression();
        let mark = contains_retry(&rescue).then(|| self.begin_loop());
        let control = self.new_control();
        let at = range(&node.location());
        let expression = node.expression();
        let child = range(&expression.location());
        self.in_branch(
            control,
            MAIN,
            Some(Handler::Rescue),
            at.clone(),
            child,
            |this| {
                this.visit(&expression);
            },
        );
        let child = range(&rescue.location());
        self.in_branch(control, 1, Some(Handler::Rescue), at, child, |this| {
            this.visit(&rescue);
        });
        if let Some(mark) = mark {
            self.end_loop(mark);
        }
        self.branching_constructs -= 1;
    }

    // Operator assignments to anything else: the right-hand side is a
    // branch too.
    fn visit_instance_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOperatorWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_class_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOperatorWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_global_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOperatorWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_constant_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantOperatorWriteNode<'pr>,
    ) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOperatorWriteNode<'pr>,
    ) {
        self.visit_constant_path_node(&node.target());
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'pr>,
    ) {
        self.visit_constant_path_node(&node.target());
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'pr>,
    ) {
        self.visit_constant_path_node(&node.target());
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        self.visit_compound_getter(
            &node.as_node(),
            node.receiver(),
            node.read_name().as_slice(),
        );
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        self.visit_compound_getter(
            &node.as_node(),
            node.receiver(),
            node.read_name().as_slice(),
        );
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        self.visit_compound_getter(
            &node.as_node(),
            node.receiver(),
            node.read_name().as_slice(),
        );
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        self.visit_optional(node.receiver());
        if let Some(arguments) = node.arguments() {
            self.visit_arguments_node(&arguments);
        }
        if let Some(block) = node.block() {
            self.visit_block_argument_node(&block);
        }
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.visit_optional(node.receiver());
        if let Some(arguments) = node.arguments() {
            self.visit_arguments_node(&arguments);
        }
        if let Some(block) = node.block() {
            self.visit_block_argument_node(&block);
        }
        self.visit_operator_value(&node.as_node(), node.value());
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.visit_optional(node.receiver());
        if let Some(arguments) = node.arguments() {
            self.visit_arguments_node(&arguments);
        }
        if let Some(block) = node.block() {
            self.visit_block_argument_node(&block);
        }
        self.visit_operator_value(&node.as_node(), node.value());
    }
}

impl<'pr> Analyzer<'pr> {
    fn visit_compound_getter(
        &mut self,
        node: &Node<'pr>,
        receiver: Option<Node<'pr>>,
        name: &[u8],
    ) {
        if let Some(receiver) = &receiver {
            self.visit(receiver);
        }
        if binding_mutator(name) {
            self.mark_binding_mutation_uncertain();
        }
        // The zero-argument getter runs before the conditional/operator RHS
        // (and before any setter). Receiver effects already happened.
        if name == b"binding" && binding_receiver_may_be_kernel(receiver) {
            self.reference_binding(range(&node.location()));
        }
    }

    /// The right-hand side of an operator assignment, as a branch.
    fn visit_operator_value(&mut self, node: &Node<'pr>, value: Node<'pr>) {
        let control = self.new_control();
        let (at, child) = (range(&node.location()), range(&value.location()));
        self.in_branch(control, 1, None, at, child, |this| this.visit(&value));
    }
}

/// A possible core Binding acquisition, not merely a call with that spelling.
/// Kernel.binding accepts any valid block (which it ignores), but no positional
/// arguments. Unknown splats/forwarding may be empty. Unlike optimized send /
/// __send__, public_send and Method#call are NOT captures: CRuby rejects their
/// non-Ruby caller. Its acquisition is handled as
/// correction uncertainty separately, never as a liveness reference. Receiver
/// ownership is classified separately: rejected ownership must retain correction
/// uncertainty, not silently prove locals unchanged.
fn binding_acquisition_call(call: &CallNode<'_>) -> bool {
    if !matches!(call.name().as_slice(), b"binding" | b"send" | b"__send__") {
        return false;
    }
    let args: Vec<_> = call
        .arguments()
        .map(|a| a.arguments().iter().collect())
        .unwrap_or_default();
    let args = match call.name().as_slice() {
        b"binding" => args,
        b"send" | b"__send__" => {
            let mut args = args.into_iter();
            let Some(first) = args.next() else {
                return false;
            };
            if static_selector_matches(first, |s| s == b"binding") != Some(true) {
                return false;
            }
            args.collect()
        }
        _ => return false,
    };
    if !args.into_iter().all(binding_argument_may_be_empty) {
        return false;
    }
    call.block()
        .and_then(|b| b.as_block_argument_node())
        .and_then(|b| b.expression())
        .is_none_or(|n| {
            !matches!(
                binding_expression(n),
                Node::IntegerNode { .. }
                    | Node::FloatNode { .. }
                    | Node::RationalNode { .. }
                    | Node::ImaginaryNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
                    | Node::StringNode { .. }
                    | Node::ArrayNode { .. }
                    | Node::HashNode { .. }
            )
        })
}

/// Unknown receivers can be Kernel aliases (constants, locals or getter
/// results). Syntactic non-ownership is not proof of non-exposure. Only exclude
/// definite non-Kernel values under the same core-Ruby assumptions as bare
/// binding itself; no alias/value-flow graph or subtree scan is needed.
fn binding_receiver_may_be_kernel(receiver: Option<Node<'_>>) -> bool {
    receiver.is_none_or(|node| {
        let node = binding_expression(node);
        if matches!(
            node,
            Node::NilNode { .. }
                | Node::TrueNode { .. }
                | Node::FalseNode { .. }
                | Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::RationalNode { .. }
                | Node::ImaginaryNode { .. }
                | Node::StringNode { .. }
                | Node::SymbolNode { .. }
                | Node::ArrayNode { .. }
                | Node::HashNode { .. }
        ) {
            return false;
        }
        !node.as_call_node().is_some_and(|call| {
            call.name().as_slice() == b"new"
                && call.receiver().is_some_and(|receiver| {
                    let receiver = binding_expression(receiver);
                    super::ast::is_global_const(&receiver, &["Object"])
                        || super::ast::is_global_const(&receiver, &["BasicObject"])
                })
        })
    })
}

fn binding_argument_may_be_empty(node: Node<'_>) -> bool {
    if node.as_forwarding_arguments_node().is_some() {
        return true;
    }
    if let Some(splat) = node.as_splat_node() {
        return splat.expression().is_none_or(|n| {
            let n = binding_expression(n);
            n.as_array_node()
                .is_none_or(|a| a.elements().iter().all(|e| e.as_splat_node().is_some()))
        });
    }
    if let Some(keywords) = node.as_keyword_hash_node() {
        return keywords.elements().iter().all(|e| {
            e.as_assoc_splat_node().is_some_and(|s| {
                s.value().is_none_or(|n| {
                    let n = binding_expression(n);
                    n.as_hash_node().is_none_or(|h| {
                        h.elements()
                            .iter()
                            .all(|e| e.as_assoc_splat_node().is_some())
                    })
                })
            })
        });
    }
    false
}

// Wrapper chains belong to disjoint arguments; no repeated subtree scan.
fn binding_expression(mut node: Node<'_>) -> Node<'_> {
    loop {
        if let Some(body) = node.as_parentheses_node().and_then(|n| n.body()) {
            node = body;
        } else if let Some(statements) = node.as_statements_node()
            && statements.body().len() == 1
        {
            node = statements.body().first().unwrap();
        } else {
            return node;
        }
    }
}

fn binding_mutator(name: &[u8]) -> bool {
    matches!(
        name,
        b"binding"
            | b"local_variable_set"
            | b"eval"
            | b"instance_eval"
            | b"class_eval"
            | b"module_eval"
            | b"send"
            | b"__send__"
            | b"public_send"
            | b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"alias_method"
    )
}

fn binding_alias_dispatch(name: &[u8]) -> bool {
    matches!(
        name,
        b"alias_method"
            | b"method"
            | b"public_method"
            | b"singleton_method"
            | b"instance_method"
            | b"public_instance_method"
            | b"send"
            | b"__send__"
            | b"public_send"
            | b"eval"
            | b"instance_eval"
            | b"class_eval"
            | b"module_eval"
    )
}

fn binding_mutation_call(call: &CallNode<'_>) -> bool {
    match call.name().as_slice() {
        b"local_variable_set" | b"eval" | b"instance_eval" | b"class_eval" | b"module_eval" => true,
        b"send" | b"__send__" | b"public_send" => call
            .arguments()
            .and_then(|a| a.arguments().iter().next())
            .is_none_or(|n| static_selector_matches(n, binding_mutator).unwrap_or(true)),
        _ => false,
    }
}

fn static_selector_matches(node: Node<'_>, predicate: fn(&[u8]) -> bool) -> Option<bool> {
    let node = binding_expression(node);
    node.as_symbol_node()
        .map(|s| predicate(s.unescaped()))
        .or_else(|| node.as_string_node().map(|s| predicate(s.unescaped())))
        .or_else(|| {
            matches!(
                node,
                Node::NilNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
                    | Node::IntegerNode { .. }
                    | Node::FloatNode { .. }
                    | Node::RationalNode { .. }
                    | Node::ImaginaryNode { .. }
            )
            .then_some(false)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shadowed_calls(source: &str) -> Vec<bool> {
        use crate::cops::{
            NodeContext,
            walk::{NodeVisitor, walk},
        };

        struct Calls<'pr> {
            locals: VisibleLocals,
            ancestors: Vec<Node<'pr>>,
            shadowed: Vec<bool>,
        }
        impl<'pr> NodeVisitor<'pr> for Calls<'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                if let Some(call) = node.as_call_node()
                    && call.name().as_slice() == b"foo"
                {
                    let cx = NodeContext {
                        ancestors: &self.ancestors,
                        corrections_requested: false,
                        require_order_reports: &Default::default(),
                    };
                    self.shadowed.push(self.locals.shadows(&call, &cx));
                }
                self.ancestors.push(node);
            }
            fn leave(&mut self) {
                self.ancestors.pop();
            }
        }
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0, "{source}");
        let mut calls = Calls {
            locals: VisibleLocals::of(&parsed.node()),
            ancestors: Vec::new(),
            shadowed: Vec::new(),
        };
        walk(&mut calls, &parsed.node());
        calls.shadowed
    }

    #[test]
    fn visible_locals_own_heredoc_interpolations_structurally() {
        for (open, close) in [
            ("class C", "end"),
            ("module M", "end"),
            ("class << self", "end"),
            ("module M; class C", "end; end"),
            ("module M; end; class M::C", "end"),
            ("class C; foo=0; end; class C", "end"),
        ] {
            let source = format!("{open}; foo=1; X=<<~TXT\n#{{foo()}}\nTXT\n{close}\n");
            assert_eq!(shadowed_calls(&source), [true], "{source}");
            // The inverse must not inherit an unrelated outer local merely
            // because the interpolation is beyond the body's source extent.
            let source = format!("foo=1; {open}; X=<<~TXT\n#{{foo()}}\nTXT\n{close}\n");
            assert_eq!(shadowed_calls(&source), [false], "{source}");
        }
    }

    #[test]
    fn visible_locals_headers_and_receivers_evaluate_in_outer_scope() {
        for header in [
            "class C < choose(<<~TXT)",
            "class choose(<<~TXT)::C",
            "module choose(<<~TXT)::M",
            "class << choose(<<~TXT)",
            "def (choose(<<~TXT)).method",
        ] {
            let source = format!("foo=1; {header}; end\n#{{foo()}}\nTXT\n");
            assert_eq!(shadowed_calls(&source), [true], "{source}");
            let source = format!("{header}; foo=1; end\n#{{foo()}}\nTXT\n");
            assert_eq!(shadowed_calls(&source), [false], "{source}");
            let header = header.replace("choose(<<~TXT)", "foo()");
            let source = format!("foo=1; {header}; end");
            assert_eq!(shadowed_calls(&source), [true], "{source}");
            let source = format!("{header}; foo=1; end");
            assert_eq!(shadowed_calls(&source), [false], "{source}");
        }
        for (source, expected) in [
            (
                "foo=1; def (choose(<<~TXT)).method(foo); end\n#{foo()}\nTXT\n",
                true,
            ),
            (
                "def (choose(<<~TXT)).method(foo); end\n#{foo()}\nTXT\n",
                false,
            ),
        ] {
            assert_eq!(shadowed_calls(source), [expected], "{source}");
        }
    }

    #[test]
    fn visible_locals_parameters_bodies_and_captures_are_inner_scope() {
        for (source, expected) in [
            ("foo=1; def f(x = [foo()]); end", false),
            ("foo=1; def f(x = <<~TXT); end\n#{foo()}\nTXT\n", false),
            ("def f(foo, x = <<~TXT); end\n#{foo()}\nTXT\n", true),
            (
                "class Outer; foo=1; class Inner < choose(<<~TXT); end\n#{foo()}\nTXT\nend",
                true,
            ),
            (
                "foo=1; class Outer; class Inner < choose(<<~TXT); end\n#{foo()}\nTXT\nend",
                false,
            ),
            (
                "class C; foo=1; X=<<~OUTER\n#{<<~INNER}\n#{foo()}\nINNER\nOUTER\nend",
                true,
            ),
            ("foo=1; def f(x = [foo()]); foo=2; end", false),
            ("def f(foo, x = [foo()]); end", true),
            ("def f(foo) = <<~TXT\n#{foo()}\nTXT\n", true),
            ("foo=1; def f = <<~TXT\n#{foo()}\nTXT\n", false),
            ("class C; foo=1; X=<<~TXT; end\n#{foo()}\nTXT\n", true),
            (
                "class C; foo=1; X=<<~TXT; rescue; end\n#{foo()}\nTXT\n",
                true,
            ),
            (
                "class C; foo=1; X=<<~TXT\n#{1.times { foo() }}\nTXT\nend",
                true,
            ),
            ("class C; foo=1; X=<<~TXT\n#{-> { foo() }}\nTXT\nend", true),
            (
                "class C; X=<<~TXT\n#{1.times { |foo| foo() }}\nTXT\nend",
                true,
            ),
            ("class C; X=<<~TXT\n#{->(foo) { foo() }}\nTXT\nend", true),
            (
                "class C; foo=1; X=<<~TXT\n#{def f; foo(); end}\nTXT\nend",
                false,
            ),
        ] {
            assert_eq!(shadowed_calls(source), [expected], "{source}");
        }
    }

    /// Every assignment as `(name, line-free name range text, used)`, in
    /// analysis order.
    fn assignments(source: &str) -> Vec<(String, bool)> {
        let result = ruby_prism::parse(source.as_bytes());
        assert_eq!(result.errors().count(), 0, "fixture must parse: {source:?}");
        let program = result.node();
        let analysis = ScopeAnalysis::of(&program.as_program_node().unwrap());
        (0..analysis.assignments.len())
            .map(|index| {
                let id = AssignmentId(index);
                let assignment = analysis.assignment(id);
                let name = analysis.variable(assignment.variable).name;
                (
                    String::from_utf8_lossy(name).into_owned(),
                    analysis.is_used(id),
                )
            })
            .collect()
    }

    fn used(pairs: &[(&str, bool)]) -> Vec<(String, bool)> {
        pairs
            .iter()
            .map(|&(name, used)| (name.to_string(), used))
            .collect()
    }

    #[test]
    #[should_panic(expected = "AST exceeds maximum depth of 64")]
    fn standalone_analysis_rejects_deep_definitions_without_stack_overflow() {
        let mut source = String::new();
        for i in 0..100 {
            source.push_str(&format!("def foo{i}(*args)\nbar(*args)\n"));
        }
        source.push_str(&"end\n".repeat(100));
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
    }

    #[test]
    fn scopes_are_listed_innermost_first() {
        let source = "a = 1\ndef m(x)\n  [1].each { |y| y }\nend\nclass C; end\n";
        let result = ruby_prism::parse(source.as_bytes());
        let program = result.node();
        let analysis = ScopeAnalysis::of(&program.as_program_node().unwrap());
        let kinds: Vec<ScopeKind> = analysis
            .scopes_in_exit_order()
            .map(|id| analysis.scope(id).kind)
            .collect();
        assert_eq!(
            kinds,
            [
                ScopeKind::Block,
                ScopeKind::Def,
                ScopeKind::Class,
                ScopeKind::TopLevel
            ]
        );
        let names: Vec<Vec<&[u8]>> = analysis
            .scopes_in_exit_order()
            .map(|id| {
                analysis
                    .scope(id)
                    .variables()
                    .map(|variable| analysis.variable(variable).name)
                    .collect()
            })
            .collect();
        assert_eq!(
            names,
            [vec![b"y".as_slice()], vec![b"x"], vec![], vec![b"a"]]
        );
    }

    #[test]
    fn blocks_see_outer_variables_and_capture_them() {
        let source = "a = 1\nfoo { a = 2 }\nb = 1\ndef m\n  b = 2\nend\n";
        let result = ruby_prism::parse(source.as_bytes());
        let program = result.node();
        let analysis = ScopeAnalysis::of(&program.as_program_node().unwrap());
        let top = analysis.scopes_in_exit_order().last().unwrap();
        let top_variables: Vec<VariableId> = analysis.scope(top).variables().collect();
        assert_eq!(top_variables.len(), 2);
        let a = analysis.variable(top_variables[0]);
        assert_eq!(a.name, b"a");
        assert_eq!(a.assignments.len(), 2, "the block assigns the outer `a`");
        assert!(a.captured_by_block);
        let b = analysis.variable(top_variables[1]);
        assert_eq!(b.assignments.len(), 1, "the def has its own `b`");
        assert!(!b.captured_by_block);
    }

    #[test]
    fn parallel_rhs_closures_share_target_identity_without_prior_declarations() {
        for source in [
            "a, b = [-> { b }, 2]",
            "a, (b, *c), d = [-> { b }, [2], 3]",
            "a, *b = [-> { b }, 2]",
            "a, *c, b = [-> { b }, 2]",
            "a, b = [-> { -> { b } }, 2]",
            "a, b = [proc { b }, 2]",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let target = analysis
                .assignments
                .iter()
                .find(|a| {
                    a.kind == AssignmentKind::Multiple && analysis.variable(a.variable).name == b"b"
                })
                .unwrap();
            let variable = analysis.variable(target.variable);
            assert!(variable.captured_by_block, "{source}");
            assert_eq!(variable.references.len(), 1, "{source}");
            assert_eq!(
                analysis.reference(variable.references[0]).variable,
                target.variable
            );
            assert_eq!(variable.scope, ScopeId(0), "{source}");
            assert_eq!(variable.declaration_range, target.name_range);
            assert!(
                !target.referenced,
                "the deferred read precedes the value write"
            );
            assert!(analysis.is_used(variable.assignments[0]), "{source}");
        }
    }

    #[test]
    fn parallel_predeclarations_do_not_capture_earlier_lhs_receiver_locals() {
        for source in [
            "obj[-> { b = 1; b }], b = [0, 2]",
            "obj[-> { b, c = [1, 2]; b }], b = [0, 2]",
            "obj[-> { b ||= 1; b }], b = [0, 2]",
            "obj[-> { /(?<b>.)/ =~ str; b }], b = [0, 2]",
            "obj[-> { [1] => [b]; b }], b = [0, 2]",
            "obj[<<~TXT], b = [0, 2]\n#{-> { b = 1; b }}\nTXT\n",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let variables: Vec<_> = analysis
                .variables
                .iter()
                .filter(|v| v.name == b"b")
                .collect();
            assert_eq!(variables.len(), 2, "{source}");
            let outer = variables.iter().find(|v| v.scope == ScopeId(0)).unwrap();
            let inner = variables.iter().find(|v| v.scope != ScopeId(0)).unwrap();
            assert!(!outer.captured_by_block, "{source}");
            assert!(outer.references.is_empty(), "{source}");
            assert!(!inner.references.is_empty(), "{source}");
        }
    }

    #[test]
    fn parallel_rhs_writes_capture_the_target_instead_of_declaring_block_locals() {
        for (source, count, references) in [
            ("a, b = [-> { b = 3 }, 2]", 2, 0),
            ("_, _ = x, -> { _, _ = y, z }", 4, 0),
            ("a, b = [-> { b ||= 3 }, 2]", 2, 1),
            ("a, b = [-> { b &&= 3 }, 2]", 2, 1),
            ("a, b = [-> { b += 3 }, 2]", 2, 1),
            ("a, b = [-> { [3] => [b] }, 2]", 1, 0),
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let name: &[u8] = if source.starts_with('_') { b"_" } else { b"b" };
            let vars: Vec<_> = analysis
                .variables
                .iter()
                .filter(|v| v.name == name)
                .collect();
            assert_eq!(vars.len(), 1, "{source}");
            let var = vars[0];
            assert_eq!(var.scope, ScopeId(0));
            assert!(var.captured_by_block, "{source}");
            assert_eq!(var.assignments.len(), count, "{source}");
            assert_eq!(var.references.len(), references, "{source}");
            // Lexical declarations are not writes: the RHS's writes are first.
            assert_eq!(
                analysis.assignment(*var.assignments.last().unwrap()).kind,
                AssignmentKind::Multiple
            );
            for &id in &var.assignments {
                assert_eq!(
                    analysis.assignment(id).variable,
                    var.assignments
                        .first()
                        .map(|id| analysis.assignment(*id).variable)
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn parallel_rhs_deferred_headers_receivers_defaults_and_interpolations_keep_bindings() {
        for expression in [
            "->(x = b) { x }",
            "->(x: b) { x }",
            "-> { \"#{b}\" }",
            "-> { <<~TXT }\n#{b}\nTXT\n",
            "def (build(<<~TXT)).m; b = 99; end\n#{b}\nTXT\n",
            "class C < build(<<~TXT); b = 99; end\n#{b}\nTXT\n",
            "class build(<<~TXT)::C; b = 99; end\n#{b}\nTXT\n",
            "module build(<<~TXT)::M; b = 99; end\n#{b}\nTXT\n",
            "class << build(<<~TXT); b = 99; end\n#{b}\nTXT\n",
        ] {
            // The extra lambda makes receiver/header accesses captures too.
            let source = format!("a, b = [-> {{ {expression} }}, 2]");
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let target = analysis
                .assignments
                .iter()
                .find(|a| {
                    a.kind == AssignmentKind::Multiple && analysis.variable(a.variable).name == b"b"
                })
                .unwrap();
            let var = analysis.variable(target.variable);
            assert_eq!(var.scope, ScopeId(0));
            assert!(var.captured_by_block, "{source}");
            assert_eq!(var.references.len(), 1, "{source}");
            assert_eq!(
                analysis.reference(var.references[0]).variable,
                target.variable
            );
            for scope in analysis
                .scopes_in_exit_order()
                .filter(|id| !analysis.scope(*id).kind.is_block() && *id != ScopeId(0))
            {
                for id in analysis.scope(scope).variables() {
                    assert_ne!(
                        id, target.variable,
                        "closed bodies must not steal the header binding: {source}"
                    );
                }
            }
        }
    }

    #[test]
    fn parallel_rhs_shadowing_and_closed_scopes_keep_separate_identities() {
        for expression in [
            "->(b) { b }",
            "proc { |b| b }",
            "proc { |; b| b = 3; b }",
            "def m(b = 3); b; end",
            "class C; b = 3; b; end",
            "module M; b = 3; b; end",
            "class << self; b = 3; b; end",
        ] {
            let source = format!("b = 0; class Container; a, b = [{expression}, 2]; end");
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let vars: Vec<_> = analysis
                .variables
                .iter()
                .filter(|v| v.name == b"b")
                .collect();
            assert_eq!(vars.len(), 3, "{source}");
            assert!(vars.iter().all(|v| !v.captured_by_block), "{source}");
            assert!(vars[0].references.is_empty());
            assert!(vars[1].references.is_empty());
            assert!(!vars[2].references.is_empty());
            assert_ne!(vars[0].scope, vars[1].scope);
            assert_ne!(vars[1].scope, vars[2].scope);
        }
        let parsed = ruby_prism::parse(b"b = 0; proc { a, b = [-> { b }, 2] }");
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        let vars: Vec<_> = analysis
            .variables
            .iter()
            .filter(|v| v.name == b"b")
            .collect();
        assert_eq!(
            vars.len(),
            1,
            "parallel targets in a block retain visible outer locals"
        );
        assert_eq!(vars[0].scope, ScopeId(0));
        assert!(vars[0].captured_by_block);
        assert_eq!(vars[0].assignments.len(), 2);
        assert_eq!(vars[0].references.len(), 1);

        for (open, close, owner) in [
            ("class Container", "end", ScopeKind::Class),
            ("module Container", "end", ScopeKind::Module),
            ("class << self", "end", ScopeKind::SingletonClass),
            ("def f", "end", ScopeKind::Def),
            ("def f; proc {", "}; end", ScopeKind::Block),
        ] {
            let source = format!("b = 0; {open}; a, b = [-> {{ b }}, 2]; {close}");
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let vars: Vec<_> = analysis
                .variables
                .iter()
                .filter(|v| v.name == b"b")
                .collect();
            assert_eq!(vars.len(), 2, "{source}");
            assert_eq!(vars[0].scope, ScopeId(0));
            assert!(!vars[0].captured_by_block);
            assert!(vars[0].references.is_empty());
            assert_eq!(analysis.scope(vars[1].scope).kind, owner);
            assert!(vars[1].captured_by_block, "{source}");
            assert_eq!(vars[1].references.len(), 1);
        }
    }

    #[test]
    fn parallel_declaration_does_not_add_later_write_dependencies() {
        // Matching RuboCop's RHS-first examples; no future write is consumed.
        for (source, expected) in [
            (
                "a = 1; b = 2; a, b = b, a",
                used(&[("a", true), ("b", true), ("a", false), ("b", false)]),
            ),
            ("a, b = a, b", used(&[("a", false), ("b", false)])),
            (
                "a, b = (b = 3), b",
                used(&[("b", true), ("a", false), ("b", false)]),
            ),
            ("a, b = [binding, 2]", used(&[("a", false), ("b", false)])),
            (
                "a, b = [-> { b = 3 }, 2]",
                used(&[("b", false), ("a", false), ("b", false)]),
            ),
        ] {
            assert_eq!(assignments(source), expected, "{source}");
        }
        let source = "a, b = [def (build(<<~TXT)).m; end, 2]\n#{b}\nTXT\n";
        let parsed = ruby_prism::parse(source.as_bytes());
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        let var = analysis.variables.iter().find(|v| v.name == b"b").unwrap();
        assert_eq!(
            var.references.len(),
            1,
            "the eager receiver reads the declared nil local"
        );
        assert!(!var.captured_by_block);
        assert!(!analysis.assignment(var.assignments[0]).referenced);
        assert!(!analysis.is_used(var.assignments[0]));
    }

    #[test]
    fn eager_reads_do_not_keep_later_captured_writes_alive() {
        for source in [
            "a, b = [b, -> { b = 3 }]; p a",
            "a, b = [binding, -> { b = 3 }]; p a",
            "a, b = [b, -> { -> { b = 3 } }]; p a",
            "a, b = [def (build(<<~TXT)).m; end, -> { b = 3 }]; p a\n#{b}\nTXT\n",
            "proc { a, b = [b, -> { b = 3 }]; p a }",
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let var = analysis.variables.iter().find(|v| v.name == b"b").unwrap();
            assert!(var.captured_by_block, "{source}");
            assert_eq!(var.references.len(), 1, "keep the lexical read: {source}");
            assert_eq!(var.assignments.len(), 2, "{source}");
            for &id in &var.assignments {
                assert!(!analysis.is_used(id), "{source}");
            }
        }
        for (source, expected) in [
            ("a = a; proc { a = 3 }", used(&[("a", false), ("a", false)])),
            (
                "a = 0; p a; proc { a = 3 }",
                used(&[("a", true), ("a", false)]),
            ),
            (
                "a, b = [-> { b = 3 }, 2]; p b; proc { b = 4 }",
                used(&[("b", true), ("a", false), ("b", true), ("b", false)]),
            ),
            (
                "a, b = [b, -> { b = 3 }]; p b",
                used(&[("b", true), ("a", false), ("b", true)]),
            ),
            (
                "a, b = [-> { b }, -> { b = 3 }]; p a",
                used(&[("b", true), ("a", true), ("b", true)]),
            ),
            (
                "a, b = [-> { binding }, -> { b = 3 }]; p a",
                used(&[("b", true), ("a", true), ("b", true)]),
            ),
        ] {
            assert_eq!(assignments(source), expected, "{source}");
        }
    }

    #[test]
    fn binding_block_arguments_and_reflective_sends_capture_live_values() {
        for acquisition in [
            "binding",
            "binding()",
            "binding(&nil)",
            "binding(&-> {})",
            "binding { raise 'not yielded' }",
            "self.binding(&nil)",
            "Kernel.binding(&nil)",
            "::Kernel.binding(&nil)",
            "send(:binding)",
            "__send__('binding', &nil)",
            "object.send(:binding)",
            "((Kernel)).binding(&nil)",
            "(self).binding(&nil)",
            "binding(*[])",
            "binding(**{})",
            "send(:binding, **{})",
        ] {
            let source = format!("def f; a = 2; {acquisition}.local_variable_get(:a); end");
            assert_eq!(assignments(&source), used(&[("a", true)]), "{source}");
        }
        for source in [
            "def f(&callback); a = 2; binding(&callback).local_variable_get(:a); end",
            "def f(&); a = 2; binding(&).local_variable_get(:a); end",
            "def f(...); a = 2; binding(...).local_variable_get(:a); end",
        ] {
            // Forwarded positional arguments may be empty; a successful
            // acquisition exposes a even though nonempty calls would raise.
            assert_eq!(assignments(source), used(&[("a", true)]), "{source}");
        }
    }

    #[test]
    fn unresolved_binding_receivers_cannot_prove_locals_unexposed() {
        for acquisition in [
            "K.binding",
            "((::K)).binding(&nil)",
            "Other::Kernel.binding(&-> {})",
            "receiver.binding",
            "receiver.binding(&nil)",
        ] {
            let source = format!("K = Kernel; def f(&block); a = 2; {acquisition}; end");
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            for (i, variable) in analysis.variables.iter().enumerate() {
                assert!(
                    analysis.may_be_dynamically_mutated(VariableId(i)),
                    "{source}"
                );
                assert!(
                    variable
                        .references
                        .iter()
                        .any(|r| { analysis.reference(*r).kind == ReferenceKind::Binding }),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn compound_binding_getters_expose_values_after_receiver_before_rhs() {
        for getter in [
            "self.binding",
            "Kernel.binding",
            "K.binding",
            "receiver.binding",
        ] {
            for operator in ["||=", "&&=", "+="] {
                let source = format!("a = 1; ({getter} {operator} (a = 2))");
                assert_eq!(
                    assignments(&source),
                    used(&[("a", true), ("a", false)]),
                    "{source}"
                );
            }
        }
        let source = "a = 1; ((a = 2; self).binding ||= (a = 3))";
        assert_eq!(
            assignments(source),
            used(&[("a", false), ("a", true), ("a", false)])
        );
        let source = "a = 1; later { self.binding ||= nil }; a = 2";
        let parsed = crate::parser::parse(source.as_bytes());
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        assert!(analysis.variables[0].captured_by_block);
        assert!((0..analysis.assignments.len()).all(|i| analysis.is_used(AssignmentId(i))));
    }

    #[test]
    fn binding_references_respect_ownership_shadowing_and_argument_evaluation() {
        for acquisition in [
            "binding(nil)",
            "binding(1)",
            "binding(&1)",
            "binding(*[1])",
            "binding(key: 1)",
            "Object.new.binding",
            "42.binding",
            "nil.binding",
            "public_send(:binding)",
            "Kernel.public_send(:binding)",
        ] {
            let source = format!("def f; a = 2; {acquisition}; end");
            assert_eq!(assignments(&source), used(&[("a", false)]), "{source}");
        }
        assert_eq!(
            assignments("a = 1; binding(&(a = nil))"),
            used(&[("a", false), ("a", true)])
        );
        assert_eq!(
            assignments("binding = nil; a = 2; p binding"),
            used(&[("binding", true), ("a", false)])
        );
        for getter in ["Object.new.binding", "42.binding", "nil.binding"] {
            let source = format!("a = 1; ({getter} ||= nil)");
            assert_eq!(assignments(&source), used(&[("a", false)]), "{source}");
        }
        let source = "a = 1; items.each { |a| binding(&nil) }";
        let parsed = ruby_prism::parse(source.as_bytes());
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        assert!(analysis.variables[0].references.is_empty());
        assert!(analysis.variables[1].is_referenced());
        let source = "a = 1; later { binding(&nil) }; a = 2";
        let parsed = ruby_prism::parse(source.as_bytes());
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        assert!(analysis.variables[0].captured_by_block);
        assert!((0..analysis.assignments.len()).all(|i| analysis.is_used(AssignmentId(i))));
    }

    #[test]
    fn fixed_34_main_prism_binding_evidence_distinguishes_capture_from_acquisition() {
        for (parameters, expression, capture, uncertain) in [
            ("", "binding", true, true),
            ("", "binding(&nil)", true, true),
            ("", "binding(&-> {})", true, true),
            ("&cb", "binding(&cb)", true, true),
            ("&", "binding(&)", true, true),
            ("...", "binding(...)", true, true),
            ("", "send(:binding)", true, true),
            ("", "method(:binding).call", false, true),
            ("", "Kernel.public_send(:binding)", false, true),
            ("", "binding(nil)", false, false),
            ("", "other.binding", true, true),
            ("", "K.binding(&nil)", true, true),
            ("", "self.binding ||= nil", true, true),
            ("", "Object.new.binding", false, true),
        ] {
            let source = format!("def f({parameters}); a = 2; {expression}; end");
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let scope = analysis
                .scopes_in_exit_order()
                .find(|s| analysis.scope(*s).kind == ScopeKind::Def)
                .unwrap();
            let a = analysis
                .scope(scope)
                .variables()
                .find(|id| analysis.variable(*id).name == b"a")
                .unwrap();
            let refs: Vec<_> = analysis
                .variable(a)
                .references
                .iter()
                .map(|r| analysis.reference(*r))
                .filter(|r| r.kind == ReferenceKind::Binding)
                .collect();
            assert_eq!(refs.len(), usize::from(capture), "{source}");
            if capture {
                assert_eq!(&source[refs[0].range.clone()], expression);
                assert_eq!(refs[0].variable, a);
            }
            assert_eq!(
                analysis.may_be_dynamically_mutated(a),
                uncertain,
                "{source}"
            );
        }
        // The fixed main-Prism grammar owns implicit-it's lexical identity;
        // this is AST evidence, not a claim about the host Ruby version.
        let parsed = crate::parser::parse(b"items.map { it; binding(&nil) }");
        assert_eq!(parsed.errors().count(), 0);
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        assert!(analysis.variables.iter().any(|v| {
            v.name == b"it"
                && v.references
                    .iter()
                    .any(|r| analysis.reference(*r).kind == ReferenceKind::Binding)
        }));
    }

    #[test]
    fn shared_mutation_uncertainty_does_not_manufacture_reads_or_writes() {
        for body in [
            "send(selector)",
            "saved.local_variable_set(:block, replacement)",
            "saved.local_variable_set ||= value",
            "saved.local_variable_set &&= value",
            "saved.local_variable_set += value",
            "saved.eval(code)",
            "Object.new.binding",
            "method(:binding)",
            "Kernel.instance_method(:binding)",
            "Kernel.alias_method(:capture, :binding)",
        ] {
            let source = format!("def f(&block); a = 2; {body}; end");
            let parsed = ruby_prism::parse(source.as_bytes());
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            for (i, variable) in analysis.variables.iter().enumerate() {
                assert!(
                    analysis.may_be_dynamically_mutated(VariableId(i)),
                    "{source}"
                );
                assert!(
                    variable.references.is_empty(),
                    "uncertainty is not a read: {source}"
                );
                assert!(!variable.captured_by_block);
            }
            assert_eq!(analysis.assignments.len(), 1);
            assert!(!analysis.is_used(AssignmentId(0)), "{source}");
        }
        for body in [
            "send(:to_s)",
            "method(:to_s)",
            "send(nil)",
            "method(nil)",
            "binding(nil)",
            "other.to_s",
        ] {
            let source = format!("def f(&block); {body}; end");
            let parsed = ruby_prism::parse(source.as_bytes());
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            assert!(
                !analysis.may_be_dynamically_mutated(VariableId(0)),
                "{source}"
            );
        }
    }

    #[test]
    fn parameter_default_exposure_uses_complete_header_identities_without_early_writes() {
        for source in [
            "def f(value = binding(&nil), &block); g(&block); end",
            "def f(value = -> { binding(&nil) }, &block); g(&block); end",
            "items.each { |value = binding(&nil), &block| g(&block) }",
            "items.each { |value = binding(&nil); block| g(&block) }",
        ] {
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source}");
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            let block = analysis
                .variables
                .iter()
                .find(|v| v.name == b"block")
                .unwrap();
            assert!(
                block
                    .references
                    .iter()
                    .any(|r| analysis.reference(*r).kind == ReferenceKind::Binding),
                "{source}"
            );
            assert!(block.assignments.is_empty());
        }
        assert_eq!(
            assignments("def f(a = (x = 1), b = (x = 2)); p x; end"),
            used(&[("x", false), ("x", true)])
        );
        let source = "def f(&block); items.each { |x = binding(&nil); block| }; g(&block); end";
        let parsed = crate::parser::parse(source.as_bytes());
        let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
        let blocks: Vec<_> = analysis
            .variables
            .iter()
            .filter(|v| v.name == b"block")
            .collect();
        assert_eq!(blocks.len(), 2);
        assert!(
            blocks[0]
                .references
                .iter()
                .all(|r| analysis.reference(*r).kind != ReferenceKind::Binding)
        );
        assert!(
            blocks[1]
                .references
                .iter()
                .any(|r| analysis.reference(*r).kind == ReferenceKind::Binding)
        );
    }

    #[test]
    fn reflective_alias_uncertainty_can_cross_closed_scopes_without_liveness_effects() {
        for setup in [
            "Kernel.send(:alias_method, :capture, selector)",
            "Kernel.send(selector)",
            "Kernel.method(selector)",
        ] {
            let source = format!("{setup}; def f(&block); a = 2; end");
            let parsed = crate::parser::parse(source.as_bytes());
            let analysis = ScopeAnalysis::of(&parsed.node().as_program_node().unwrap());
            for scope in analysis.scopes_in_exit_order() {
                for id in analysis.scope(scope).variables() {
                    assert!(analysis.may_be_dynamically_mutated(id), "{source}");
                    assert!(analysis.variable(id).references.is_empty());
                }
            }
        }
    }

    #[test]
    fn records_declarations_and_reference_kinds() {
        let source =
            "def m(a, b = 1, *c, d:, e: 2, **f, &g)\n  super\nend\nfoo { |x; y| binding }\n";
        let result = ruby_prism::parse(source.as_bytes());
        let program = result.node();
        let analysis = ScopeAnalysis::of(&program.as_program_node().unwrap());
        let kinds: Vec<(String, Declaration, Vec<ReferenceKind>)> = analysis
            .variables
            .iter()
            .map(|variable| {
                (
                    String::from_utf8_lossy(variable.name).into_owned(),
                    variable.declaration,
                    variable
                        .references
                        .iter()
                        .map(|&id| analysis.reference(id).kind)
                        .collect(),
                )
            })
            .collect();
        use ParameterKind::*;
        let super_ref = vec![ReferenceKind::ZeroAritySuper];
        let binding_ref = vec![ReferenceKind::Binding];
        assert_eq!(
            kinds,
            [
                (
                    "a".into(),
                    Declaration::Parameter(Required),
                    super_ref.clone()
                ),
                (
                    "b".into(),
                    Declaration::Parameter(Optional),
                    super_ref.clone()
                ),
                ("c".into(), Declaration::Parameter(Rest), super_ref.clone()),
                (
                    "d".into(),
                    Declaration::Parameter(RequiredKeyword),
                    super_ref.clone()
                ),
                (
                    "e".into(),
                    Declaration::Parameter(OptionalKeyword),
                    super_ref.clone()
                ),
                (
                    "f".into(),
                    Declaration::Parameter(KeywordRest),
                    super_ref.clone()
                ),
                ("g".into(), Declaration::Parameter(Block), super_ref),
                (
                    "x".into(),
                    Declaration::Parameter(Required),
                    binding_ref.clone()
                ),
                ("y".into(), Declaration::Parameter(BlockLocal), binding_ref),
            ]
        );
        assert!(!analysis.reference(ReferenceId(0)).is_explicit());
    }

    #[test]
    fn reassignment_without_reference_is_unused() {
        assert_eq!(
            assignments("a = 1\na = 2\nputs a\n"),
            used(&[("a", false), ("a", true)])
        );
        assert_eq!(
            assignments("a = 1\nputs a\na = 2\n"),
            used(&[("a", true), ("a", false)])
        );
    }

    #[test]
    fn branches_keep_earlier_assignments_alive() {
        assert_eq!(
            assignments("a = 1\nif c\n  a = 2\nend\nputs a\n"),
            used(&[("a", true), ("a", true)])
        );
        assert_eq!(
            assignments("if c\n  a = 1\nelse\n  a = 2\nend\nputs a\n"),
            used(&[("a", true), ("a", true)])
        );
        // Exclusive branches: the `if` body's value never reaches `else`.
        assert_eq!(
            assignments("if c\n  a = 1\nelse\n  a = 2\n  puts a\nend\n"),
            used(&[("a", false), ("a", true)])
        );
    }

    #[test]
    fn loops_make_the_last_assignment_visible_to_the_next_iteration() {
        assert_eq!(
            assignments("a = 0\nwhile a < 3\n  a += 1\nend\n"),
            used(&[("a", true), ("a", true)])
        );
        assert_eq!(
            assignments("while c\n  a = 1\nend\n"),
            used(&[("a", false)])
        );
    }

    #[test]
    fn retry_makes_a_rescue_a_loop() {
        assert_eq!(
            assignments("tries = 0\nbegin\n  x\nrescue\n  tries += 1\n  retry if tries < 3\nend\n"),
            used(&[("tries", true), ("tries", true)])
        );
    }

    #[test]
    fn exceptions_carry_assignments_into_rescue() {
        assert_eq!(
            assignments("begin\n  a = 1\n  x\n  a = 2\nrescue\n  puts a\nend\n"),
            used(&[("a", true), ("a", true)])
        );
    }

    #[test]
    fn pattern_variables_and_named_captures() {
        assert_eq!(assignments("case x\nin [a]\nend\n"), used(&[]));
        assert_eq!(
            assignments("/(?<a>.)(?<b>.)/ =~ s\nputs a\n"),
            used(&[("a", true), ("b", false)])
        );
    }

    #[test]
    fn pattern_bindings_track_writes_without_changing_assignment_candidates() {
        for pattern in ["[7] in [*args]", "[7] => [*args]"] {
            let source =
                format!("def foo(*args); {pattern}; items.each {{ |args| {pattern} }}; end");
            let result = ruby_prism::parse(source.as_bytes());
            assert_eq!(result.errors().count(), 0);
            let analysis = ScopeAnalysis::of(&result.node().as_program_node().unwrap());
            let variables: Vec<_> = analysis
                .variables
                .iter()
                .filter(|v| v.name == b"args")
                .collect();
            assert_eq!(variables.len(), 2);
            assert!(
                variables
                    .iter()
                    .all(|v| v.has_pattern_bindings && v.assignments.is_empty())
            );
            assert!(
                !variables[0].captured_by_block,
                "the block binds its own args"
            );
        }
        let result = ruby_prism::parse(b"def foo(*args); items.each { [7] in [*args] }; end");
        let analysis = ScopeAnalysis::of(&result.node().as_program_node().unwrap());
        let variable = analysis
            .variables
            .iter()
            .find(|v| v.name == b"args")
            .unwrap();
        assert!(variable.has_pattern_bindings && variable.captured_by_block);
        assert!(variable.assignments.is_empty());
    }

    #[test]
    fn assignment_kinds() {
        let source = "a = 1\nb += 1\nc, (d, *e) = x\nfor f in g; end\nbegin\nrescue => h\nend\n/(?<i>.)/ =~ s\n";
        let result = ruby_prism::parse(source.as_bytes());
        let program = result.node();
        let analysis = ScopeAnalysis::of(&program.as_program_node().unwrap());
        let kinds: Vec<AssignmentKind> = analysis.assignments.iter().map(|a| a.kind).collect();
        use AssignmentKind::*;
        assert_eq!(
            kinds,
            [
                Write,
                Operator,
                Multiple,
                Multiple,
                Multiple,
                For,
                Rescue,
                NamedCapture
            ]
        );
    }

    #[test]
    fn variable_like_names_skip_inner_scopes() {
        let source = "foo\nbar(1)\nx.baz\nqux { quux }\ndef m; corge; end\nv = 1\n";
        let result = ruby_prism::parse(source.as_bytes());
        let program = result.node();
        let analysis = ScopeAnalysis::of(&program.as_program_node().unwrap());
        let top = analysis.scopes_in_exit_order().last().unwrap();
        assert_eq!(
            analysis.variable_like_names(top),
            [b"foo".as_slice(), b"x", b"qux", b"v"]
        );
    }
}
