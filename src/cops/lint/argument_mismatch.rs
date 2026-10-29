//! `Lint/ArgumentMismatch`: positional arity of statically named singleton methods.
//! Requires an explicitly supplied internal declaration index. Production inputs
//! have no assigned index (pinned UseProjectIndex=false), even when forced.
//! No filesystem discovery, runtime type inference, or source correction.

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

pub struct ArgumentMismatch;
impl Cop for ArgumentMismatch {
    fn name(&self) -> &'static str {
        "Lint/ArgumentMismatch"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Selection/config/directives cannot manufacture the data prerequisite.
        // Normal production deliberately does not self-index the supplied file.
        if node.as_program_node().is_some() {
            self.check_with_index(None, file, out);
        }
    }
}

impl ArgumentMismatch {
    /// Internal supplied-index seam, independent of the raw configuration flag.
    /// The real matcher remains compiled; only explicitly received indexed input
    /// can reach it. No public activation or project discovery is provided.
    fn check_with_index(
        &self,
        index: Option<Index<'_>>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(index) = index else { return };
        let mut completeness = HashMap::new();
        let mut signatures = HashMap::new();
        for (call, nesting) in &index.calls {
            let Some(receiver) = call.receiver().and_then(|n| reference(&n, nesting)) else {
                continue;
            };
            if call.name().as_slice() == b"new" {
                continue;
            }
            let Some(owner) = index.resolve(&receiver) else {
                continue;
            };
            if !*completeness
                .entry(owner.clone())
                .or_insert_with(|| index.complete(&owner))
            {
                continue;
            }
            let name = String::from_utf8_lossy(call.name().as_slice());
            let shape = signatures
                .entry((owner.clone(), name.to_string()))
                .or_insert_with(|| {
                    index
                        .lookup(&owner, true, &name, &mut HashSet::new())
                        .flatten()
                });
            let Some(shape) = shape else { continue };
            let Some(given) = positional_count(call, shape.keywords) else {
                continue;
            };
            if given >= shape.min && shape.max.is_none_or(|max| given <= max) {
                continue;
            }
            let expected = match shape.max {
                None => format!("{}+", shape.min),
                Some(max) if max == shape.min => max.to_string(),
                Some(max) => format!("{}..{max}", shape.min),
            };
            let Some(loc) = call.message_loc() else {
                continue;
            };
            let (line, col) = file.line_col(loc.start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                correction: None,
                line,
                col,
                end_line: file.line_of(loc.end_offset().saturating_sub(1)),
                message: format!(
                    "Wrong number of arguments to `{name}` (given {given}, expected {expected})."
                ),
            });
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Shape {
    pub(super) min: usize,
    pub(super) max: Option<usize>,
    pub(super) keywords: bool,
}
fn shape(node: &ruby_prism::DefNode<'_>) -> Option<Shape> {
    let Some(params) = node.parameters() else {
        return Some(Shape {
            min: 0,
            max: Some(0),
            keywords: false,
        });
    };
    if params
        .keyword_rest()
        .is_some_and(|n| n.as_forwarding_parameter_node().is_some())
    {
        return None;
    }
    let min = params.requireds().len() + params.posts().len();
    Some(Shape {
        min,
        max: if params.rest().is_some() {
            None
        } else {
            Some(min + params.optionals().len())
        },
        keywords: !params.keywords().is_empty()
            || params
                .keyword_rest()
                .is_some_and(|n| n.as_keyword_rest_parameter_node().is_some()),
    })
}
fn positional_count(call: &CallNode<'_>, keywords: bool) -> Option<usize> {
    positional_arguments(call.arguments(), keywords)
}
pub(super) fn positional_arguments(
    arguments: Option<ruby_prism::ArgumentsNode<'_>>,
    keywords: bool,
) -> Option<usize> {
    let args: Vec<_> = arguments
        .map(|a| a.arguments().iter().collect())
        .unwrap_or_default();
    if args.iter().any(|n| {
        matches!(
            n,
            Node::SplatNode { .. } | Node::ForwardingArgumentsNode { .. }
        )
    }) {
        return None;
    }
    if let Some(hash) = args.last().and_then(|n| n.as_keyword_hash_node()) {
        if hash
            .elements()
            .iter()
            .any(|n| n.as_assoc_splat_node().is_some())
        {
            return None;
        }
        return Some(args.len() - usize::from(keywords));
    }
    Some(args.len())
}

fn literal_name(node: &Node<'_>) -> Option<String> {
    if let Some(s) = node.as_symbol_node() {
        Some(String::from_utf8_lossy(s.unescaped()).into())
    } else {
        node.as_string_node()
            .map(|s| String::from_utf8_lossy(s.unescaped()).into())
    }
}
fn constant_write_name(node: &Node<'_>) -> Option<String> {
    let name = match node {
        Node::ConstantWriteNode { .. } => node.as_constant_write_node()?.name(),
        Node::ConstantOrWriteNode { .. } => node.as_constant_or_write_node()?.name(),
        Node::ConstantAndWriteNode { .. } => node.as_constant_and_write_node()?.name(),
        Node::ConstantOperatorWriteNode { .. } => node.as_constant_operator_write_node()?.name(),
        Node::ConstantTargetNode { .. } => node.as_constant_target_node()?.name(),
        _ => return None,
    };
    Some(String::from_utf8_lossy(name.as_slice()).into())
}
fn constant_path_write<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    Some(match node {
        Node::ConstantPathWriteNode { .. } => {
            node.as_constant_path_write_node()?.target().as_node()
        }
        Node::ConstantPathOrWriteNode { .. } => {
            node.as_constant_path_or_write_node()?.target().as_node()
        }
        Node::ConstantPathAndWriteNode { .. } => {
            node.as_constant_path_and_write_node()?.target().as_node()
        }
        Node::ConstantPathOperatorWriteNode { .. } => node
            .as_constant_path_operator_write_node()?
            .target()
            .as_node(),
        _ => return None,
    })
}

#[derive(Clone)]
struct Reference {
    name: String,
    absolute: bool,
    nesting: Vec<String>,
}
fn reference(node: &Node<'_>, nesting: &[String]) -> Option<Reference> {
    // Unlike const_name's display helper, reject dynamic constant paths.
    if let Some(path) = node.as_constant_path_node() {
        if let Some(parent) = path.parent() {
            reference(&parent, nesting)?;
        }
    } else if node.as_constant_read_node().is_none() {
        return None;
    }
    let name = ast::const_name(node)?;
    Some(Reference {
        absolute: node.location().as_slice().starts_with(b"::"),
        name,
        nesting: nesting.to_vec(),
    })
}
#[derive(Default)]
struct Namespace {
    parents: Vec<Reference>,
    module: bool,
    methods: HashMap<(bool, String), Vec<Option<Shape>>>,
    hidden: HashSet<(bool, String)>,
    mixins: Vec<(Reference, bool, bool)>,
}
type SuperShapes = HashMap<(String, String), Option<(Shape, String)>>;

#[derive(Default)]
pub(super) struct Index<'pr> {
    namespaces: HashMap<String, Namespace>,
    constants: HashSet<String>,
    declared: HashSet<String>,
    seeding: bool,
    module_functions: HashSet<usize>,
    stack: Vec<Node<'pr>>,
    scopes: HashMap<usize, String>,
    class_declarations: HashMap<usize, usize>,
    super_shapes: RefCell<SuperShapes>,
    // Factory blocks change method ownership, not lexical constant nesting.
    // None is an anonymous namespace, which must not leak into an outer owner.
    block_owners: HashMap<usize, Option<String>>,
    calls: Vec<(CallNode<'pr>, Vec<String>)>,
    methods: Vec<(Reference, bool, String, Option<Shape>, bool)>,
    visibility: HashMap<usize, bool>,
    visibility_changes: Vec<(Reference, bool, String, bool)>,
}
impl<'pr> Index<'pr> {
    // Internal source projection adapter, not a default index provider. Kept
    // compiled alongside the matcher; private tests explicitly supply its result.
    #[cfg_attr(not(test), expect(dead_code, reason = "no production index provider"))]
    pub(super) fn supplied(node: &Node<'pr>) -> Self {
        let mut index = Self::new();
        walk(&mut index, node);
        index.seeding = false;
        walk(&mut index, node);
        index.finish();
        index
    }

    fn new() -> Self {
        let mut index = Self {
            seeding: true,
            ..Self::default()
        };
        // These are Rubydex's five built-in declarations, not a runtime type
        // table. No methods from gems or the standard library are invented.
        for name in ["Object", "BasicObject", "Class", "Module", "Kernel"] {
            index.namespaces.insert(
                name.into(),
                Namespace {
                    module: name == "Kernel",
                    ..Namespace::default()
                },
            );
        }
        for (name, parent) in [
            ("Object", "BasicObject"),
            ("Class", "Module"),
            ("Module", "Object"),
        ] {
            index
                .namespaces
                .get_mut(name)
                .unwrap()
                .parents
                .push(Reference {
                    name: parent.into(),
                    absolute: true,
                    nesting: vec![],
                });
        }
        index.declared.extend(index.namespaces.keys().cloned());
        index.namespaces.get_mut("Object").unwrap().mixins.push((
            Reference {
                name: "Kernel".into(),
                absolute: true,
                nesting: vec![],
            },
            false,
            false,
        ));
        index
    }
    fn exists(&self, name: &str) -> bool {
        self.namespaces.contains_key(name) || self.constants.contains(name)
    }
    fn resolve_direct(&self, r: &Reference) -> Option<String> {
        if !r.absolute {
            for scope in r.nesting.iter().rev() {
                let first = format!("{scope}::{}", r.name.split("::").next()?);
                if self.exists(&first) {
                    let full = format!("{scope}::{}", r.name);
                    return self.namespaces.contains_key(&full).then_some(full);
                }
            }
        }
        self.namespaces
            .contains_key(&r.name)
            .then(|| r.name.clone())
    }
    fn constant_member(&self, owner: &str, name: &str) -> Option<String> {
        let mut pending = vec![owner.to_string()];
        let mut seen = HashSet::new();
        while let Some(scope) = pending.pop() {
            if !seen.insert(scope.clone()) {
                continue;
            }
            let member = format!("{scope}::{name}");
            if self.exists(&member) {
                return Some(member);
            }
            let Some(ns) = self.namespaces.get(&scope) else {
                continue;
            };
            for r in ns.parents.iter().rev().chain(
                ns.mixins
                    .iter()
                    .filter(|(_, side, _)| !side)
                    .map(|(r, _, _)| r),
            ) {
                if let Some(parent) = self.resolve_direct(r) {
                    pending.push(parent);
                }
            }
        }
        None
    }
    fn resolve(&self, r: &Reference) -> Option<String> {
        let mut parts = r.name.split("::");
        let first = parts.next()?;
        let mut current = None;
        if !r.absolute {
            // Ruby searches every lexical declaration before the innermost
            // scope's ancestors. An inherited constant must not hide an outer
            // lexical constant (including a non-namespace shadow).
            for scope in r.nesting.iter().rev() {
                let member = format!("{scope}::{first}");
                if self.exists(&member) {
                    current = Some(member);
                    break;
                }
            }
            if current.is_none() {
                current = r
                    .nesting
                    .last()
                    .and_then(|scope| self.constant_member(scope, first));
            }
        }
        let mut current = current.unwrap_or_else(|| first.into());
        for segment in parts {
            current = self.constant_member(&current, segment)?;
        }
        self.namespaces.contains_key(&current).then_some(current)
    }
    fn complete(&self, owner: &str) -> bool {
        self.complete_ancestry(owner, false)
    }
    fn complete_ancestry(&self, owner: &str, ignore_extend: bool) -> bool {
        let mut pending = vec![owner.to_string()];
        let mut seen = HashSet::new();
        while let Some(owner) = pending.pop() {
            if !seen.insert(owner.clone()) {
                continue;
            }
            let Some(ns) = self.namespaces.get(&owner) else {
                return false;
            };
            for r in ns.parents.iter().chain(
                ns.mixins
                    .iter()
                    .filter(|(_, side, _)| !ignore_extend || !side)
                    .map(|(r, _, _)| r),
            ) {
                let Some(parent) = self.resolve(r) else {
                    return false;
                };
                pending.push(parent);
            }
        }
        true
    }
    fn finish(&mut self) {
        self.namespaces
            .retain(|name, _| self.declared.contains(name));
        for (name, ns) in &mut self.namespaces {
            if !ns.module && ns.parents.is_empty() && name != "BasicObject" {
                ns.parents.push(Reference {
                    name: "Object".into(),
                    absolute: true,
                    nesting: vec![],
                });
            }
        }
        for (r, singleton, name, shape, public) in std::mem::take(&mut self.methods) {
            let Some(owner) = self.resolve(&r) else {
                continue;
            };
            let ns = self.namespaces.get_mut(&owner).unwrap();
            // IndexedMethodArity ignores forwarding signatures before checking
            // whether the remaining signatures agree.
            if let Some(shape) = shape {
                ns.methods
                    .entry((singleton, name.clone()))
                    .or_default()
                    .push(Some(shape));
            } else {
                ns.methods.entry((singleton, name.clone())).or_default();
            }
            if !public {
                ns.hidden.insert((singleton, name));
            } else {
                ns.hidden.remove(&(singleton, name));
            }
        }
        for (r, singleton, name, public) in std::mem::take(&mut self.visibility_changes) {
            let Some(owner) = self.resolve(&r) else {
                continue;
            };
            let hidden = &mut self.namespaces.get_mut(&owner).unwrap().hidden;
            if public {
                hidden.remove(&(singleton, name));
            } else {
                hidden.insert((singleton, name));
            }
        }
    }
    // Outer Option distinguishes a missing member from a present but private,
    // forwarding-only or ambiguous member. Such a member must not fall through
    // to an inherited signature.
    fn lookup(
        &self,
        owner: &str,
        singleton: bool,
        name: &str,
        seen: &mut HashSet<(String, bool)>,
    ) -> Option<Option<Shape>> {
        // Explicit work items avoid Rust recursion on a flat source containing
        // an arbitrarily long inheritance chain. `member` separates prepend
        // lookup from the owner and its includes/superclass.
        let mut pending = vec![(owner.to_string(), singleton, false)];
        while let Some((owner, singleton, member)) = pending.pop() {
            let ns = self.namespaces.get(&owner)?;
            if member {
                let key = (singleton, name.to_string());
                if ns.hidden.contains(&key) {
                    return Some(None);
                }
                if let Some(shapes) = ns.methods.get(&key) {
                    let shape = shapes.first().copied().flatten();
                    return Some(shape.filter(|first| shapes.iter().all(|s| *s == Some(*first))));
                }
                continue;
            }
            if !seen.insert((owner.clone(), singleton)) {
                continue;
            }
            // Singleton ancestry terminates in Class/Module's instance side.
            if singleton && (owner == "BasicObject" || ns.module) {
                pending.push((
                    if ns.module { "Module" } else { "Class" }.into(),
                    false,
                    false,
                ));
            }
            for parent in ns.parents.iter().rev() {
                pending.push((self.resolve(parent)?, singleton, false));
            }
            for (r, side, prepend) in &ns.mixins {
                if *side == singleton && !prepend {
                    let resolved = self.resolve(r)?;
                    if self.namespaces.get(&resolved)?.module {
                        pending.push((resolved, false, false));
                    }
                }
            }
            pending.push((owner, singleton, true));
            for (r, side, prepend) in &ns.mixins {
                if *side == singleton && *prepend {
                    let resolved = self.resolve(r)?;
                    if self.namespaces.get(&resolved)?.module {
                        pending.push((resolved, false, false));
                    }
                }
            }
        }
        None
    }
    /// Instance-side MRO after the anchored class, ignoring its prepends.
    /// Private/protected methods are valid super targets (unlike explicit calls).
    pub(super) fn super_shape(
        &self,
        class: &Node<'_>,
        lexical_name: &str,
        method: &str,
    ) -> Option<(Shape, String)> {
        let offset = class.location().start_offset();
        let declaration = self.class_declarations.get(&offset)?;
        // The private adapter indexes one supplied parse. Anchor to that actual
        // source, not a coincidentally equal offset/name in an unrelated file.
        if *declaration != class.location().as_slice().as_ptr() as usize
            || self.scopes.get(&offset)? != lexical_name
        {
            return None;
        }
        let key = (lexical_name.to_string(), method.to_string());
        if let Some(shape) = self.super_shapes.borrow().get(&key) {
            return shape.clone();
        }
        let shape = self.resolved_super_shape(lexical_name, method);
        self.super_shapes.borrow_mut().insert(key, shape.clone());
        shape
    }
    fn resolved_super_shape(&self, lexical_name: &str, method: &str) -> Option<(Shape, String)> {
        if !self.complete_ancestry(lexical_name, true) {
            return None;
        }
        let chain = self.instance_chain(lexical_name)?;
        let position = chain.iter().position(|owner| *owner == lexical_name)?;
        for owner in &chain[position + 1..] {
            if matches!(*owner, "Object" | "Kernel" | "BasicObject") {
                return None;
            }
            if let Some(shapes) = self
                .namespaces
                .get(*owner)?
                .methods
                .get(&(false, method.to_string()))
            {
                let first = shapes.first().copied().flatten()?;
                return shapes
                    .iter()
                    .all(|s| *s == Some(first))
                    .then_some((first, owner.to_string()));
            }
        }
        None
    }
    /// Rubydex's instance linearization: includes deduplicate against the
    /// superclass chain, prepends only against prepends. No recursive Rust
    /// calls; chains borrow interned namespace names rather than cloning them.
    fn instance_chain<'a>(&'a self, owner: &'a str) -> Option<Vec<&'a str>> {
        let resolve = |r: &Reference| {
            let name = self.resolve(r)?;
            Some(self.namespaces.get_key_value(&name)?.0.as_str())
        };
        let mut reachable = HashSet::new();
        let mut pending = vec![owner];
        while let Some(name) = pending.pop() {
            if !reachable.insert(name) {
                continue;
            }
            let ns = self.namespaces.get(name)?;
            for r in ns.parents.iter().take(1).chain(
                ns.mixins
                    .iter()
                    .filter(|(_, side, _)| !side)
                    .map(|(r, _, _)| r),
            ) {
                pending.push(resolve(r)?);
            }
        }
        // Rubydex resolves declarations in source order. This matters for its
        // conservative cyclic-chain approximation (class => Object, module =>
        // empty), and must not depend on HashMap iteration or the first call.
        let mut roots: Vec<_> = self
            .scopes
            .iter()
            .filter(|(_, name)| reachable.contains(name.as_str()))
            .collect();
        roots.sort_by_key(|(offset, _)| **offset);
        let mut pending = vec![(owner, false)];
        pending.extend(
            roots
                .into_iter()
                .rev()
                .map(|(_, name)| (name.as_str(), false)),
        );
        let mut chains: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut active = HashSet::new();
        while let Some((owner, leaving)) = pending.pop() {
            let ns = self.namespaces.get(owner)?;
            if leaving {
                let parents = match ns.parents.first() {
                    Some(r) => Some(resolve(r)?),
                    None => None,
                };
                let parent_chain = parents
                    .and_then(|p| chains.get(p))
                    .cloned()
                    .unwrap_or_default();
                let mut prepends = Vec::new();
                let mut includes = Vec::new();
                for (r, singleton, prepend) in &ns.mixins {
                    if *singleton {
                        continue;
                    }
                    let ids = chains.get(resolve(r)?)?;
                    if *prepend {
                        if ids.iter().any(|id| !prepends.contains(id)) {
                            prepends.retain(|id| !ids.contains(id));
                            prepends.splice(0..0, ids.iter().copied());
                        }
                    } else {
                        let ids: Vec<_> = ids
                            .iter()
                            .copied()
                            .filter(|id| {
                                !prepends.contains(id)
                                    && !includes.contains(id)
                                    && !parent_chain.contains(id)
                            })
                            .collect();
                        includes.splice(0..0, ids);
                    }
                }
                prepends.push(owner);
                prepends.extend(includes);
                prepends.extend(parent_chain);
                chains.insert(owner, prepends);
                active.remove(owner);
                continue;
            }
            if chains.contains_key(owner) {
                continue;
            }
            if !active.insert(owner) {
                chains.insert(owner, if ns.module { vec![] } else { vec!["Object"] });
                continue;
            }
            pending.push((owner, true));
            for r in ns.parents.iter().take(1).chain(
                ns.mixins
                    .iter()
                    .filter(|(_, side, _)| !side)
                    .map(|(r, _, _)| r),
            ) {
                pending.push((resolve(r)?, false));
            }
        }
        chains.remove(owner)
    }
    fn owner_boundary(&self, node: &Node<'_>) -> bool {
        node.as_class_node().is_some()
            || node.as_module_node().is_some()
            || self
                .block_owners
                .contains_key(&node.location().start_offset())
    }
    fn top_level(&self) -> bool {
        !self.stack.iter().any(|n| {
            self.owner_boundary(n)
                || n.as_singleton_class_node().is_some()
                || n.as_def_node().is_some()
        })
    }
    fn singleton_scope(&self) -> bool {
        self.stack
            .iter()
            .rev()
            .take_while(|n| !self.owner_boundary(n))
            .any(|n| n.as_singleton_class_node().is_some())
    }
    fn scope_key(&self) -> usize {
        self.stack
            .iter()
            .rev()
            .find(|n| self.owner_boundary(n) || n.as_singleton_class_node().is_some())
            .map_or(0, |n| n.location().start_offset())
    }
    fn nesting(&self, node: &Node<'_>) -> Vec<String> {
        self.stack
            .iter()
            .filter_map(|ancestor| {
                let body = if let Some(c) = ancestor.as_class_node() {
                    c.body()
                } else {
                    ancestor.as_module_node().and_then(|m| m.body())
                }?;
                let loc = node.location();
                (loc.start_offset() >= body.location().start_offset()
                    && loc.end_offset() <= body.location().end_offset())
                .then(|| {
                    self.scopes
                        .get(&ancestor.location().start_offset())
                        .cloned()
                })
                .flatten()
            })
            .collect()
    }
    fn owner(&self, nesting: &[String]) -> Option<Reference> {
        if self
            .stack
            .iter()
            .rev()
            .take_while(|n| !self.owner_boundary(n))
            .filter(|n| n.as_singleton_class_node().is_some())
            .count()
            > 1
        {
            return None;
        }
        for n in self.stack.iter().rev() {
            if n.as_def_node().is_some() {
                return None;
            }
            if let Some(singleton) = n.as_singleton_class_node() {
                let expression = singleton.expression();
                return if expression.as_self_node().is_some() {
                    Some(Reference {
                        name: nesting.last()?.clone(),
                        absolute: true,
                        nesting: vec![],
                    })
                } else {
                    reference(&expression, nesting)
                };
            }
            if let Some(owner) = self.block_owners.get(&n.location().start_offset()) {
                return owner.as_ref().map(|name| Reference {
                    name: name.clone(),
                    absolute: true,
                    nesting: vec![],
                });
            }
            if n.as_class_node().is_some() || n.as_module_node().is_some() {
                return Some(Reference {
                    name: nesting.last()?.clone(),
                    absolute: true,
                    nesting: vec![],
                });
            }
        }
        None
    }
}
impl<'pr> NodeVisitor<'pr> for Index<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let nesting = self.nesting(&node);
        let declaration = if let Some(c) = node.as_class_node() {
            Some((c.constant_path(), c.superclass()))
        } else {
            node.as_module_node().map(|m| (m.constant_path(), None))
        };
        if let Some((path, superclass)) = declaration
            && let Some(r) = reference(&path, &nesting)
        {
            let name = if r.absolute || nesting.is_empty() {
                r.name.clone()
            } else if let Some((parent, last)) = r.name.rsplit_once("::") {
                let prefix = Reference {
                    name: parent.into(),
                    ..r.clone()
                };
                self.resolve(&prefix)
                    .map_or_else(|| r.name.clone(), |p| format!("{p}::{last}"))
            } else {
                format!("{}::{}", nesting.last().unwrap(), r.name)
            };
            self.scopes
                .insert(node.location().start_offset(), name.clone());
            if node.as_class_node().is_some() {
                self.class_declarations.insert(
                    node.location().start_offset(),
                    node.location().as_slice().as_ptr() as usize,
                );
            }
            if !self.seeding {
                self.declared.insert(name.clone());
            }
            let ns = self.namespaces.entry(name).or_default();
            ns.module = node.as_module_node().is_some();
            if let Some(parent) = superclass
                && let Some(r) = reference(&parent, &nesting)
            {
                ns.parents.push(r);
            }
        }
        if let Some(name) = constant_write_name(&node) {
            let full = nesting
                .last()
                .map_or_else(|| name.clone(), |scope| format!("{scope}::{name}"));
            self.constants.insert(full);
        }
        if let Some(target) = constant_path_write(&node)
            && let Some(r) = reference(&target, &nesting)
        {
            let full = r
                .name
                .rsplit_once("::")
                .and_then(|(parent, last)| {
                    self.resolve(&Reference {
                        name: parent.into(),
                        ..r.clone()
                    })
                    .map(|p| format!("{p}::{last}"))
                })
                .unwrap_or(r.name);
            self.constants.insert(full);
        }
        if let Some(call) = node.as_call_node()
            && call.name().as_slice() == b"new"
            && let Some(receiver) = call.receiver()
            && matches!(
                receiver.location().as_slice(),
                b"Class" | b"::Class" | b"Module" | b"::Module"
            )
        {
            let assigned = self.stack.last().and_then(|parent| {
                if let Some(write) = parent.as_constant_write_node() {
                    let name = String::from_utf8_lossy(write.name().as_slice());
                    Some(
                        nesting
                            .last()
                            .map_or_else(|| name.to_string(), |scope| format!("{scope}::{name}")),
                    )
                } else if let Some(write) = parent.as_constant_path_write_node() {
                    let r = reference(&write.target().as_node(), &nesting)?;
                    if let Some((prefix, last)) = r.name.rsplit_once("::") {
                        self.resolve(&Reference {
                            name: prefix.into(),
                            ..r.clone()
                        })
                        .map(|p| format!("{p}::{last}"))
                    } else {
                        Some(r.name)
                    }
                } else {
                    None
                }
            });
            if let Some(name) = &assigned {
                if !self.seeding {
                    self.declared.insert(name.clone());
                }
                let ns = self.namespaces.entry(name.clone()).or_default();
                ns.module = receiver.location().as_slice().ends_with(b"Module");
                if !ns.module
                    && let Some(parent) = call.arguments().and_then(|a| a.arguments().iter().next())
                    && let Some(r) = reference(&parent, &nesting)
                {
                    ns.parents.push(r);
                }
            }
            if let Some(block) = call.block().filter(|n| n.as_block_node().is_some()) {
                self.block_owners
                    .insert(block.location().start_offset(), assigned);
            }
        }
        if self.seeding {
            self.stack.push(node);
            return;
        }
        if let Some(alias) = node.as_alias_method_node()
            && let Some(owner) = self.owner(&nesting)
            && let Some(name) = literal_name(&alias.new_name())
        {
            self.methods
                .push((owner, self.singleton_scope(), name, None, true));
        }
        if let Some(def) = node.as_def_node() {
            let owner = match def.receiver() {
                Some(r) if r.as_self_node().is_some() && self.singleton_scope() => None,
                Some(r) if r.as_self_node().is_some() => self.owner(&nesting).or_else(|| {
                    self.top_level().then(|| Reference {
                        name: "Object".into(),
                        absolute: true,
                        nesting: vec![],
                    })
                }),
                Some(r) => reference(&r, &nesting),
                None if self.top_level() => Some(Reference {
                    name: "Object".into(),
                    absolute: true,
                    nesting: vec![],
                }),
                None => self.owner(&nesting),
            };
            // Rubydex indexes a top-level `def self.f` as a public Object
            // instance member, even inside an ordinary block such as class_eval.
            // Do not infer the block receiver as its owner: the pinned graph
            // also exposes this member through unrelated constants' ancestry.
            let top_self =
                self.top_level() && def.receiver().is_some_and(|r| r.as_self_node().is_some());
            let singleton = def.receiver().is_some() && !top_self || self.singleton_scope();
            let module_function = !singleton
                && (self.module_functions.contains(&self.scope_key())
                    || self
                        .stack
                        .iter()
                        .rev()
                        .find(|n| n.as_arguments_node().is_none())
                        .and_then(|n| n.as_call_node())
                        .is_some_and(|c| {
                            c.name().as_slice() == b"module_function" && c.receiver().is_none()
                        }));
            let public = !module_function
                && (def.receiver().is_some()
                    || self
                        .visibility
                        .get(&self.scope_key())
                        .copied()
                        .unwrap_or(!self.top_level() || singleton));
            if owner.as_ref().is_some_and(|r| r.name == "Object") {
                self.namespaces.entry("Object".into()).or_default();
            }
            if let Some(owner) = owner {
                if module_function {
                    self.methods.push((
                        owner.clone(),
                        true,
                        String::from_utf8_lossy(def.name().as_slice()).into(),
                        shape(&def),
                        true,
                    ));
                }
                self.methods.push((
                    owner,
                    singleton,
                    String::from_utf8_lossy(def.name().as_slice()).into(),
                    shape(&def),
                    public,
                ));
            }
        }
        if let Some(call) = node.as_call_node() {
            let name = call.name().as_slice();
            // Rubydex also records explicit-self mixins on the current owner.
            // Do not broaden visibility/attribute handling or infer other receivers.
            if call.receiver().is_none()
                || (call.receiver().is_some_and(|r| r.as_self_node().is_some())
                    && matches!(name, b"include" | b"extend" | b"prepend"))
            {
                let args: Vec<_> = call
                    .arguments()
                    .map(|a| a.arguments().iter().collect())
                    .unwrap_or_default();
                if matches!(
                    name,
                    b"private"
                        | b"protected"
                        | b"public"
                        | b"private_class_method"
                        | b"public_class_method"
                ) {
                    let public = matches!(name, b"public" | b"public_class_method");
                    if args.is_empty() {
                        self.visibility.insert(self.scope_key(), public);
                        self.module_functions.remove(&self.scope_key());
                    } else if let Some(owner) = self.owner(&nesting).or_else(|| {
                        nesting.is_empty().then(|| Reference {
                            name: "Object".into(),
                            absolute: true,
                            nesting: vec![],
                        })
                    }) {
                        for arg in args {
                            let method = if let Some(name) = literal_name(&arg) {
                                Some(name)
                            } else {
                                arg.as_def_node().map(|d| {
                                    String::from_utf8_lossy(d.name().as_slice()).into_owned()
                                })
                            };
                            if let Some(method) = method {
                                self.visibility_changes.push((
                                    owner.clone(),
                                    self.singleton_scope() || name.ends_with(b"_class_method"),
                                    method,
                                    public,
                                ));
                            }
                        }
                    }
                } else if name == b"module_function" {
                    if args.is_empty() {
                        self.module_functions.insert(self.scope_key());
                    } else if let Some(owner) = self.owner(&nesting) {
                        for arg in &args {
                            if let Some(method) = literal_name(arg) {
                                self.methods.push((owner.clone(), true, method, None, true));
                            }
                        }
                    }
                } else if matches!(
                    name,
                    b"attr" | b"attr_reader" | b"attr_writer" | b"attr_accessor" | b"alias_method"
                ) {
                    if let Some(owner) = self.owner(&nesting) {
                        for arg in args.iter().take(if name == b"alias_method" {
                            1
                        } else {
                            args.len()
                        }) {
                            if let Some(method) = literal_name(arg) {
                                if name != b"attr_writer" {
                                    self.methods.push((
                                        owner.clone(),
                                        self.singleton_scope(),
                                        method.clone(),
                                        None,
                                        true,
                                    ));
                                }
                                if matches!(name, b"attr_writer" | b"attr_accessor") {
                                    self.methods.push((
                                        owner.clone(),
                                        self.singleton_scope(),
                                        format!("{method}="),
                                        None,
                                        true,
                                    ));
                                }
                            }
                        }
                    }
                } else if matches!(name, b"include" | b"extend" | b"prepend")
                    && let Some(owner) = self.owner(&nesting).and_then(|r| self.resolve(&r))
                {
                    let singleton = self.singleton_scope() || name == b"extend";
                    let ns = self.namespaces.get_mut(&owner).unwrap();
                    for arg in args.into_iter().rev() {
                        if let Some(r) = reference(&arg, &nesting) {
                            ns.mixins.push((r, singleton, name == b"prepend"));
                        }
                    }
                }
            }
            if call.name().as_slice() != b"new"
                && call
                    .receiver()
                    .is_some_and(|r| reference(&r, &nesting).is_some())
            {
                self.calls.push((call, nesting));
            }
        }
        self.stack.push(node);
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop argument_mismatch_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Index inputs are supplied together.
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert!(parsed.errors().next().is_none(), "{source}");
        let mut out = Vec::new();
        ArgumentMismatch.check_with_index(Some(Index::supplied(&parsed.node())), &file, &mut out);
        out
    }

    #[test]
    fn absent_index_and_explicit_index_have_distinct_real_results() {
        let source = "class A\n  def self.f(a); end\nend\nA.f\n";
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let mut out = Vec::new();
        ArgumentMismatch.check_with_index(None, &file, &mut out);
        assert!(out.is_empty());
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (4, 3));
        assert_eq!(
            found[0].message,
            "Wrong number of arguments to `f` (given 0, expected 1)."
        );
        assert!(
            crate::pipeline::lint_str(source)
                .into_iter()
                .all(|o| o.cop != "Lint/ArgumentMismatch")
        );
    }

    #[test]
    fn all_347_explicit_index_projections_and_controls() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/argument_mismatch.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 347);
        for row in rows.as_array().unwrap() {
            let found: Vec<_> = offenses(row["source"].as_str().unwrap())
                .into_iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, crate::offense::Severity::Error);
                    (o.line, o.col, o.end_line, o.message)
                })
                .collect();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            assert_eq!(found, expected, "{row}");
        }
    }

    #[test]
    fn explicit_index_deep_ancestry_and_cycles_remain_bounded() {
        let mut source = String::from("class C0; def self.f(a); end; end\n");
        for i in 1..2500 {
            source.push_str(&format!("class C{i} < C{}; end\n", i - 1));
        }
        for _ in 0..100 {
            source.push_str("C2499.f\n");
        }
        assert_eq!(offenses(&source).len(), 100);
        source.push_str("class C0 < Unknown; end\n");
        assert!(offenses(&source).is_empty());
        assert!(offenses("class A < B; def self.f(a); end; include Unknown; end\nclass B < A; end\nA.f\nB.f\n").is_empty());
    }
    #[test]
    fn fixed_arity_and_selector_location() {
        let found = offenses(
            "class Report\n  def self.generate(source, format); end\nend\nReport.generate(source)\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 8, 4));
        assert_eq!(
            found[0].message,
            "Wrong number of arguments to `generate` (given 1, expected 2)."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn lexical_constants_precede_inherited_constants() {
        let found = offenses(
            "module Outer\n  class C\n    def self.f(a); end\n  end\n  class Parent\n    class C\n      def self.f(a, b); end\n    end\n  end\n  class Child < Parent\n    C.f(1)\n    C.f(1, 2)\n  end\nend\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (12, 7));
        assert_eq!(
            found[0].message,
            "Wrong number of arguments to `f` (given 2, expected 1)."
        );
    }
    #[test]
    fn namespace_factory_blocks_own_their_methods() {
        for factory in ["Class", "Module"] {
            let found = offenses(&format!(
                "module Outer\n  A = {factory}.new do\n    def self.f(a); end\n  end\nend\nOuter::A.f\nOuter.f\n",
            ));
            assert_eq!(found.len(), 1, "{factory}");
            assert_eq!((found[0].line, found[0].col), (6, 10));
            assert_eq!(
                found[0].message,
                "Wrong number of arguments to `f` (given 0, expected 1)."
            );
        }
        assert!(
            offenses("module Outer; x = Class.new { def self.f(a); end }; end\nOuter.f\n")
                .is_empty()
        );
    }
    #[test]
    fn top_level_self_definitions_follow_pinned_object_fallback_not_eval_inference() {
        let found = offenses(
            "class A; end\nclass B; end\nA.class_eval { def self.f(a); end }\nA.f\nB.f\nObject.f\n",
        );
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(4, 3), (5, 3), (6, 8)]
        );
        assert!(
            found
                .iter()
                .all(|o| o.message == "Wrong number of arguments to `f` (given 0, expected 1).")
        );
    }
    #[test]
    fn modules_without_singleton_declarations_inherit_known_instance_methods() {
        for owner in ["Module", "Object"] {
            let found = offenses(&format!(
                "class {owner}\n  def f(a); end\nend\nmodule M; end\nM.f\nM.f(1)\n"
            ));
            assert_eq!(found.len(), 1, "{owner}");
            assert_eq!((found[0].line, found[0].col), (5, 3));
            assert_eq!(
                found[0].message,
                "Wrong number of arguments to `f` (given 0, expected 1)."
            );
        }
    }
    #[test]
    fn accepts_matching_and_unknown_receivers() {
        for call in [
            "Report.generate(1, 2)",
            "Other.generate(1)",
            "report.generate(1)",
            "generate(1)",
            "Report.new(1)",
            "Report.generate(*args)",
        ] {
            assert!(
                offenses(&format!(
                    "class Report; def self.generate(a, b); end; end\n{call}\n"
                ))
                .is_empty(),
                "{call}"
            );
        }
    }
}
