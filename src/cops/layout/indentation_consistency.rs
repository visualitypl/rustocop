//! `Layout/IndentationConsistency` — fixed `normal` style.
//!
//! RuboCop checks the children of every `begin` and `kwbegin` node. In Prism
//! those children are the statements of a `StatementsNode`: an implicit body,
//! a `begin ... end` body, parentheses and interpolation all hold one. A list
//! with a single statement never reports (its own column is the base), so only
//! lists of two or more statements are inspected.
//!
//! Bare access modifiers are skipped. When the first statement is one, its
//! column is the base, unless it is not indented deeper than the parser parent
//! of the `begin` (an outdented modifier, see `Layout/AccessModifierIndentation`);
//! then the first other statement's column is the base. Columns are display
//! columns, and only a statement that begins its line and starts on a later
//! line than the previous statement is checked.
use crate::{
    cops::{
        Cop, NodeContext,
        layout::alignment::{Alignment, INDENT, display_column, starts_line},
        layout::indentation_width::bare_access,
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct IndentationConsistency;

const MSG: &str = "Inconsistent indentation detected.";

impl Cop for IndentationConsistency {
    fn name(&self) -> &'static str {
        "Layout/IndentationConsistency"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_statements_node().is_none() {
            return;
        }
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let bad = bad_items(node, &ancestors, file);
        if bad.is_empty() {
            return;
        }
        // RuboCop reports, but does not correct, an offense inside the range
        // of an earlier offense of this cop: the outer realignment already
        // rewrites those lines. Earlier offenses that contain this list are
        // misaligned statements of enclosing lists.
        let nested = cx.corrections_requested() && inside_bad_item(node, &ancestors, file);
        let root = ancestors.last().copied().unwrap_or(node);
        let keyword = flat_member_keyword(node, &ancestors, file);
        for (item, delta) in bad {
            let location = item.location();
            let alignment = Alignment {
                expression: location.start_offset()..location.end_offset(),
                delta,
            };
            let (line, col) = file.line_col(location.start_offset());
            out.push(Offense {
                correction: (cx.corrections_requested()
                    && !nested
                    && !keyword.is_some_and(|keyword| {
                        defers_to_width(file, location.start_offset(), delta, keyword)
                    }))
                .then(|| alignment.correction(file, root))
                .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: MSG.to_owned(),
                line,
                col,
                end_line: file.last_line_of(location),
            });
        }
    }
}

/// The display column of the keyword of the class, module or singleton class
/// whose direct body is `list`, when its first statement is on a later line
/// than the keyword. `Layout/IndentationWidth` (default-on) then realigns
/// every member of the body to the keyword column plus two.
fn flat_member_keyword<'pr>(
    list: &Node<'pr>,
    ancestors: &[&Node<'pr>],
    file: &SourceFile,
) -> Option<usize> {
    let owner = ancestors.first()?;
    let keyword = if let Some(n) = owner.as_class_node() {
        n.class_keyword_loc()
    } else if let Some(n) = owner.as_module_node() {
        n.module_keyword_loc()
    } else {
        owner.as_singleton_class_node()?.class_keyword_loc()
    };
    let first = list.as_statements_node()?.body().iter().next()?;
    (file.line_of(first.location().start_offset()) != file.line_of(keyword.start_offset()))
        .then(|| display_column(file, keyword.start_offset()))
}

/// Whether the realignment of a misaligned member at `start` is withheld
/// (its finding stays): the base, `delta` columns away, is not indented past
/// the `keyword` column at all, while the member already sits at the keyword
/// column plus two. IndentationWidth realigns that flat base to the member's
/// column in the same pass; moving the member to the base instead would swap
/// the two, keep the body inconsistent and displace other cops' rewrites of
/// the member (`Style/BisectedAttrAccessor`'s, which start at its line).
/// A base indented past the keyword, the case the pinned examples correct,
/// keeps the upstream edit.
fn defers_to_width(file: &SourceFile, start: usize, delta: i64, keyword: usize) -> bool {
    let column = display_column(file, start) as i64;
    let keyword = keyword as i64;
    column == keyword + INDENT as i64 && column + delta <= keyword
}

/// `Alignment#check_alignment` over the non-modifier statements of `list`
/// (a `StatementsNode`), whose ancestors are `ancestors`, innermost first.
/// Returns each misaligned statement with its column delta.
fn bad_items<'pr>(
    list: &Node<'pr>,
    ancestors: &[&Node<'pr>],
    file: &SourceFile,
) -> Vec<(Node<'pr>, i64)> {
    let body = list.as_statements_node().unwrap().body();
    if body.len() < 2 {
        return Vec::new();
    }
    let modifier = |child: &Node<'pr>| {
        bare_access(
            child,
            std::iter::once(list).chain(ancestors.iter().copied()),
        )
    };
    let mut items = body.iter().filter(|child| !modifier(child)).peekable();
    let Some(first) = items.peek() else {
        return Vec::new();
    };
    let base = base_column(&body.iter().next().unwrap(), &modifier, ancestors, file)
        .unwrap_or_else(|| display_column(file, first.location().start_offset()));
    let mut bad = Vec::new();
    let mut previous_line = 0;
    for item in items {
        let start = item.location().start_offset();
        let line = file.line_of(start);
        if line > previous_line && starts_line(file, start) {
            let delta = base as i64 - display_column(file, start) as i64;
            if delta != 0 {
                bad.push((item, delta));
            }
        }
        previous_line = line;
    }
    bad
}

/// `base_column_for_normal_style`: the column of a leading bare access
/// modifier, unless it is not deeper than the `begin`'s parser parent.
fn base_column<'pr>(
    first: &Node<'pr>,
    modifier: &impl Fn(&Node<'pr>) -> bool,
    ancestors: &[&Node<'pr>],
    file: &SourceFile,
) -> Option<usize> {
    if !modifier(first) {
        return None;
    }
    let access = display_column(file, first.location().start_offset());
    match parser_parent_start(ancestors) {
        None => Some(access),
        Some(parent) => (access > display_column(file, parent)).then_some(access),
    }
}

/// The start of the parser-gem parent of the `begin`/`kwbegin` node that a
/// statement list stands for; `None` when that node is the root. `ancestors`
/// are the list's Prism ancestors, innermost first.
fn parser_parent_start(ancestors: &[&Node<'_>]) -> Option<usize> {
    let (owner, rest) = ancestors.split_first()?;
    let start = |node: &Node<'_>| node.location().start_offset();
    let after =
        |rest: &[&Node<'_>], node: &Node<'_>| rest.first().map_or(start(node), |n| start(n));
    match owner {
        Node::ProgramNode { .. } => return None,
        // The list is the kwbegin, parenthesized begin or interpolation
        // itself: its parent is further up.
        Node::BeginNode { .. } if kwbegin(owner) => {}
        Node::ParenthesesNode { .. } | Node::EmbeddedStatementsNode { .. } => {}
        // Parser's block node starts at its call; an else body belongs to the
        // enclosing conditional.
        Node::BlockNode { .. } | Node::ElseNode { .. } => return Some(after(rest, owner)),
        _ => return Some(start(owner)),
    }
    let mut rest = rest;
    while let Some((node, tail)) = rest.split_first() {
        match node {
            Node::ProgramNode { .. } => return None,
            // A single statement is not wrapped in a parser `begin`; several
            // are, and that implicit `begin` starts at the first one.
            Node::StatementsNode { .. } => {
                let body = node.as_statements_node().unwrap().body();
                if body.len() >= 2 {
                    return body.iter().next().map(|n| start(&n));
                }
            }
            Node::BlockNode { .. } | Node::ElseNode { .. } => return Some(after(tail, node)),
            _ => return Some(start(node)),
        }
        rest = tail;
    }
    None
}

/// A `begin ... end` whose statements are the kwbegin's own children.
fn kwbegin(node: &Node<'_>) -> bool {
    node.as_begin_node().is_some_and(|begin| {
        begin.begin_keyword_loc().is_some()
            && begin.rescue_clause().is_none()
            && begin.else_clause().is_none()
            && begin.ensure_clause().is_none()
    })
}

/// Whether `list` lies inside a misaligned statement of an enclosing list.
fn inside_bad_item<'pr>(list: &Node<'pr>, ancestors: &[&Node<'pr>], file: &SourceFile) -> bool {
    let mut child = list;
    for (index, parent) in ancestors.iter().enumerate() {
        if parent.as_statements_node().is_some() {
            let location = child.location();
            if bad_items(parent, &ancestors[index + 1..], file)
                .iter()
                .any(|(item, _)| {
                    let item = item.location();
                    item.start_offset() == location.start_offset()
                        && item.end_offset() == location.end_offset()
                })
            {
                return true;
            }
        }
        child = parent;
    }
    false
}

#[cfg(test)]
mod tests;
