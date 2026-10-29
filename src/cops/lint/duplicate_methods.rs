//! `Lint/DuplicateMethods` — flags a method defined twice in the same scope
//! of one file.
//!
//! Ports RuboCop's single-file semantics, including its scope naming: `def`,
//! `def self.x`, `def Const.x`, `class << self`, `alias`, `alias_method`,
//! `attr_*`, Forwardable's `def_delegator(s)`, `class_eval` blocks,
//! `Class.new`/`Module.new` blocks (named via a constant or anonymous), nested
//! `def`s (keyed by the enclosing method), and RuboCop's `rescue`/`ensure`
//! leniency (one redefinition per key inside such branches). Definitions
//! inside an `if`/`unless` are ignored, as are self-aliases (`alias foo foo`).
//!
//! Deviations from RuboCop:
//! - Single file only: RuboCop's cross-file detection (shared cop state or a
//!   project index) has no equivalent in rustocop's stateless, parallel cops.
//! - `AllCops/ActiveSupportExtensionsEnabled` is RuboCop's default (off), so
//!   `delegate` and Active Support redefinition markers are not recognized.
//! - `def Const.x` where `Const` names no enclosing class or module is not
//!   tracked (RuboCop keys it on the node's s-expression, an accident).

use std::collections::{HashMap, HashSet};

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{const_name, is_const, is_global_const, short_const_name},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/DuplicateMethods` cop.
pub struct DuplicateMethods;

impl Cop for DuplicateMethods {
    fn name(&self) -> &'static str {
        "Lint/DuplicateMethods"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Definitions are tracked across the whole file, so the cop walks the
        // tree itself, once, from the root.
        if node.as_program_node().is_none() {
            return;
        }
        let mut walker = Walker {
            cop: self,
            file,
            out,
            stack: Vec::new(),
            definitions: HashMap::new(),
            scopes: HashMap::new(),
        };
        walk(&mut walker, node);
    }
}

/// A `rescue` or `ensure` region (RuboCop tracks redefinitions per kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Region {
    Rescue,
    Ensure,
}

/// A constant's namespace, as RuboCop's `defined_module` destructures it.
enum Namespace<'pr> {
    None,
    /// A leading `::`.
    Cbase,
    Node(Node<'pr>),
}

/// What RuboCop's (parser gem) AST would have as a node's parent.
enum Parent {
    /// No parent: the node is the root.
    Root,
    /// A block; the index points at the call that owns it (or the lambda).
    Block(usize),
    /// An implicit `begin` grouping several statements (or parentheses);
    /// records whether that `begin`'s own parent is a block.
    Begin {
        in_block: bool,
    },
    /// A method call (the node is its receiver or an argument).
    Call(usize),
    Casgn,
    Ivasgn,
    Lvasgn,
    /// A method definition (`def` or `def recv.name`).
    Def(usize),
    Other,
}

struct Walker<'a, 'pr> {
    cop: &'a DuplicateMethods,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    /// Ancestors of the node being visited, outermost first.
    stack: Vec<Node<'pr>>,
    /// Method key -> line of the definition that currently owns it.
    definitions: HashMap<String, u32>,
    /// Keys already redefined once inside a `rescue`/`ensure` region.
    scopes: HashMap<Region, HashSet<String>>,
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A block with explicit or no parameters (`numblock`/`itblock` are other
/// node types in RuboCop's AST).
fn is_regular_block(parameters: Option<Node<'_>>) -> bool {
    !matches!(
        parameters,
        Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
    )
}

/// `Class.new(...)` / `Module.new(...)` on any constant path ending in
/// `Class`/`Module` (RuboCop's `(send (const _ {:Class :Module}) :new ...)`).
fn is_class_or_module_new(call: &CallNode<'_>) -> bool {
    call.name().as_slice() == b"new"
        && !call.is_safe_navigation()
        && call.receiver().is_some_and(|receiver| {
            is_const(&receiver)
                && short_const_name(&receiver)
                    .is_some_and(|name| name == "Class" || name == "Module")
        })
}

/// A `Class.new`/`Module.new` call with a `do`/`{}` block.
fn is_class_or_module_new_block(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        is_class_or_module_new(&call)
            && call
                .block()
                .is_some_and(|block| block.as_block_node().is_some())
    })
}

/// The value of a constant assignment, if `node` is one.
fn casgn_value<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    if let Some(write) = node.as_constant_write_node() {
        Some(write.value())
    } else {
        node.as_constant_path_write_node()
            .map(|write| write.value())
    }
}

/// Whether `node` is the constant path that `write` (`A::B = …`, `A::B ||= …`,
/// `A::B &&= …`, `A::B += …`) assigns.
fn is_constant_path_target(write: &Node<'_>, node: &Node<'_>) -> bool {
    let target = if let Some(write) = write.as_constant_path_write_node() {
        write.target()
    } else if let Some(write) = write.as_constant_path_or_write_node() {
        write.target()
    } else if let Some(write) = write.as_constant_path_and_write_node() {
        write.target()
    } else if let Some(write) = write.as_constant_path_operator_write_node() {
        write.target()
    } else {
        return false;
    };
    target.location().start_offset() == node.location().start_offset()
}

/// The namespace and name a constant assignment writes.
fn casgn_target<'pr>(node: &Node<'pr>) -> Option<(Namespace<'pr>, String)> {
    if let Some(write) = node.as_constant_write_node() {
        Some((Namespace::None, text(write.name().as_slice())))
    } else {
        let target = node.as_constant_path_write_node()?.target();
        let namespace = match target.parent() {
            None => Namespace::Cbase,
            Some(parent) => Namespace::Node(parent),
        };
        Some((namespace, text(target.name()?.as_slice())))
    }
}

/// RuboCop's `defined_module`: the namespace and name a class, module, or
/// `Const = Class.new`/`Module.new` assignment defines.
fn defined_module<'pr>(node: &Node<'pr>) -> Option<(Namespace<'pr>, String)> {
    let path = if let Some(class) = node.as_class_node() {
        class.constant_path()
    } else if let Some(module) = node.as_module_node() {
        module.constant_path()
    } else {
        let value = casgn_value(node)?;
        let call = value.as_call_node()?;
        let defines_module = call.name().as_slice() == b"new"
            && call
                .receiver()
                .is_some_and(|receiver| is_global_const(&receiver, &["Class", "Module"]));
        return if defines_module {
            casgn_target(node)
        } else {
            None
        };
    };
    destructure_const(&path)
}

/// Split a constant reference into namespace and name.
fn destructure_const<'pr>(node: &Node<'pr>) -> Option<(Namespace<'pr>, String)> {
    if let Some(read) = node.as_constant_read_node() {
        return Some((Namespace::None, text(read.name().as_slice())));
    }
    let path = node.as_constant_path_node()?;
    let namespace = match path.parent() {
        None => Namespace::Cbase,
        Some(parent) => Namespace::Node(parent),
    };
    Some((namespace, text(path.name()?.as_slice())))
}

/// RuboCop's `const_name` of a `defined_module` result.
fn qualified_const_name(namespace: &Namespace<'_>, name: &str) -> String {
    match namespace {
        Namespace::None | Namespace::Cbase => name.to_string(),
        Namespace::Node(parent) => {
            format!("{}::{name}", const_name(parent).unwrap_or_default())
        }
    }
}

/// RuboCop's `qualified_name(enclosing, namespace, mod_name)`.
fn qualified_name(enclosing: Option<&str>, namespace: &Namespace<'_>, name: &str) -> String {
    let namespace = match namespace {
        Namespace::None => None,
        Namespace::Cbase => Some(String::new()),
        Namespace::Node(node) => Some(const_name(node).unwrap_or_default()),
    };
    match (enclosing, namespace) {
        (Some("Object"), Some(namespace)) => format!("{namespace}::{name}"),
        (Some("Object"), None) => name.to_string(),
        (enclosing, Some(namespace)) => {
            format!("{}::{namespace}::{name}", enclosing.unwrap_or_default())
        }
        (enclosing, None) => format!("{}::{name}", enclosing.unwrap_or_default()),
    }
}

/// RuboCop's `humanize_scope`: rewrites the first singleton-class marker
/// (`A::#<Class:A>` or `#<Class:A>`) to `A.`, and appends `#` to instance
/// scopes. A literal port of the regexp
/// `(?:(?<name>.*)::)#<Class:\k<name>>|#<Class:(?<name>.*)>(?:::)?`.
fn humanize_scope(scope: &str) -> String {
    let replaced = replace_first_singleton(scope);
    if replaced.ends_with('.') {
        replaced
    } else {
        format!("{replaced}#")
    }
}

fn replace_first_singleton(scope: &str) -> String {
    const MARKER: &str = "#<Class:";
    if !scope.contains(MARKER) {
        return scope.to_string();
    }
    let separators: Vec<usize> = scope.match_indices("::#<Class:").map(|(i, _)| i).collect();
    for start in 0..=scope.len() {
        if !scope.is_char_boundary(start) {
            continue;
        }
        // First alternative, greedy: `name::#<Class:name>`.
        for &sep in separators.iter().rev().filter(|&&sep| sep >= start) {
            let name = &scope[start..sep];
            let tail = format!("::{MARKER}{name}>");
            if scope[sep..].starts_with(&tail) {
                return format!("{}{name}.{}", &scope[..start], &scope[sep + tail.len()..]);
            }
        }
        // Second alternative: `#<Class:name>` (greedy up to the last `>`).
        if scope[start..].starts_with(MARKER) {
            let inner = start + MARKER.len();
            if let Some(offset) = scope[inner..].rfind('>') {
                let close = inner + offset;
                let mut end = close + 1;
                if scope[end..].starts_with("::") {
                    end += 2;
                }
                return format!(
                    "{}{}.{}",
                    &scope[..start],
                    &scope[inner..close],
                    &scope[end..]
                );
            }
        }
    }
    scope.to_string()
}

/// A `sym` or `str` argument's value.
fn sym_or_str_value(node: &Node<'_>) -> Option<String> {
    if let Some(symbol) = node.as_symbol_node() {
        Some(text(symbol.unescaped()))
    } else {
        node.as_string_node().map(|string| text(string.unescaped()))
    }
}

impl<'pr> Walker<'_, 'pr> {
    fn inside_if(&self) -> bool {
        self.stack
            .iter()
            .any(|ancestor| matches!(ancestor, Node::IfNode { .. } | Node::UnlessNode { .. }))
    }

    /// Whether the call at `index` is a `MatchWriteNode`'s (a regexp with
    /// named captures matched with `=~`).
    fn is_match_write_call(&self, index: usize) -> bool {
        index
            .checked_sub(1)
            .is_some_and(|parent| self.stack[parent].as_match_write_node().is_some())
    }

    /// The call owning the block at `index`, if it is a call.
    fn block_call(&self, block_index: usize) -> Option<CallNode<'pr>> {
        block_index
            .checked_sub(1)
            .and_then(|i| self.stack[i].as_call_node())
    }

    /// RuboCop's `parent_module_name` for a node whose ancestors are
    /// `stack[..end]`: the enclosing class/module path (`Object` at top
    /// level), or `None` when it cannot be determined.
    fn parent_module_name(&self, end: usize) -> Option<String> {
        let mut parts = Vec::new();
        for index in (0..end).rev() {
            let ancestor = &self.stack[index];
            match ancestor {
                Node::ClassNode { .. } | Node::ModuleNode { .. } => {
                    if let Some((namespace, name)) = defined_module(ancestor) {
                        parts.push(qualified_const_name(&namespace, &name));
                    }
                }
                Node::ConstantWriteNode { .. } | Node::ConstantPathWriteNode { .. } => {
                    if let Some((namespace, name)) = defined_module(ancestor) {
                        parts.push(qualified_const_name(&namespace, &name));
                    }
                }
                Node::SingletonClassNode { .. } => {
                    let subject = ancestor.as_singleton_class_node().unwrap().expression();
                    if is_const(&subject) {
                        parts.push(format!(
                            "#<Class:{}>",
                            const_name(&subject).unwrap_or_default()
                        ));
                    } else if subject.as_self_node().is_some() {
                        let enclosing = self.parent_module_name(index).unwrap_or_default();
                        parts.push(format!("#<Class:{enclosing}>"));
                    } else {
                        return None;
                    }
                }
                Node::BlockNode { .. } => {
                    let block = ancestor.as_block_node().unwrap();
                    if !is_regular_block(block.parameters()) {
                        continue;
                    }
                    let call = self.block_call(index)?;
                    if call.name().as_slice() == b"class_eval" {
                        match call.receiver() {
                            None => {}
                            Some(receiver) if is_const(&receiver) => {
                                parts.push(const_name(&receiver).unwrap_or_default());
                            }
                            Some(_) => return None,
                        }
                    } else if !self.is_new_class_or_module_block(index - 1) {
                        return None;
                    }
                }
                Node::LambdaNode { .. } => {
                    let lambda = ancestor.as_lambda_node().unwrap();
                    if is_regular_block(lambda.parameters()) {
                        return None;
                    }
                }
                _ => {}
            }
        }
        if parts.is_empty() {
            Some("Object".to_string())
        } else {
            parts.reverse();
            Some(parts.join("::"))
        }
    }

    /// RuboCop's `new_class_or_module_block?`: `Const = Class.new do ... end`
    /// with no arguments to `new`.
    fn is_new_class_or_module_block(&self, call_index: usize) -> bool {
        let Some(call) = self.stack[call_index].as_call_node() else {
            return false;
        };
        is_class_or_module_new(&call)
            && call.arguments().is_none()
            && call_index
                .checked_sub(1)
                .is_some_and(|parent| casgn_value(&self.stack[parent]).is_some())
    }

    /// RuboCop's AST parent of the node at `index`.
    fn parser_parent(&self, index: usize) -> (Parent, usize) {
        let mut j = index;
        while j > 0 {
            j -= 1;
            let node = &self.stack[j];
            match node {
                Node::StatementsNode { .. } => {
                    if node.as_statements_node().unwrap().body().iter().count() < 2 {
                        continue;
                    }
                    let parent = j.checked_sub(1).map(|i| &self.stack[i]);
                    return match parent {
                        Some(Node::ParenthesesNode { .. }) => (self.begin_parent(j - 1), j - 1),
                        Some(Node::BeginNode { .. }) => {
                            let begin = parent.unwrap().as_begin_node().unwrap();
                            if begin.rescue_clause().is_none() && begin.ensure_clause().is_none() {
                                (Parent::Other, j - 1)
                            } else {
                                (Parent::Begin { in_block: false }, j)
                            }
                        }
                        Some(Node::BlockNode { .. } | Node::LambdaNode { .. }) => {
                            (Parent::Begin { in_block: true }, j)
                        }
                        _ => (Parent::Begin { in_block: false }, j),
                    };
                }
                Node::ParenthesesNode { .. } => return (self.begin_parent(j), j),
                Node::BlockNode { .. } => {
                    return match self.block_call(j) {
                        Some(_) => (Parent::Block(j - 1), j - 1),
                        None => (Parent::Other, j),
                    };
                }
                Node::LambdaNode { .. } => return (Parent::Block(j), j),
                // `/(?<name>..)/ =~ expr` is a `match_with_lvasgn`, not a call.
                // (Its receiver is a leaf regexp, so only its arguments can be
                // on the path.)
                Node::ArgumentsNode { .. } => {
                    return match j.checked_sub(1).map(|i| &self.stack[i]) {
                        Some(Node::CallNode { .. }) if self.is_match_write_call(j - 1) => {
                            (Parent::Other, j - 2)
                        }
                        Some(Node::CallNode { .. }) => (Parent::Call(j - 1), j - 1),
                        _ => (Parent::Other, j),
                    };
                }
                Node::CallNode { .. } => return (Parent::Call(j), j),
                // The parser gem folds the constant path a write assigns into
                // its `casgn`.
                Node::ConstantPathNode { .. }
                    if j > 0 && is_constant_path_target(&self.stack[j - 1], node) =>
                {
                    return (Parent::Casgn, j);
                }
                Node::DefNode { .. } => return (Parent::Def(j), j),
                Node::ConstantWriteNode { .. } | Node::ConstantPathWriteNode { .. } => {
                    return (Parent::Casgn, j);
                }
                Node::InstanceVariableWriteNode { .. } => return (Parent::Ivasgn, j),
                Node::LocalVariableWriteNode { .. } => return (Parent::Lvasgn, j),
                Node::ProgramNode { .. } => return (Parent::Root, j),
                _ => return (Parent::Other, j),
            }
        }
        (Parent::Root, 0)
    }

    /// A `begin` (parentheses at `index`) and whether its parent is a block.
    fn begin_parent(&self, index: usize) -> Parent {
        let in_block = matches!(self.parser_parent(index).0, Parent::Block(_));
        Parent::Begin { in_block }
    }

    /// RuboCop's `anonymous_class_block`: the innermost block, if it is a
    /// `Class.new`/`Module.new` block not assigned to a local variable (and no
    /// `class << obj` intervenes). Returns the index of its call.
    fn anonymous_class_block(&self) -> Option<usize> {
        let end = self.stack.len();
        let block_index = (0..end).rev().find(|&i| match &self.stack[i] {
            Node::BlockNode { .. } => {
                is_regular_block(self.stack[i].as_block_node().unwrap().parameters())
            }
            Node::LambdaNode { .. } => {
                is_regular_block(self.stack[i].as_lambda_node().unwrap().parameters())
            }
            _ => false,
        })?;
        self.stack[block_index].as_block_node()?;
        let call = self.block_call(block_index)?;
        if !is_class_or_module_new(&call) {
            return None;
        }
        let call_index = block_index - 1;
        if matches!(self.parser_parent(call_index).0, Parent::Lvasgn) {
            return None;
        }
        let foreign_singleton = self.stack.iter().any(|ancestor| {
            ancestor
                .as_singleton_class_node()
                .is_some_and(|sclass| sclass.expression().as_self_node().is_none())
        });
        if foreign_singleton {
            return None;
        }
        Some(call_index)
    }

    /// RuboCop's `anon_block_scope_id`: distinguishes anonymous class blocks
    /// that RuboCop considers separate scopes.
    fn anon_block_scope_id(&self, call_index: usize) -> Option<String> {
        let (parent, _) = self.parser_parent(call_index);
        let anon = self.stack[call_index].as_call_node()?;
        // The parent's receiver and method name (RuboCop's `receiver` and
        // `method_name`): calls, blocks (via their call), and `def recv.x`.
        let named = match parent {
            Parent::Block(i) | Parent::Call(i) => self.stack[i]
                .as_call_node()
                .and_then(|call| Some((call.receiver()?, call.name()))),
            Parent::Def(i) => self.stack[i]
                .as_def_node()
                .and_then(|def| Some((def.receiver()?, def.name()))),
            Parent::Begin { .. } | Parent::Casgn | Parent::Ivasgn => None,
            Parent::Root | Parent::Lvasgn | Parent::Other => return None,
        };
        let is_class_new = anon
            .receiver()
            .and_then(|receiver| short_const_name(&receiver))
            .is_some_and(|name| name == "Class");
        let named = if is_class_new && matches!(parent, Parent::Call(_)) {
            None
        } else {
            named.filter(|(receiver, _)| !is_class_or_module_new_block(receiver))
        };
        if let Some((receiver, method_name)) = named {
            return Some(format!(
                "{}.{}",
                text(receiver.location().as_slice()),
                text(method_name.as_slice())
            ));
        }
        match parent {
            Parent::Begin { in_block: false } => None,
            _ => {
                let start = anon.location().start_offset();
                let (line, _) = self.file.line_col(start);
                Some(format!("{line}:{start}"))
            }
        }
    }

    /// The innermost `rescue`/`ensure` region around the current node.
    fn region(&self) -> Option<Region> {
        for index in (0..self.stack.len()).rev() {
            match &self.stack[index] {
                Node::EnsureNode { .. } => return Some(Region::Ensure),
                Node::RescueNode { .. } | Node::RescueModifierNode { .. } => {
                    return Some(Region::Rescue);
                }
                Node::StatementsNode { .. } | Node::ElseNode { .. } => {
                    let parent = index.checked_sub(1).map(|i| &self.stack[i]);
                    if let Some(begin) = parent.and_then(|parent| parent.as_begin_node()) {
                        if begin.rescue_clause().is_some() {
                            return Some(Region::Rescue);
                        }
                        if begin.ensure_clause().is_some() {
                            return Some(Region::Ensure);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// RuboCop's `method_key`: nested `def`s are keyed by their enclosing
    /// method.
    fn method_key(&self, method_name: &str) -> String {
        let enclosing = self
            .stack
            .iter()
            .rev()
            .find_map(|ancestor| ancestor.as_def_node());
        match enclosing {
            Some(def) => format!("{}.{method_name}", text(def.name().as_slice())),
            None => method_name.to_string(),
        }
    }

    fn found_instance_method(&mut self, node: &Node<'pr>, name: &str) {
        if let Some(scope) = self.parent_module_name(self.stack.len()) {
            self.found_method(node, format!("{}{name}", humanize_scope(&scope)), None);
        } else if let Some(anon) = self.anonymous_class_block() {
            let base = qualified_name(
                self.parent_module_name(anon).as_deref(),
                &Namespace::None,
                "Object",
            );
            let in_singleton = self
                .stack
                .iter()
                .any(|ancestor| ancestor.as_singleton_class_node().is_some());
            let scope = if in_singleton {
                format!("#<Class:{base}>")
            } else {
                base
            };
            let scope_id = self.anon_block_scope_id(anon);
            self.found_method(node, format!("{}{name}", humanize_scope(&scope)), scope_id);
        } else {
            self.found_sclass_method(node, name);
        }
    }

    /// `class << expr` where `expr` is a method call: keyed on the call name.
    fn found_sclass_method(&mut self, node: &Node<'pr>, name: &str) {
        let Some(sclass) = self
            .stack
            .iter()
            .rev()
            .find_map(|ancestor| ancestor.as_singleton_class_node())
        else {
            return;
        };
        let Some(subject) = sclass.expression().as_call_node() else {
            return;
        };
        if subject.is_safe_navigation() {
            return;
        }
        let method_name = format!("{}.{name}", text(subject.name().as_slice()));
        self.found_method(node, method_name, None);
    }

    fn check_self_receiver(&mut self, node: &Node<'pr>, name: &str) {
        if let Some(enclosing) = self.parent_module_name(self.stack.len()) {
            self.found_method(node, format!("{enclosing}.{name}"), None);
        } else if let Some(anon) = self.anonymous_class_block() {
            let scope = qualified_name(
                self.parent_module_name(anon).as_deref(),
                &Namespace::None,
                "Object",
            );
            let scope_id = self.anon_block_scope_id(anon);
            self.found_method(node, format!("{scope}.{name}"), scope_id);
        }
    }

    fn check_const_receiver(&mut self, node: &Node<'pr>, name: &str, receiver: &str) {
        if let Some(qualified) = self.lookup_constant(receiver) {
            self.found_method(node, format!("{qualified}.{name}"), None);
        }
    }

    /// RuboCop's `lookup_constant`: resolve `receiver` against the names of
    /// the enclosing classes and modules (and their namespaces).
    fn lookup_constant(&self, receiver: &str) -> Option<String> {
        for index in (0..self.stack.len()).rev() {
            let ancestor = &self.stack[index];
            if !matches!(
                ancestor,
                Node::ClassNode { .. }
                    | Node::ModuleNode { .. }
                    | Node::ConstantWriteNode { .. }
                    | Node::ConstantPathWriteNode { .. }
            ) {
                continue;
            }
            let (mut namespace, mut mod_name) = match defined_module(ancestor) {
                Some((namespace, name)) => (namespace, Some(name)),
                None => (Namespace::None, None),
            };
            loop {
                if mod_name.as_deref() == Some(receiver) {
                    let enclosing = self.parent_module_name(index);
                    return Some(qualified_name(
                        enclosing.as_deref(),
                        &namespace,
                        mod_name.as_deref().unwrap_or_default(),
                    ));
                }
                (namespace, mod_name) = match namespace {
                    Namespace::None => break,
                    Namespace::Cbase => (Namespace::None, None),
                    Namespace::Node(node) => match destructure_const(&node) {
                        Some((namespace, name)) => (namespace, Some(name)),
                        None => (Namespace::None, None),
                    },
                };
            }
        }
        None
    }

    fn found_method(&mut self, node: &Node<'pr>, method_name: String, scope_id: Option<String>) {
        let mut key = self.method_key(&method_name);
        if let Some(scope_id) = scope_id {
            key = format!("{key}@{scope_id}");
        }
        let region = self.region();
        let (line, col) = self.file.line_col(node.location().start_offset());
        let Some(&previous) = self.definitions.get(&key) else {
            self.definitions.insert(key, line);
            return;
        };
        if let Some(region) = region {
            let redefined = self.scopes.entry(region).or_default();
            if !redefined.contains(&key) {
                redefined.insert(key.clone());
                self.definitions.insert(key, line);
                return;
            }
        }
        // One offense per statement, as RuboCop keeps one per range: e.g.
        // a repeated `attr_accessor :a` duplicates both `a` and `a=`.
        let cop = self.cop.name();
        if self
            .out
            .iter()
            .any(|o| o.cop == cop && o.line == line && o.col == col)
        {
            return;
        }
        let path = self.file.path.display();
        self.out.push(Offense {
            correction: None,
            cop: self.cop.name(),
            severity: self.cop.severity(),
            message: format!(
                "Method `{method_name}` is defined at both {path}:{previous} and {path}:{line}."
            ),
            line,
            end_line: self.file.line_of(
                node.as_def_node()
                    .map_or_else(
                        || crate::cops::ast::source_end(node),
                        |def| def.name_loc().end_offset(),
                    )
                    .saturating_sub(1),
            ),
            col,
        });
    }

    fn on_def(&mut self, node: &Node<'pr>) {
        if self.inside_if() {
            return;
        }
        let def = node.as_def_node().unwrap();
        let name = text(def.name().as_slice());
        match def.receiver() {
            None => self.found_instance_method(node, &name),
            Some(receiver) if receiver.as_self_node().is_some() => {
                self.check_self_receiver(node, &name);
            }
            Some(receiver) if is_const(&receiver) => {
                let short = short_const_name(&receiver).unwrap_or_default();
                self.check_const_receiver(node, &name, &short);
            }
            Some(_) => {}
        }
    }

    fn on_alias(&mut self, node: &Node<'pr>) {
        let alias = node.as_alias_method_node().unwrap();
        let (Some(new_name), Some(old_name)) = (
            alias.new_name().as_symbol_node(),
            alias.old_name().as_symbol_node(),
        ) else {
            return;
        };
        // `alias foo foo` marks an intentional redefinition.
        if new_name.unescaped() == old_name.unescaped() || self.inside_if() {
            return;
        }
        self.found_instance_method(node, &text(new_name.unescaped()));
    }

    fn on_send(&mut self, node: &Node<'pr>) {
        let call = node.as_call_node().unwrap();
        const DEFINERS: [&[u8]; 9] = [
            b"alias_method",
            b"attr_reader",
            b"attr_writer",
            b"attr_accessor",
            b"attr",
            b"def_delegator",
            b"def_instance_delegator",
            b"def_delegators",
            b"def_instance_delegators",
        ];
        if call.receiver().is_some() || !DEFINERS.contains(&call.name().as_slice()) {
            return;
        }
        // A block-pass is an argument in RuboCop's AST.
        if call
            .block()
            .is_some_and(|block| block.as_block_argument_node().is_some())
        {
            return;
        }
        let arguments: Vec<Node<'pr>> = call
            .arguments()
            .map(|arguments| arguments.arguments().iter().collect())
            .unwrap_or_default();
        match call.name().as_slice() {
            b"alias_method" => {
                let [new_name, old_name] = arguments.as_slice() else {
                    return;
                };
                let (Some(new_name), Some(old_name)) =
                    (new_name.as_symbol_node(), old_name.as_symbol_node())
                else {
                    return;
                };
                if new_name.unescaped() == old_name.unescaped() || self.inside_if() {
                    return;
                }
                self.found_instance_method(node, &text(new_name.unescaped()));
            }
            name @ (b"attr_reader" | b"attr_writer" | b"attr_accessor" | b"attr") => {
                if arguments.is_empty() {
                    return;
                }
                let (names, readable, writable) = match name {
                    b"attr" => {
                        let writable =
                            arguments.len() == 2 && arguments[1].as_true_node().is_some();
                        (&arguments[..1], true, writable)
                    }
                    b"attr_reader" => (&arguments[..], true, false),
                    b"attr_writer" => (&arguments[..], false, true),
                    _ => (&arguments[..], true, true),
                };
                for argument in names {
                    let Some(symbol) = argument.as_symbol_node() else {
                        continue;
                    };
                    let attribute = text(symbol.unescaped());
                    if readable {
                        self.found_instance_method(node, &attribute);
                    }
                    if writable {
                        self.found_instance_method(node, &format!("{attribute}="));
                    }
                }
            }
            b"def_delegator" | b"def_instance_delegator" => {
                if !(arguments.len() == 2 || arguments.len() == 3) {
                    return;
                }
                let values: Option<Vec<String>> = arguments.iter().map(sym_or_str_value).collect();
                let Some(values) = values else {
                    return;
                };
                if self.inside_if() {
                    return;
                }
                let name = values.last().unwrap().clone();
                self.found_instance_method(node, &name);
            }
            b"def_delegators" | b"def_instance_delegators" => {
                if arguments.len() < 2 {
                    return;
                }
                let values: Option<Vec<String>> = arguments.iter().map(sym_or_str_value).collect();
                let Some(values) = values else {
                    return;
                };
                if self.inside_if() {
                    return;
                }
                for name in &values[1..] {
                    self.found_instance_method(node, name);
                }
            }
            _ => {}
        }
    }
}

impl<'pr> NodeVisitor<'pr> for Walker<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        match node {
            Node::DefNode { .. } => self.on_def(&node),
            Node::AliasMethodNode { .. } => self.on_alias(&node),
            Node::CallNode { .. } => self.on_send(&node),
            _ => {}
        }
        self.stack.push(node);
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/duplicate_methods_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Examples that
    //! need cross-file state, a project index, or non-default configuration
    //! (`ActiveSupportExtensionsEnabled: true`, `AllowedCrossFilePaths`) do
    //! not apply.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/DuplicateMethods")
            .collect()
    }

    /// (line, col, message) triples.
    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn msg(method: &str, first: u32, second: u32) -> String {
        format!("Method `{method}` is defined at both test.rb:{first} and test.rb:{second}.")
    }

    fn assert_none(source: &str) {
        assert!(
            offenses(source).is_empty(),
            "should not flag:\n{source}\ngot {:?}",
            found(source)
        );
    }

    #[test]
    fn humanizes_scopes_like_rubocop() {
        assert_eq!(humanize_scope("A"), "A#");
        assert_eq!(humanize_scope("A::B"), "A::B#");
        assert_eq!(humanize_scope("A::#<Class:A>"), "A.");
        assert_eq!(humanize_scope("M::A::#<Class:M::A>"), "M::A.");
        assert_eq!(humanize_scope("#<Class:Object>"), "Object.");
        assert_eq!(humanize_scope("M::#<Class:A>"), "M::A.");
        assert_eq!(humanize_scope("A::#<Class:A>::B"), "A.::B#");
        assert_eq!(humanize_scope("#<Class:>"), ".");
    }

    /// The scopes RuboCop's shared `'in scope'` examples run in: opening line
    /// and the receiver named in messages.
    const SCOPES: [(&str, &str); 7] = [
        ("class A", "A"),
        ("module A", "A"),
        ("A = Class.new do", "A"),
        ("A = Module.new do", "A"),
        ("A.class_eval do", "A"),
        ("Class.new do", "Object"),
        ("Module.new do", "Object"),
    ];

    /// Run `check(opening, receiver)` for every scope.
    fn in_each_scope(check: impl Fn(&str, &str)) {
        for (opening, receiver) in SCOPES {
            check(opening, receiver);
        }
    }

    #[test]
    fn flags_duplicate_method_in_scope() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  def some_method\n    implement 1\n  end\n  def some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 3, msg(&format!("{recv}#some_method"), 2, 5))],
                "{open}"
            );
        });
    }

    #[test]
    fn accepts_non_duplicate_methods_in_scope() {
        in_each_scope(|open, _| {
            assert_none(&format!(
                "{open}\n  def some_method\n    implement 1\n  end\n  def any_method\n    implement 2\n  end\nend\n"
            ));
        });
    }

    #[test]
    fn flags_duplicate_class_methods_in_scope() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  def self.some_method\n    implement 1\n  end\n  def self.some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 3, msg(&format!("{recv}.some_method"), 2, 5))],
                "{open}"
            );
        });
    }

    #[test]
    fn accepts_non_duplicate_class_methods_and_instance_vs_class() {
        in_each_scope(|open, _| {
            assert_none(&format!(
                "{open}\n  def self.some_method\n    implement 1\n  end\n  def self.any_method\n    implement 2\n  end\nend\n"
            ));
            assert_none(&format!(
                "{open}\n  def some_method\n    implement 1\n  end\n  def self.some_method\n    implement 2\n  end\nend\n"
            ));
        });
    }

    #[test]
    fn flags_duplicate_private_and_protected_methods() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  private def some_method\n    implement 1\n  end\n  private def some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 11, msg(&format!("{recv}#some_method"), 2, 5))],
                "{open}"
            );
            let source = format!(
                "{open}\n  private def self.some_method\n    implement 1\n  end\n  private def self.some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 11, msg(&format!("{recv}.some_method"), 2, 5))],
                "{open}"
            );
            let source = format!(
                "{open}\n  protected def some_method\n    implement 1\n  end\n  protected def some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 13, msg(&format!("{recv}#some_method"), 2, 5))],
                "{open}"
            );
            assert_none(&format!(
                "{open}\n  private def some_method\n    implement 1\n  end\n  private def any_method\n    implement 2\n  end\nend\n"
            ));
        });
    }

    #[test]
    fn flags_two_pairs_of_duplicates() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  def some_method\n    implement 1\n  end\n  def some_method\n    implement 2\n  end\n  def any_method\n    implement 1\n  end\n  def any_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![
                    (5, 3, msg(&format!("{recv}#some_method"), 2, 5)),
                    (11, 3, msg(&format!("{recv}#any_method"), 8, 11)),
                ],
                "{open}"
            );
        });
    }

    #[test]
    fn flags_duplicates_across_reopened_scopes() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  def some_method\n    implement 1\n  end\nend\n{open}\n  def some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(7, 3, msg(&format!("{recv}#some_method"), 2, 7))],
                "{open}"
            );
            let source = format!(
                "{open}\n  def self.some_method\n    implement 1\n  end\nend\n{open}\n  def self.some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(7, 3, msg(&format!("{recv}.some_method"), 2, 7))],
                "{open}"
            );
        });
    }

    #[test]
    fn understands_class_self_block() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  class << self\n    def some_method\n      implement 1\n    end\n    def some_method\n      implement 2\n    end\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(6, 5, msg(&format!("{recv}.some_method"), 3, 6))],
                "{open}"
            );
        });
    }

    #[test]
    fn understands_nested_modules() {
        in_each_scope(|open, recv| {
            let source = format!(
                "module B\n  {open}\n    def some_method\n      implement 1\n    end\n    def some_method\n      implement 2\n    end\n    def self.another\n    end\n    def self.another\n    end\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![
                    (6, 5, msg(&format!("B::{recv}#some_method"), 3, 6)),
                    (11, 5, msg(&format!("B::{recv}.another"), 9, 11)),
                ],
                "{open}"
            );
        });
    }

    #[test]
    fn flags_class_shovel_expression() {
        in_each_scope(|open, _| {
            let source = format!(
                "{open}\n  class << blah\n    def some_method\n      implement 1\n    end\n    def some_method\n      implement 2\n    end\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(6, 5, msg("blah.some_method", 3, 6))],
                "{open}"
            );
        });
    }

    #[test]
    fn flags_duplicate_alias_and_alias_method() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  def some_method\n    implement 1\n  end\n  alias some_method any_method\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 3, msg(&format!("{recv}#some_method"), 2, 5))],
                "{open}"
            );
            let source = format!(
                "{open}\n  def some_method\n    implement 1\n  end\n  alias_method :some_method, :any_method\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 3, msg(&format!("{recv}#some_method"), 2, 5))],
                "{open}"
            );
        });
    }

    #[test]
    fn accepts_self_aliases_and_non_duplicate_aliases() {
        in_each_scope(|open, _| {
            for body in [
                "  alias some_method some_method\n  def some_method\n    implement 1\n  end\n",
                "  def some_method\n    implement 1\n  end\n  alias any_method some_method\n",
                "  alias_method :some_method, :some_method\n  def some_method\n    implement 1\n  end\n",
                "  alias_method :some_method, unknown()\n  def some_method\n    implement 1\n  end\n",
                "  def some_method\n    implement 1\n  end\n  alias_method :any_method, :some_method\n",
                "  alias $foo $bar\n",
            ] {
                assert_none(&format!("{open}\n{body}end\n"));
            }
        });
    }

    #[test]
    fn flags_duplicate_attribute_methods() {
        in_each_scope(|open, recv| {
            let source = format!("{open}\n  def something\n  end\n  attr_reader :something\nend\n");
            assert_eq!(
                found(&source),
                vec![(4, 3, msg(&format!("{recv}#something"), 2, 4))],
                "{open}"
            );
            let source =
                format!("{open}\n  def something=(right)\n  end\n  attr_writer :something\nend\n");
            assert_eq!(
                found(&source),
                vec![(4, 3, msg(&format!("{recv}#something="), 2, 4))],
                "{open}"
            );
            let source = format!(
                "{open}\n  attr_accessor :something\n\n  def something\n  end\n  def something=(right)\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![
                    (4, 3, msg(&format!("{recv}#something"), 2, 4)),
                    (6, 3, msg(&format!("{recv}#something="), 2, 6)),
                ],
                "{open}"
            );
            let source = format!("{open}\n  def something\n  end\n  attr :something\nend\n");
            assert_eq!(
                found(&source),
                vec![(4, 3, msg(&format!("{recv}#something"), 2, 4))],
                "{open}"
            );
            let source = format!(
                "{open}\n  attr :something, true\n\n  def something\n  end\n  def something=(right)\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![
                    (4, 3, msg(&format!("{recv}#something"), 2, 4)),
                    (6, 3, msg(&format!("{recv}#something="), 2, 6)),
                ],
                "{open}"
            );
        });
    }

    #[test]
    fn accepts_reader_with_setter_and_writer_with_getter() {
        in_each_scope(|open, _| {
            assert_none(&format!(
                "{open}\n  def something=(right)\n  end\n  attr_reader :something\nend\n"
            ));
            assert_none(&format!(
                "{open}\n  def something\n  end\n  attr_writer :something\nend\n"
            ));
        });
    }

    #[test]
    fn flags_duplicate_nested_methods() {
        in_each_scope(|open, recv| {
            let source = format!(
                "{open}\n  def foo\n    def some_method\n      implement 1\n    end\n  end\n\n  def foo\n    def some_method\n      implement 2\n    end\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![
                    (8, 3, msg(&format!("{recv}#foo"), 2, 8)),
                    (9, 5, msg(&format!("{recv}#some_method"), 3, 9)),
                ],
                "{open}"
            );
            let source = format!(
                "{open}\n  def self.foo\n    def some_method\n      implement 1\n    end\n  end\n\n  def self.foo\n    def some_method\n      implement 2\n    end\n  end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![
                    (8, 3, msg(&format!("{recv}.foo"), 2, 8)),
                    (9, 5, msg(&format!("{recv}#some_method"), 3, 9)),
                ],
                "{open}"
            );
        });
    }

    #[test]
    fn accepts_same_name_in_different_methods_and_blocks() {
        in_each_scope(|open, _| {
            for (outer_a, outer_b) in [
                ("def foo", "def bar"),
                ("def self.foo", "def self.bar"),
                ("dsl_like('foo') do", "dsl_like('bar') do"),
            ] {
                assert_none(&format!(
                    "{open}\n  {outer_a}\n    def some_method\n      implement 1\n    end\n  end\n\n  {outer_b}\n    def some_method\n      implement 2\n    end\n  end\nend\n"
                ));
            }
        });
    }

    #[test]
    fn ignores_delegate_without_active_support_extensions() {
        in_each_scope(|open, _| {
            for line in [
                "delegate :some_method, to: :foo",
                "delegate 'some_method', to: :foo",
                "delegate :method, to: :foo, prefix: :some",
                "delegate :method, to: :some, prefix: true",
                "expose :some_method, to: :foo",
            ] {
                assert_none(&format!(
                    "{open}\n  def some_method\n    implement 1\n  end\n  {line}\nend\n"
                ));
            }
        });
    }

    #[test]
    fn flags_forwardable_delegators() {
        in_each_scope(|open, recv| {
            for line in [
                "def_delegator :foo, :bar",
                "def_delegator 'foo', 'bar'",
                "def_instance_delegator :foo, :bar",
                "def_delegators :foo, :bar, :baz",
                "def_delegators 'foo', 'bar', 'baz'",
                "def_instance_delegators :foo, :bar, :baz",
            ] {
                let source = format!("{open}\n  {line}\n\n  def bar; end\nend\n");
                assert_eq!(
                    found(&source),
                    vec![(4, 3, msg(&format!("{recv}#bar"), 2, 4))],
                    "{open} / {line}"
                );
            }
            let source = format!(
                "{open}\n  def_delegator :foo, :bar, :baz\n\n  def bar; end\n\n  def baz; end\nend\n"
            );
            assert_eq!(
                found(&source),
                vec![(6, 3, msg(&format!("{recv}#baz"), 2, 6))],
                "{open}"
            );
        });
    }

    #[test]
    fn ignores_delegators_within_conditions() {
        in_each_scope(|open, _| {
            assert_none(&format!(
                "{open}\n  def_delegator :foo, :bar if baz?\n\n  def bar; end\nend\n"
            ));
            assert_none(&format!(
                "{open}\n  def_delegators :foo, :bar, :baz if qux?\n\n  def bar; end\nend\n"
            ));
        });
    }

    #[test]
    fn flags_same_file_duplicate_despite_self_alias_or_markers() {
        let source = "class A\n  alias_method :some_method, :some_method\n  def some_method\n    implement 1\n  end\n  def some_method\n    implement 2\n  end\nend\n";
        assert_eq!(found(source), vec![(6, 3, msg("A#some_method", 3, 6))]);
        let source = "class A\n  silence_redefinition_of_method :some_method\n  def some_method\n    implement 1\n  end\n  def some_method\n    implement 2\n  end\nend\n";
        assert_eq!(found(source), vec![(6, 3, msg("A#some_method", 3, 6))]);
    }

    #[test]
    fn flags_duplicate_class_methods_with_named_receiver() {
        for kind in ["class", "module"] {
            let source = format!(
                "{kind} A\n  def A.some_method\n    implement 1\n  end\n  def A.some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(found(&source), vec![(5, 3, msg("A.some_method", 2, 5))]);
            let source = format!(
                "{kind} A\n  def self.some_method\n    implement 1\n  end\n  def A.some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(found(&source), vec![(5, 3, msg("A.some_method", 2, 5))]);
            let source = format!(
                "{kind} A\n  class << self\n    def some_method\n      implement 1\n    end\n  end\n  def A.some_method\n    implement 2\n  end\nend\n"
            );
            assert_eq!(found(&source), vec![(7, 3, msg("A.some_method", 3, 7))]);
        }
    }

    #[test]
    fn flags_duplicate_methods_at_top_level() {
        let source = "def some_method\n  implement 1\nend\ndef some_method\n  implement 2\nend\n";
        assert_eq!(found(source), vec![(4, 1, msg("Object#some_method", 1, 4))]);
    }

    #[test]
    fn understands_class_shovel_constant() {
        let source = "class << A\n  def some_method\n    implement 1\n  end\n  def some_method\n    implement 2\n  end\nend\n";
        assert_eq!(found(source), vec![(5, 3, msg("A.some_method", 2, 5))]);
    }

    #[test]
    fn handles_class_eval_with_implicit_receiver() {
        let source = "module A\n  class_eval do\n    def some_method\n      implement 1\n    end\n    def some_method\n      implement 2\n    end\n  end\nend\n";
        assert_eq!(found(source), vec![(6, 5, msg("A#some_method", 3, 6))]);
    }

    #[test]
    fn ignores_definitions_in_rspec_describe_blocks() {
        assert_none(
            "describe \"something\" do\n  def some_method\n    implement 1\n  end\n  def some_method\n    implement 2\n  end\nend\n",
        );
    }

    #[test]
    fn ignores_class_new_blocks_assigned_to_local_variables() {
        assert_none(
            "a = Class.new do\n  def foo\n  end\nend\nb = Class.new do\n  def foo\n  end\nend\n",
        );
    }

    #[test]
    fn distinguishes_class_new_blocks_assigned_to_different_constants() {
        assert_none(
            "self::A = Class.new do\n  def foo\n    1\n  end\nend\nself::B = Class.new do\n  def foo\n    2\n  end\nend\n",
        );
        assert_none(
            "self::A = Class.new do\n  def self.name\n    'Foo'\n  end\nend\nself::B = Class.new do\n  def self.name\n    'Bar'\n  end\nend\n",
        );
        let source =
            "self::A = Class.new do\n  def foo\n    1\n  end\n  def foo\n    2\n  end\nend\n";
        assert_eq!(found(source), vec![(5, 3, msg("::A#foo", 2, 5))]);
    }

    #[test]
    fn distinguishes_class_new_blocks_returned_from_different_methods() {
        assert_none(
            "def build_foo\n  Class.new do\n    def name\n      'Foo'\n    end\n  end\nend\ndef build_bar\n  Class.new do\n    def name\n      'Bar'\n    end\n  end\nend\n",
        );
        assert_none(
            "def build_foo\n  Class.new do\n    def self.name\n      'Foo'\n    end\n  end\nend\ndef build_bar\n  Class.new do\n    def self.name\n      'Bar'\n    end\n  end\nend\n",
        );
        let source = "def build_klass\n  Class.new do\n    def name\n      1\n    end\n    def name\n      2\n    end\n  end\nend\n";
        assert_eq!(found(source), vec![(6, 5, msg("Object#name", 3, 6))]);
    }

    #[test]
    fn class_new_blocks_in_same_named_singleton_method() {
        // RuboCop keys an anonymous class returned from `def self.define` by
        // the method's receiver and name, so both blocks share one scope.
        let source = "class Foo\n  def self.define\n    Class.new(S) do\n      def y\n      end\n    end\n  end\nend\n\nclass Foo\n  def self.define\n    Class.new do\n      def y\n      end\n    end\n  end\nend\n";
        assert_eq!(
            found(source),
            vec![
                (11, 3, msg("Foo.define", 2, 11)),
                (13, 7, msg("Foo::Object#y", 4, 13)),
            ]
        );
        assert_none(
            "class Foo\n  def define\n    Class.new do\n      def y\n      end\n    end\n  end\nend\n\nclass Bar\n  def define\n    Class.new do\n      def y\n      end\n    end\n  end\nend\n",
        );
    }

    #[test]
    fn module_new_blocks_passed_as_arguments() {
        assert_none(
            "A.prepend(\n  Module.new { def foo; end }\n)\nB.prepend(\n  Module.new { def foo; end }\n)\n",
        );
        assert_none(
            "A&.prepend(\n  Module.new { def foo; end }\n)\nB&.prepend(\n  Module.new { def foo; end }\n)\n",
        );
        for dot in [".", "&."] {
            let source = format!(
                "A{dot}prepend(\n  Module.new do\n    def foo\n      x\n    end\n  end\n)\n\nA{dot}prepend(\n  Module.new do\n    def foo\n      y\n    end\n  end\n)\n"
            );
            assert_eq!(
                found(&source),
                vec![(11, 5, msg("Object#foo", 3, 11))],
                "{dot}"
            );
            assert_none(&format!(
                "A{dot}prepend(\n  Module.new do\n    def foo\n      x\n    end\n  end\n)\n\nB{dot}prepend(\n  Module.new do\n    def foo\n      y\n    end\n  end\n)\n"
            ));
        }
        assert_none(
            "stub_const('Foo',\n           Module.new { def self.underscore(_); end })\nstub_const('Bar',\n           Module.new { def self.underscore(_); end })\n",
        );
        assert_none(
            "stub_const('Foo',\n           Module.new { def underscore(_); end })\nstub_const('Bar',\n           Module.new { def underscore(_); end })\n",
        );
    }

    #[test]
    fn class_new_blocks_passed_to_named_receiver_calls() {
        for def in ["def perform", "def self.perform"] {
            assert_none(&format!(
                "T.cast(\n  Class.new(Base) do\n    {def}\n      1\n    end\n  end,\n  T.class_of(Base)\n)\n\nT.cast(\n  Class.new(Base) do\n    {def}\n      2\n    end\n  end,\n  T.class_of(Base)\n)\n"
            ));
        }
        assert_none(
            "T.cast(Class.new(Base) { def perform; 1; end }, T.class_of(Base)); T.cast(Class.new(Base) { def perform; 2; end }, T.class_of(Base))\n",
        );
    }

    #[test]
    fn class_new_blocks_inside_blocks() {
        for (first, second) in [
            ("foo do\n  a = bar", "foo do\n  b = baz"),
            ("foo do\n  _1", "bar do\n  _1"),
            ("foo do\n  it", "bar do\n  it"),
            ("let(:foo_class) do", "let(:bar_class) do"),
            ("describe 'something' do", "describe 'other' do"),
        ] {
            for def in ["def custom_validation", "def self.name"] {
                assert_none(&format!(
                    "{first}\n  Class.new do\n    {def}\n    end\n  end\nend\n\n{second}\n  Class.new do\n    {def}\n    end\n  end\nend\n"
                ));
            }
        }
        assert_none(
            "let(:foo_mod) do\n  Module.new do\n    def name\n      'Foo'\n    end\n  end\nend\n\nlet(:bar_mod) do\n  Module.new do\n    def name\n      'Bar'\n    end\n  end\nend\n",
        );
    }

    #[test]
    fn singleton_methods_in_separate_class_new_blocks() {
        assert_none(
            "def setup\n  @first = Class.new do\n    class << self\n      def name\n        'FIRST'\n      end\n    end\n  end\n\n  Class.new do\n    class << self\n      def name\n        'SECOND'\n      end\n    end\n  end\nend\n",
        );
    }

    #[test]
    fn flags_duplicates_inside_one_class_new_block_inside_a_block() {
        let source = "let(:klass) do\n  Class.new do\n    def foo\n      1\n    end\n    def foo\n      2\n    end\n  end\nend\n";
        assert_eq!(found(source), vec![(6, 5, msg("::Object#foo", 3, 6))]);
        let source = "let(:klass) do\n  Class.new do\n    def self.foo\n      1\n    end\n    def self.foo\n      2\n    end\n  end\nend\n";
        assert_eq!(found(source), vec![(6, 5, msg("::Object.foo", 3, 6))]);
    }

    #[test]
    fn module_new_blocks_with_chained_calls() {
        assert_none(
            "Module.new { def test; end }\n      .instance_method(:test)\n\nModule.new { def test; end }\n      .instance_method(:test)\n",
        );
        assert_none(
            "Module.new { def test(*one, **two, &block) = super(one, two, yield(block)) }\n      .instance_method(:test)\n      .parameters\n",
        );
    }

    #[test]
    fn class_new_blocks_inside_numbered_and_it_blocks() {
        for param in ["_1", "it"] {
            assert_none(&format!(
                "foo {{ Class.new({param}) {{ def name; end }} }}\n\nbar {{ Class.new({param}) {{ def name; end }} }}\n"
            ));
            assert_none(&format!(
                "foo {{ Class.new({param}) {{ def self.name; 'Foo'; end }} }}\n\nbar {{ Class.new({param}) {{ def self.name; 'Bar'; end }} }}\n"
            ));
            let source = format!(
                "foo {{ Class.new({param}) do\n    def foo\n      1\n    end\n    def foo\n      2\n    end\n  end\n}}\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 5, msg("Object#foo", 2, 5))],
                "{param}"
            );
            let source = format!(
                "foo {{ Class.new({param}) do\n    def self.foo\n      1\n    end\n    def self.foo\n      2\n    end\n  end\n}}\n"
            );
            assert_eq!(
                found(&source),
                vec![(5, 5, msg("Object.foo", 2, 5))],
                "{param}"
            );
        }
    }

    #[test]
    fn allows_one_redefinition_inside_ensure_and_rescue() {
        for branch in ["ensure", "rescue"] {
            let prefix = format!(
                "module FooTest\n  def make_save_always_fail\n    Foo.class_eval do\n      def failed_save\n        raise\n      end\n      alias_method :original_save, :save\n      alias_method :save, :failed_save\n    end\n\n    yield\n  {branch}\n    Foo.class_eval do\n      alias_method :save, :original_save\n"
            );
            assert_none(&format!("{prefix}    end\n  end\nend\n"));
            let source =
                format!("{prefix}      alias_method :save, :original_save\n    end\n  end\nend\n");
            assert_eq!(
                found(&source),
                vec![(15, 7, msg("FooTest::Foo#save", 14, 15))],
                "{branch}"
            );
        }
    }

    #[test]
    fn distinguishes_scopes_within_class_self() {
        assert_none(
            "class A\n  class << self\n    def foo\n    end\n\n    class B\n      def foo\n      end\n    end\n  end\nend\n",
        );
    }

    #[test]
    fn flags_deeply_nested_singleton_methods() {
        let source = "module A\n  module B\n    class C\n      class << self\n        def foo\n        end\n\n        def foo\n        end\n      end\n    end\n  end\nend\n";
        assert_eq!(found(source), vec![(8, 9, msg("A::B::C.foo", 5, 8))]);
    }

    #[test]
    fn reports_file_path_in_message() {
        let found = offenses("def something\nend\ndef something\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "Method `Object#something` is defined at both test.rb:1 and test.rb:3."
        );
    }

    #[test]
    fn flags_more_lookalike_duplicates() {
        // Endless defs, operator methods, reopened namespaced classes.
        assert_eq!(
            found("class A\n  def foo = 1\n  def foo = 2\nend\n"),
            vec![(3, 3, msg("A#foo", 2, 3))]
        );
        assert_eq!(
            found("class A\n  def ==(o); end\n  def ==(o); end\nend\n"),
            vec![(3, 3, msg("A#==", 2, 3))]
        );
        assert_eq!(
            found("class A::B\n  def x; end\nend\nclass A::B\n  def x; end\nend\n"),
            vec![(5, 3, msg("A::B#x", 2, 5))]
        );
        assert_eq!(
            found("class A\n  def x; end\n  attr_accessor :x, :y\nend\n"),
            vec![(3, 3, msg("A#x", 2, 3))]
        );
    }

    #[test]
    fn reports_one_offense_per_statement() {
        // RuboCop keeps one offense per range; the first one wins.
        assert_eq!(
            found("class A\n  attr_accessor :a\n  attr_accessor :a\nend\n"),
            vec![(3, 3, msg("A#a", 2, 3))]
        );
        assert_eq!(
            found("class A\n  attr_reader :b, :b, :b\nend\n"),
            vec![(2, 3, msg("A#b", 2, 2))]
        );
        assert_eq!(
            found("class A\n  def_delegators :@x, :y, :y, :y\nend\n"),
            vec![(2, 3, msg("A#y", 2, 2))]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            // Different classes, modules, or namespaces.
            "class A\n  def foo; end\nend\nclass B\n  def foo; end\nend\n",
            "module M\n  class A\n    def foo; end\n  end\nend\nclass A\n  def foo; end\nend\n",
            // Conditional (platform-specific) definitions.
            "if RUBY_VERSION > '3'\n  def foo; end\nelse\n  def foo; end\nend\n",
            "class A\n  def foo; end\n  unless defined?(bar)\n    def foo; end\n  end\nend\n",
            "class A\n  def foo; end\n  alias foo bar if x\nend\n",
            // Singleton receivers that are not constants or self.
            "def obj.foo; end\ndef obj.foo; end\n",
            // Unresolvable constant receivers.
            "def Foo.bar; end\ndef Foo.bar; end\n",
            // Struct and other blocks are unknown scopes.
            "Foo = Struct.new(:a) do\n  def x; end\n  def x; end\nend\n",
            "x = lambda do\n  def foo; end\n  def foo; end\nend\n",
            // `attr_*` with string names is ignored, as in RuboCop.
            "class A\n  def foo; end\n  attr_reader 'foo'\nend\n",
            // Getter and setter are different methods.
            "class A\n  def foo; end\n  def foo=(v); end\nend\n",
        ] {
            assert_none(source);
        }
    }

    // The walker's ancestors include typed children (constant path targets,
    // the call of a regexp match with named captures); the parser gem's AST,
    // which RuboCop scopes by, folds them into their parent. Expectations
    // checked against RuboCop 1.92.

    #[test]
    fn anonymous_class_in_a_constant_path_target_is_its_own_scope() {
        // The block is the `casgn`'s namespace, not its value: no constant
        // names it, and RuboCop keys it by its own location.
        assert_none("Class.new { def foo; end }::B = 1\nClass.new { def foo; end }::B = 2\n");
        assert_none("Class.new { def foo; end }::B ||= 1\nClass.new { def foo; end }::B ||= 2\n");
        assert_none("Class.new { def foo; end }::B += 1\nClass.new { def foo; end }::B &&= 2\n");
    }

    #[test]
    fn anonymous_class_matched_by_a_named_capture_regexp_is_unnamed() {
        // `/(?<a>x)/ =~ expr` is a `match_with_lvasgn` in RuboCop's AST, not
        // a call: the block's parent gives it no scope id.
        for class in ["Class", "Module"] {
            let col = 21 + class.len() as u32;
            assert_eq!(
                found(&format!(
                    "/(?<a>x)/ =~ {class}.new {{ def foo; end }}\n/(?<b>x)/ =~ {class}.new {{ def foo; end }}\n"
                )),
                vec![(2, col, msg("Object#foo", 1, 2))],
                "{class}"
            );
        }
    }

    #[test]
    fn honors_directives() {
        assert_none(
            "def foo; end\n# rubocop:disable Lint/DuplicateMethods\ndef foo; end\n# rubocop:enable Lint/DuplicateMethods\n",
        );
    }
}
