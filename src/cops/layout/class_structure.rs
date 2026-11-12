//! `Layout/ClassStructure` (opt-in) — the definitions of a class body follow
//! the fixed expected order.
//!
//! Port of upstream's `on_class`/`on_sclass` over the parser AST that RuboCop
//! builds from Prism, at the pinned defaults with no knob: `Categories` is
//! `module_inclusion: [include, prepend, extend]` and `ExpectedOrder` is
//! `module_inclusion, constants, public_class_methods, initializer,
//! public_methods, protected_methods, private_class_methods, private_methods`.
//!
//! The class elements are the class's statement list (a parser `begin`: two
//! or more statements, or a sole parenthesized one), with every `begin ...
//! end` block (`kwbegin`) flattened into it recursively; a sole other
//! statement is the only element, and a body with `rescue`/`ensure` is one
//! `rescue`/`ensure` element. Each element is classified as upstream:
//!
//! - a send (no safe navigation; a block argument is an argument) by its
//!   method name, `include`/`prepend`/`extend` as `module_inclusion`: a def
//!   modifier (`private def`, `protected memoize def`, nil receivers down to
//!   the def) gives `<name>_methods` or, for `*_class_method`, `<name>s`;
//!   otherwise `<visibility>_<category or name>` when that is in the order,
//!   else the category or name itself;
//! - a literal block (never a numbered or `it` block) as its send, whose
//!   visibility is always public (its siblings are the block's);
//! - `def initialize` as `initializer`, another `def` as
//!   `<visibility>_methods`: the last right sibling `private`/`protected`/
//!   `public` naming it by symbol, else the nearest left bare modifier, else
//!   public;
//! - `def self.x`/`def obj.x` as `private_class_methods` or
//!   `public_class_methods` after the last right sibling
//!   `private_class_method`/`public_class_method` naming it, else public (a
//!   bare `private` never reaches it);
//! - a constant assignment (`X =`, `A::X =`) as `constants`, ignored when an
//!   unqualified one is named by symbol in a `private_constant` send of the
//!   same statement list;
//! - anything else (nested classes, modules, singleton classes, other
//!   assignments and nodes) as its node type, which is not in the order.
//!
//! Siblings are those of the element's own statement list, so a section or
//! `private_constant` outside a `begin` block does not reach into it.
//! Elements not in the order are skipped. An element is reported when its
//! category is expected before the highest-priority category seen so far,
//! naming that category, unless it continues the previous element's category
//! (a group is reported once, on its first element).
//!
//! Detection-only: upstream's reorder is `SafeAutoCorrect: false` (moving
//! constants, macros and methods changes lookup, hooks, order and
//! visibility), so no edit is ever produced. Its dynamic-constant and
//! bare-visibility barriers only limit those moves and never change a
//! finding.
//!
//! The cop is opt-in as upstream (`Enabled: false`): only its exact name in
//! `--only` or its own `enabled = true` enables it, never a department
//! selection, so the accepted `Layout` compositions keep their results.
use ruby_prism::{CallNode, DefNode, Node};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};

pub struct ClassStructure;

/// The pinned `ExpectedOrder`, highest priority first.
const EXPECTED_ORDER: [&str; 8] = [
    "module_inclusion",
    "constants",
    "public_class_methods",
    "initializer",
    "public_methods",
    "protected_methods",
    "private_class_methods",
    "private_methods",
];

/// The pinned `Categories`: macro names mapped to a category.
const CATEGORIES: [(&str, &[&[u8]]); 1] =
    [("module_inclusion", &[b"include", b"prepend", b"extend"])];

const VISIBILITY_SCOPES: [&[u8]; 3] = [b"private", b"protected", b"public"];

const CLASS_METHOD_MODIFIERS: [&[u8]; 2] = [b"private_class_method", b"public_class_method"];

impl Cop for ClassStructure {
    fn name(&self) -> &'static str {
        "Layout/ClassStructure"
    }

    /// Opt-in as upstream: a department selection does not enable it.
    fn department_selection_enables(&self) -> bool {
        false
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let body = if let Some(class) = node.as_class_node() {
            class.body()
        } else if let Some(sclass) = node.as_singleton_class_node() {
            sclass.body()
        } else {
            return;
        };
        let mut lists = Vec::new();
        let mut elements = Vec::new();
        if let Some(statements) = body.as_ref().and_then(Node::as_statements_node) {
            let statements: Vec<_> = statements.body().iter().collect();
            if let [sole] = statements.as_slice() {
                // A sole statement is the only element (and cannot be out of
                // order) unless it is a parser `begin` (parentheses) or a
                // `kwbegin`.
                let inner = match sole.as_parentheses_node() {
                    Some(parentheses) => Some(statement_list(parentheses.body())),
                    None => kwbegin_statements(sole),
                };
                if let Some(inner) = inner {
                    flatten(inner, &mut lists, &mut elements);
                }
            } else {
                flatten(statements, &mut lists, &mut elements);
            }
        }
        // Otherwise the body is empty or a `rescue`/`ensure`: no elements.
        let mut max_index: Option<usize> = None;
        let mut previous_category: Option<String> = None;
        for (list, index) in elements {
            let siblings: &[Node<'pr>] = &lists[list];
            let element = &siblings[index];
            let Some(category) = classify(element, &siblings[..index], &siblings[index + 1..])
            else {
                continue;
            };
            let Some(order) = EXPECTED_ORDER.iter().position(|&c| c == category) else {
                continue;
            };
            if private_constant(element, siblings) {
                continue;
            }
            if let Some(max) = max_index
                && order < max
                && previous_category.as_deref() != Some(category.as_str())
            {
                out.push(self.offense(element, &category, EXPECTED_ORDER[max], file));
            }
            max_index = max_index.max(Some(order));
            previous_category = Some(category);
        }
    }
}

impl ClassStructure {
    fn offense(
        &self,
        element: &Node<'_>,
        category: &str,
        previous: &str,
        file: &SourceFile,
    ) -> Offense {
        let start = element.location().start_offset();
        // A literal block belongs to the element; a send stops at its
        // arguments (a heredoc body is never part of an expression).
        let end = if element
            .as_call_node()
            .and_then(|call| call.block())
            .is_some_and(|block| block.as_block_node().is_some())
        {
            element.location().end_offset()
        } else {
            ast::source_end(element)
        };
        let (line, col) = file.line_col(start);
        Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("`{category}` is supposed to appear before `{previous}`."),
            line,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            col,
        }
    }
}

fn statement_list<'pr>(body: Option<Node<'pr>>) -> Vec<Node<'pr>> {
    match body {
        Some(body) => match body.as_statements_node() {
            Some(statements) => statements.body().iter().collect(),
            None => vec![body],
        },
        None => Vec::new(),
    }
}

/// Record the elements of one statement list in source order, as
/// `(list, position)`; each `kwbegin` in it is replaced by its own elements,
/// whose siblings are those of its own list.
fn flatten<'pr>(
    statements: Vec<Node<'pr>>,
    lists: &mut Vec<Vec<Node<'pr>>>,
    elements: &mut Vec<(usize, usize)>,
) {
    let list = lists.len();
    let nested: Vec<_> = statements.iter().map(kwbegin_statements).collect();
    lists.push(statements);
    for (index, nested) in nested.into_iter().enumerate() {
        match nested {
            Some(inner) => flatten(inner, lists, elements),
            None => elements.push((list, index)),
        }
    }
}

/// The children of a `begin ... end` block (`kwbegin`), or `None` for
/// another node. With `rescue`/`ensure` its only child is that clause, a
/// single unordered element, so it contributes nothing.
fn kwbegin_statements<'pr>(node: &Node<'pr>) -> Option<Vec<Node<'pr>>> {
    let begin = node.as_begin_node()?;
    begin.begin_keyword_loc()?;
    if begin.rescue_clause().is_some() || begin.ensure_clause().is_some() {
        return Some(Vec::new());
    }
    Some(statement_list(begin.statements().map(|s| s.as_node())))
}

/// The element's category, or `None` for a node type never in the order.
fn classify(node: &Node<'_>, left: &[Node<'_>], right: &[Node<'_>]) -> Option<String> {
    if let Some(call) = node.as_call_node() {
        if call.is_safe_navigation() {
            return None;
        }
        return match call.block() {
            Some(block) => match block.as_block_node() {
                Some(block)
                    if matches!(
                        block.parameters(),
                        Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
                    ) =>
                {
                    None
                }
                // The block's send has no siblings of its own: public.
                Some(_) => Some(send_category(&call, b"public")),
                None => Some(send_category(&call, visibility_section(left))),
            },
            None => Some(send_category(&call, visibility_section(left))),
        };
    }
    if let Some(def) = node.as_def_node() {
        let name = def.name().as_slice();
        let category = if def.receiver().is_some() {
            class_method_category(&def, right)
        } else if name == b"initialize" {
            "initializer".to_owned()
        } else {
            let visibility =
                inline_visibility(name, right).unwrap_or_else(|| visibility_section(left));
            format!("{}_methods", String::from_utf8_lossy(visibility))
        };
        return Some(find_category(category.as_bytes()).map_or(category, str::to_owned));
    }
    if node.as_constant_write_node().is_some() || node.as_constant_path_write_node().is_some() {
        return Some(
            find_category(b"constants")
                .unwrap_or("constants")
                .to_owned(),
        );
    }
    None
}

/// `find_send_node_category`.
fn send_category(call: &CallNode<'_>, visibility: &[u8]) -> String {
    let name = call.name().as_slice();
    let key = find_category(name)
        .map_or_else(|| String::from_utf8_lossy(name).into_owned(), str::to_owned);
    let visibility_key = if def_modifier(call) {
        let name = String::from_utf8_lossy(name);
        if name.ends_with("_class_method") {
            format!("{name}s")
        } else {
            format!("{name}_methods")
        }
    } else {
        format!("{}_{key}", String::from_utf8_lossy(visibility))
    };
    if EXPECTED_ORDER.contains(&visibility_key.as_str()) {
        visibility_key
    } else {
        key
    }
}

fn find_category(name: &[u8]) -> Option<&'static str> {
    CATEGORIES
        .iter()
        .find(|(_, names)| names.contains(&name))
        .map(|&(category, _)| category)
}

/// rubocop-ast's `def_modifier?`: a send with no receiver whose first
/// argument is a `def`/`defs`, or another such send.
fn def_modifier(call: &CallNode<'_>) -> bool {
    if call.receiver().is_some() {
        return false;
    }
    match ast::call_arguments(call).first() {
        Some(Node::DefNode { .. }) => true,
        Some(argument) => argument
            .as_call_node()
            .filter(|inner| is_send(inner))
            .is_some_and(|inner| def_modifier(&inner)),
        None => false,
    }
}

/// A parser `send`: neither safe navigation nor a literal block.
fn is_send(call: &CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call
            .block()
            .is_none_or(|block| block.as_block_node().is_none())
}

/// The nearest left bare `private`/`protected`/`public`, else public.
fn visibility_section(left: &[Node<'_>]) -> &'static [u8] {
    left.iter()
        .rev()
        .find_map(|sibling| {
            let call = sibling.as_call_node()?;
            if call.receiver().is_some() || call.arguments().is_some() || call.block().is_some() {
                return None;
            }
            VISIBILITY_SCOPES
                .into_iter()
                .find(|&scope| scope == call.name().as_slice())
        })
        .unwrap_or(b"public")
}

/// The last right sibling send with no receiver named one of `methods`
/// whose arguments include the symbol `name`.
fn naming_modifier(
    name: &[u8],
    right: &[Node<'_>],
    methods: &[&'static [u8]],
) -> Option<&'static [u8]> {
    right.iter().rev().find_map(|sibling| {
        let call = sibling.as_call_node()?;
        if call.receiver().is_some() || !is_send(&call) {
            return None;
        }
        let method = methods
            .iter()
            .copied()
            .find(|&m| m == call.name().as_slice())?;
        ast::call_arguments(&call)
            .iter()
            .any(|argument| {
                argument
                    .as_symbol_node()
                    .is_some_and(|symbol| symbol.unescaped() == name)
            })
            .then_some(method)
    })
}

fn inline_visibility(name: &[u8], right: &[Node<'_>]) -> Option<&'static [u8]> {
    naming_modifier(name, right, &VISIBILITY_SCOPES)
}

fn class_method_category(def: &DefNode<'_>, right: &[Node<'_>]) -> String {
    match naming_modifier(def.name().as_slice(), right, &CLASS_METHOD_MODIFIERS) {
        Some(modifier) => format!("{}s", String::from_utf8_lossy(modifier)),
        None => "public_class_methods".to_owned(),
    }
}

/// An unqualified constant named by symbol in a `private_constant` send
/// (any receiver) of the same statement list.
fn private_constant(element: &Node<'_>, siblings: &[Node<'_>]) -> bool {
    let Some(constant) = element.as_constant_write_node() else {
        return false;
    };
    let name = constant.name().as_slice();
    siblings.iter().any(|sibling| {
        sibling.as_call_node().is_some_and(|call| {
            is_send(&call)
                && call.name().as_slice() == b"private_constant"
                && ast::call_arguments(&call).iter().any(|argument| {
                    argument
                        .as_symbol_node()
                        .is_some_and(|symbol| symbol.unescaped() == name)
                })
        })
    })
}

#[cfg(test)]
mod tests;
