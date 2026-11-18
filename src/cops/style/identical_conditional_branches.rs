//! `Style/IdenticalConditionalBranches` — common branch heads and tails.
//! Unsafe correction: hoisting can reorder effects and change return values.

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense, single_line},
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};
pub struct IdenticalConditionalBranches;

#[cfg(test)]
thread_local! {
    static CHILD_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl Cop for IdenticalConditionalBranches {
    fn name(&self) -> &'static str {
        "Style/IdenticalConditionalBranches"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((condition, branches)) = branches(node) else {
            return;
        };
        let Some(edges) = branches.into_iter().map(edges).collect::<Option<Vec<_>>>() else {
            return;
        };
        let tails: Vec<_> = edges.iter().map(|b| &b.1).collect();
        // Upstream's correctors remove each reported expression; the first
        // also inserts it before (heads) or after (tails) the conditional.
        // One correction per offense: the first offense carries the group.
        let correctable = cx.corrections_requested()
            && !node
                .as_if_node()
                .map(|n| n.then_keyword_loc())
                .or_else(|| node.as_unless_node().map(|n| n.then_keyword_loc()))
                .is_some_and(|then| then.is_some());
        let emit = |expressions: &[&Node<'_>], after: bool, out: &mut Vec<Offense>| {
            let mut seen = file.identical_branch_edges().lock().unwrap();
            let first = out.len();
            let mut edits = Vec::new();
            let mut lead = None;
            for expression in expressions {
                let loc = expression.location();
                if !seen.insert((loc.start_offset(), loc.end_offset())) {
                    continue;
                }
                if correctable {
                    edits.push((removal_range(expression, file), String::new()));
                    lead.get_or_insert(*expression);
                }
                let (line, col) = file.line_col(loc.start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Move `{}` out of the conditional.",
                        single_line(loc.as_slice())
                    ),
                    line,
                    col,
                    end_line: file.last_line_of(loc),
                    correction: None,
                });
            }
            if let (Some(offense), Some(expression)) = (out.get_mut(first), lead) {
                edits.extend(insertion(node, cx.parent(), expression, after, file));
                offense.correction = Correction::from_edits(&file.bytes, edits);
            }
        };
        if duplicated(&tails, condition.as_ref(), file) {
            emit(&tails, true, out);
        }
        // Hoisting a sole branch head can discard a consumed return value.
        if edges.iter().any(|b| b.2) && last_child_of_parent(node, cx) {
            return;
        }
        let heads: Vec<_> = edges.iter().map(|b| &b.0).collect();
        if !duplicated(&heads, condition.as_ref(), file) {
            return;
        }
        if let Some(assignment) = assignment(heads[0])
            && condition.as_ref().and_then(assignable_condition).as_deref()
                == Some(assignment.name.as_slice())
        {
            return;
        }
        emit(&heads, false, out);
    }
}

/// Upstream's `removal_range`: the expression's whole lines when it is alone
/// on them, else the expression with an adjacent `;` separator, else with
/// the spaces after it.
fn removal_range(expression: &Node<'_>, file: &SourceFile) -> std::ops::Range<usize> {
    let loc = expression.location();
    let range = loc.start_offset()..loc.end_offset();
    let lines = super::correction::whole_lines(file, range.clone());
    let strip = |b: &u8| b.is_ascii_whitespace() || *b == 0 || *b == 0x0b;
    let text = &file.bytes[lines.clone()];
    let start = text.iter().position(|b| !strip(b)).unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|b| !strip(b))
        .map_or(start, |i| i + 1);
    if &text[start..end] == loc.as_slice() {
        return lines;
    }
    let bytes = &file.bytes;
    let blank = |b: &u8| matches!(b, b' ' | b'\t');
    let mut after = range.end;
    while bytes.get(after).is_some_and(blank) {
        after += 1;
    }
    if bytes.get(after) == Some(&b';') {
        after += 1;
        while bytes.get(after).is_some_and(blank) {
            after += 1;
        }
        return range.start..after;
    }
    let mut before = range.start;
    while before > 0 && blank(&bytes[before - 1]) {
        before -= 1;
    }
    if before > 0 && bytes[before - 1] == b';' {
        before -= 1;
        while before > 0 && blank(&bytes[before - 1]) {
            before -= 1;
        }
        return before..range.end;
    }
    range.start..crate::offense::space_right_blank(bytes, range.end)
}

/// `correct_assignment` / `correct_no_assignment`.
fn insertion(
    node: &Node<'_>,
    parent: Option<&Node<'_>>,
    expression: &Node<'_>,
    after: bool,
    file: &SourceFile,
) -> Vec<(std::ops::Range<usize>, String)> {
    let text = String::from_utf8_lossy(expression.location().as_slice()).into_owned();
    let column = |offset: usize| " ".repeat(file.line_col(offset).1 as usize - 1);
    let (start, end) = (node.location().start_offset(), node.location().end_offset());
    match parent.filter(|p| super::and_or::is_assignment(p)) {
        Some(parent) => {
            let parent_start = parent.location().start_offset();
            let indentation = column(parent_start);
            if after {
                let assignment =
                    String::from_utf8_lossy(&file.bytes[parent_start..start]).into_owned();
                vec![
                    (parent_start..start, String::new()),
                    (end..end, format!("\n{indentation}{assignment}{text}")),
                ]
            } else {
                vec![(parent_start..parent_start, format!("{text}\n{indentation}"))]
            }
        }
        None => {
            let indentation = column(start);
            if after {
                vec![(end..end, format!("\n{indentation}{text}"))]
            } else {
                vec![(start..start, format!("{text}\n{indentation}"))]
            }
        }
    }
}

fn branches<'pr>(node: &Node<'pr>) -> Option<(Option<Node<'pr>>, Vec<StatementsNode<'pr>>)> {
    if let Some(n) = node.as_if_node() {
        if n.if_keyword_loc().is_some_and(|k| k.as_slice() == b"elsif") {
            return None;
        }
        let mut branches = Vec::new();
        expand_if(node, &mut branches)?;
        return Some((Some(n.predicate()), branches));
    }
    if let Some(n) = node.as_unless_node() {
        // RuboCop orders branches by truth value, not source order.
        let mut branches = vec![n.else_clause()?.statements()?];
        expand_else(n.statements()?, &mut branches)?;
        return Some((Some(n.predicate()), branches));
    }
    if let Some(n) = node.as_case_node() {
        let mut branches = n
            .conditions()
            .iter()
            .map(|b| b.as_when_node()?.statements())
            .collect::<Option<Vec<_>>>()?;
        branches.push(n.else_clause()?.statements()?);
        return Some((n.predicate(), branches));
    }
    if let Some(n) = node.as_case_match_node() {
        let mut branches = n
            .conditions()
            .iter()
            .map(|b| b.as_in_node()?.statements())
            .collect::<Option<Vec<_>>>()?;
        branches.push(n.else_clause()?.statements()?);
        return Some((n.predicate(), branches));
    }
    None
}

// Upstream expands any if-shaped else, including an explicit nested if or
// unless and modifiers (whose absent branch makes the whole chain incomplete).
fn expand_else<'pr>(
    body: StatementsNode<'pr>,
    branches: &mut Vec<StatementsNode<'pr>>,
) -> Option<()> {
    if body.body().len() == 1 {
        let node = body.body().first()?;
        if node.as_if_node().is_some() {
            return expand_if(&node, branches);
        }
        if let Some(n) = node.as_unless_node() {
            branches.push(n.else_clause()?.statements()?);
            return expand_else(n.statements()?, branches);
        }
    }
    branches.push(body);
    Some(())
}
fn expand_if<'pr>(node: &Node<'pr>, branches: &mut Vec<StatementsNode<'pr>>) -> Option<()> {
    let n = node.as_if_node()?;
    branches.push(n.statements()?);
    let next = n.subsequent()?;
    if let Some(e) = next.as_else_node() {
        expand_else(e.statements()?, branches)
    } else {
        expand_if(&next, branches)
    }
}

// Parenthesized begin nodes are real branch edges in the reference AST. Peel
// only a branch's begin wrapper, not arbitrary nested control flow or kwbegin.
fn edges(body: StatementsNode<'_>) -> Option<(Node<'_>, Node<'_>, bool)> {
    let list = body.body();
    if list.len() != 1 {
        return Some((list.first()?, list.last()?, false));
    }
    let node = list.first()?;
    if let Some(p) = node.as_parentheses_node() {
        let body = p.body()?;
        let list = body.as_statements_node()?.body();
        return Some((list.first()?, list.last()?, list.len() <= 1));
    }
    Some((list.first()?, node, true))
}

fn duplicated(expressions: &[&Node<'_>], condition: Option<&Node<'_>>, file: &SourceFile) -> bool {
    let Some(first) = expressions.first() else {
        return false;
    };
    // Detection uses the same parser-value equality as CaseLikeIf (including
    // evaluated __LINE__, numeric values and explicit ! sends), never a proof
    // that hoisting preserves Ruby behavior.
    let key = ast::fingerprint_for_case_subject(first, file);
    if !expressions
        .iter()
        .skip(1)
        .all(|n| ast::fingerprint_for_case_subject(n, file) == key)
    {
        return false;
    }
    let Some(assignment) = assignment(first) else {
        return true;
    };
    !condition.is_some_and(|condition| {
        condition_children(condition)
            .iter()
            .any(|n| ast::is_variable(n) && n.location().as_slice() == assignment.child)
    })
}

fn assignable_condition(node: &Node<'_>) -> Option<Vec<u8>> {
    if let Some(call) = node.as_call_node() {
        if call.block().is_some_and(|b| b.as_block_node().is_some()) {
            return None;
        }
        return Some(call.receiver().map_or_else(
            || node.location().as_slice().to_vec(),
            |r| r.location().as_slice().to_vec(),
        ));
    }
    ast::is_variable(node).then(|| node.location().as_slice().to_vec())
}

struct Assignment {
    name: Vec<u8>,
    child: Vec<u8>,
}
fn assignment(node: &Node<'_>) -> Option<Assignment> {
    macro_rules! plain {
        ($($access:ident),*) => { $(if let Some(n) = node.$access() {
            return Some(Assignment { name: n.name().as_slice().to_vec(), child: n.value().location().as_slice().to_vec() });
        })* };
    }
    plain!(
        as_local_variable_write_node,
        as_instance_variable_write_node,
        as_class_variable_write_node,
        as_global_variable_write_node
    );
    if let Some(n) = node.as_constant_write_node() {
        return Some(Assignment {
            name: Vec::new(),
            child: n.value().location().as_slice().to_vec(),
        });
    }
    macro_rules! compound {
        ($($access:ident),*) => { $(if let Some(n) = node.$access() {
            return Some(Assignment { name: Vec::new(), child: n.name().as_slice().to_vec() });
        })* };
    }
    compound!(
        as_local_variable_and_write_node,
        as_local_variable_or_write_node,
        as_local_variable_operator_write_node,
        as_instance_variable_and_write_node,
        as_instance_variable_or_write_node,
        as_instance_variable_operator_write_node,
        as_class_variable_and_write_node,
        as_class_variable_or_write_node,
        as_class_variable_operator_write_node,
        as_global_variable_and_write_node,
        as_global_variable_or_write_node,
        as_global_variable_operator_write_node,
        as_constant_and_write_node,
        as_constant_or_write_node,
        as_constant_operator_write_node
    );
    if let Some(call) = node.as_call_node().filter(|c| c.equal_loc().is_some()) {
        let receiver = call
            .receiver()
            .map(|r| r.location().as_slice().to_vec())
            .unwrap_or_default();
        return Some(Assignment {
            name: receiver.clone(),
            child: receiver,
        });
    }
    if let Some(n) = node.as_constant_path_write_node() {
        return Some(Assignment {
            name: Vec::new(),
            child: n.target().parent().map_or_else(
                || n.value().location().as_slice().to_vec(),
                |p| p.location().as_slice().to_vec(),
            ),
        });
    }
    // These reference assignment nodes have a composite first child, not a
    // bare variable. Neither direct-variable guard can match that child.
    if matches!(
        node,
        Node::CallAndWriteNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::CallOperatorWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::IndexOperatorWriteNode { .. }
            | Node::MultiWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::ConstantPathOperatorWriteNode { .. }
    ) {
        return Some(Assignment {
            name: Vec::new(),
            child: Vec::new(),
        });
    }
    None
}

// Only direct variable children guard an assignment, not nested reads. Model
// synthetic block/getter/rescue parents instead of exposing Prism-only edges.
fn condition_children<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    fn sole(body: Option<StatementsNode<'_>>) -> Option<Node<'_>> {
        let body = body?;
        (body.body().len() == 1).then(|| body.body().first().unwrap())
    }
    if let Some(c) = node.as_call_node()
        && let Some(b) = c.block().and_then(|b| b.as_block_node())
    {
        return b
            .body()
            .and_then(|n| {
                if let Some(s) = n.as_statements_node() {
                    sole(Some(s))
                } else {
                    Some(n)
                }
            })
            .into_iter()
            .collect();
    }
    macro_rules! composite_assignment {
        ($($access:ident),*) => { $(if let Some(n) = node.$access() {
            return vec![n.value()];
        })* };
    }
    composite_assignment!(
        as_call_and_write_node,
        as_call_or_write_node,
        as_call_operator_write_node,
        as_index_and_write_node,
        as_index_or_write_node,
        as_index_operator_write_node,
        as_constant_path_and_write_node,
        as_constant_path_or_write_node,
        as_constant_path_operator_write_node
    );
    if let Some(n) = node.as_constant_path_write_node() {
        return n
            .target()
            .parent()
            .into_iter()
            .chain(Some(n.value()))
            .collect();
    }
    if let Some(n) = node.as_match_write_node() {
        return children(&n.call().as_node());
    }
    if let Some(n) = node.as_parentheses_node() {
        return n
            .body()
            .and_then(|b| b.as_statements_node())
            .map_or_else(Vec::new, |s| s.body().iter().collect());
    }
    if let Some(n) = node.as_begin_node() {
        if n.rescue_clause().is_some() || n.ensure_clause().is_some() {
            return Vec::new();
        }
        return n
            .statements()
            .map_or_else(Vec::new, |s| s.body().iter().collect());
    }
    if let Some(n) = node.as_if_node() {
        let other = n
            .subsequent()
            .and_then(|n| n.as_else_node())
            .and_then(|n| sole(n.statements()));
        return Some(n.predicate())
            .into_iter()
            .chain(sole(n.statements()))
            .chain(other)
            .collect();
    }
    if let Some(n) = node.as_unless_node() {
        return Some(n.predicate())
            .into_iter()
            .chain(sole(n.statements()))
            .chain(n.else_clause().and_then(|e| sole(e.statements())))
            .collect();
    }
    // A case's when/in clauses remain composite children, but its else body
    // has no reference wrapper: a sole expression is a direct child.
    if let Some(n) = node.as_case_node() {
        return n
            .predicate()
            .into_iter()
            .chain(n.conditions().iter())
            .chain(n.else_clause().and_then(|e| sole(e.statements())))
            .collect();
    }
    if let Some(n) = node.as_case_match_node() {
        return n
            .predicate()
            .into_iter()
            .chain(n.conditions().iter())
            .chain(n.else_clause().and_then(|e| sole(e.statements())))
            .collect();
    }
    children(node)
}

// Direct reference children: Prism's argument lists are transparent and sole
// statement bodies collapse. The generic fallback visits a fixed-arity parent
// subtree; wide lists/calls below use O(1) last lookups instead. Total traversal
// work is bounded by the pipeline's fixed AST depth, not sibling-count squared.
fn children<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    struct Children<'pr> {
        depth: usize,
        nodes: Vec<Node<'pr>>,
    }
    impl<'pr> NodeVisitor<'pr> for Children<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            #[cfg(test)]
            CHILD_VISITS.with(|count| count.set(count.get() + 1));
            if self.depth == 1 {
                self.nodes.push(node);
            }
            self.depth += 1;
        }
        fn leave(&mut self) {
            self.depth -= 1;
        }
    }
    let mut visitor = Children {
        depth: 0,
        nodes: Vec::new(),
    };
    walk(&mut visitor, node);
    visitor
        .nodes
        .into_iter()
        .flat_map(|n| {
            if let Some(a) = n.as_arguments_node() {
                return a.arguments().iter().collect();
            }
            if let Some(s) = n.as_statements_node()
                && s.body().len() == 1
            {
                return s.body().iter().collect();
            }
            vec![n]
        })
        .collect()
}
fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}
fn last_list_child<'pr>(parent: &Node<'pr>, child: &Node<'pr>) -> Option<Option<Node<'pr>>> {
    macro_rules! index_value {
        ($($access:ident),*) => { $(if let Some(n) = parent.$access()
            && same(&n.value(), child) { return Some(Some(n.value())); })* };
    }
    // An index compound has a synthetic getter child for its indices and a
    // separate RHS; each can be its own parent's last child in RuboCop.
    index_value!(
        as_index_and_write_node,
        as_index_or_write_node,
        as_index_operator_write_node
    );
    macro_rules! arguments {
        ($($access:ident),*) => { $(if let Some(n) = parent.$access() {
            return Some(n.arguments().and_then(|a| a.arguments().last()));
        })* };
    }
    arguments!(
        as_yield_node,
        as_super_node,
        as_return_node,
        as_break_node,
        as_next_node,
        as_index_and_write_node,
        as_index_or_write_node,
        as_index_operator_write_node,
        as_index_target_node
    );
    // Prism links sibling rescues through `subsequent`; the reference AST
    // instead gives each resbody its own parent whose last child is its body.
    if let Some(n) = parent.as_rescue_node()
        && let Some(body) = n.statements()
    {
        return Some(Some(body.as_node()));
    }
    if let Some(n) = parent.as_when_node() {
        return Some(
            n.statements()
                .map(|s| s.as_node())
                .or_else(|| n.conditions().last()),
        );
    }
    None
}
fn last_child_of_parent(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    for parent in cx.ancestors() {
        if let Some(s) = parent.as_statements_node() {
            if s.body().len() == 1 {
                continue;
            }
            return s.body().last().is_some_and(|n| same(&n, node));
        }
        if parent.as_arguments_node().is_some() || parent.as_else_node().is_some() {
            continue;
        }
        let last = if let Some(c) = parent.as_call_node() {
            c.block()
                .filter(|b| b.as_block_node().is_none())
                .or_else(|| c.arguments().and_then(|a| a.arguments().last()))
                .or_else(|| c.receiver())
        } else if let Some(last) = last_list_child(parent, node) {
            last
        } else if let Some(n) = parent.as_begin_node() {
            if n.rescue_clause().is_some() {
                // Body and else belong to a synthetic rescue, inside any
                // synthetic ensure. Only else can be its last direct body.
                n.else_clause()
                    .and_then(|e| e.statements())
                    .map(|s| s.as_node())
            } else {
                n.ensure_clause()
                    .and_then(|e| e.statements())
                    .or_else(|| n.statements())
                    .map(|s| s.as_node())
            }
        } else if let Some(a) = parent.as_array_node() {
            a.elements().last()
        } else if let Some(a) = parent.as_hash_node() {
            a.elements().last()
        } else if let Some(a) = parent.as_keyword_hash_node() {
            a.elements().last()
        } else if let Some(n) = parent.as_unless_node() {
            n.statements()
                .map(|s| s.as_node())
                .or_else(|| {
                    n.else_clause()
                        .and_then(|e| e.statements())
                        .map(|s| s.as_node())
                })
                .or_else(|| Some(n.predicate()))
        } else {
            children(parent).pop()
        };
        return last.is_some_and(|n| {
            if let Some(s) = n.as_statements_node()
                && s.body().len() == 1
            {
                return s.body().first().is_some_and(|n| same(&n, node));
            }
            if let Some(e) = n.as_else_node() {
                return e.statements().is_some_and(|s| {
                    s.body().len() == 1 && s.body().first().is_some_and(|n| same(&n, node))
                });
            }
            same(&n, node)
        });
    }
    true
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/identical_conditional_branches_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/IdenticalConditionalBranches")
            .collect()
    }
    fn check(source: &str, expected: &[(u32, u32, &str)]) {
        let found = offenses(source);
        assert_eq!(found.len(), expected.len(), "{source}: {found:?}");
        for (o, &(line, col, expression)) in found.iter().zip(expected) {
            assert_eq!((o.line, o.col), (line, col), "{source}");
            assert_eq!(
                o.message,
                format!("Move `{expression}` out of the conditional.")
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn common_bodies_and_edges_report_each_expression() {
        check(
            "if something\n  do_x\nelse\n  do_x\nend\n",
            &[(2, 3, "do_x"), (4, 3, "do_x")],
        );
        check(
            "if something\n  before\n  do_x\nelse\n  other\n  do_x\nend\n",
            &[(3, 3, "do_x"), (6, 3, "do_x")],
        );
        check(
            "if something\n  do_x\n  before\nelse\n  do_x\n  other\nend\n",
            &[(2, 3, "do_x"), (5, 3, "do_x")],
        );
    }
    #[test]
    fn else_conditionals_are_expanded_even_without_elsif_syntax() {
        check(
            "if a; work; elsif b; work; else; work; end",
            &[(1, 7, "work"), (1, 22, "work"), (1, 34, "work")],
        );
        check(
            "if a; work; else; if b; work; else; work; end; end",
            &[(1, 7, "work"), (1, 25, "work"), (1, 37, "work")],
        );
        check("if a; work if ready; else; work if ready; end", &[]);
        check("if a; work unless ready; else; work unless ready; end", &[]);
        check(
            "if a; work; else; unless b; work; else; work; end; end",
            &[(1, 7, "work"), (1, 29, "work"), (1, 41, "work")],
        );
    }
    #[test]
    fn assignment_guards_distinguish_variables_constants_and_receivers() {
        for (condition, assignment) in [
            ("x", "x = value"),
            ("x.condition", "x = value"),
            ("x&.condition", "x = value"),
            ("@x", "@x = value"),
            ("x == 0", "x += 1"),
            ("h[:key]", "h[:key] = value"),
        ] {
            check(
                &format!(
                    "x = 0\nif {condition}\n  {assignment}\n  first\nelse\n  {assignment}\n  second\nend\n"
                ),
                &[],
            );
        }
        check(
            "if CONST == 0\n  CONST = value\n  first\nelse\n  CONST = value\n  second\nend",
            &[(2, 3, "CONST = value"), (5, 3, "CONST = value")],
        );
        check(
            "if x.condition\n  first\n  x = value\nelse\n  second\n  x = value\nend",
            &[(3, 3, "x = value"), (6, 3, "x = value")],
        );
    }
    #[test]
    fn single_branch_heads_follow_immediate_parent_value_policy() {
        let conditional = "if a\n  work\nelse\n  work\n  other\nend";
        for source in [
            conditional.to_owned(),
            format!("def f\n{conditional}\nend"),
            format!("value = {conditional}"),
            format!("[{conditional}]"),
            format!("consume({conditional})"),
        ] {
            check(&source, &[]);
        }
        for (source, expected) in [
            (
                format!("{conditional}\nafter"),
                vec![(2, 3, "work"), (4, 3, "work")],
            ),
            (
                format!("consume({conditional}, after)"),
                vec![(2, 3, "work"), (4, 3, "work")],
            ),
        ] {
            check(&source, &expected);
        }
    }
    #[test]
    fn rescue_and_ensure_have_separate_reference_parent_boundaries() {
        let conditional = "if a\n  work\nelse\n  work\n  other\nend";
        for (prefix, suffix) in [
            (
                "begin; risky; rescue ArgumentError",
                "rescue RuntimeError; recover; end",
            ),
            (
                "begin; risky; rescue ArgumentError",
                "rescue RuntimeError; recover; ensure; cleanup; end",
            ),
            (
                "begin; risky; rescue ArgumentError; recover; rescue RuntimeError",
                "end",
            ),
            (
                "begin; risky; rescue; recover; else",
                "ensure; cleanup; end",
            ),
            (
                "def f; risky; rescue; recover; else",
                "ensure; cleanup; end",
            ),
            ("begin; risky; ensure", "end"),
            ("begin", "ensure; end"),
        ] {
            check(&format!("{prefix}\n{conditional}\n{suffix}"), &[]);
        }
        for (prefix, suffix) in [
            (
                "begin; risky; rescue ArgumentError",
                "after; rescue RuntimeError; recover; end",
            ),
            (
                "begin; risky; rescue; recover; else",
                "after; ensure; cleanup; end",
            ),
            ("begin", "rescue; recover; ensure; cleanup; end"),
            ("begin", "ensure; cleanup; end"),
        ] {
            check(
                &format!("{prefix}\n{conditional}\n{suffix}"),
                &[(3, 3, "work"), (5, 3, "work")],
            );
        }
    }
    #[test]
    fn compound_index_value_is_a_consumed_last_child() {
        for operator in ["||=", "&&=", "+="] {
            check(
                &format!("obj[1] {operator} if a; work; else; work; other; end"),
                &[],
            );
        }
    }
    #[test]
    fn wide_siblings_have_linear_child_work() {
        // Every conditional is a direct child, not wrapped in parentheses.
        let count = 128;
        let conditional = "if a\n  work\nelse\n  work\n  other\nend";
        let list = vec![conditional; count].join(", ");
        for source in [
            format!("def f\n  yield({list})\nend"),
            format!("def f\n  super({list})\nend"),
            format!("consume({list})"),
            format!("[{list}]"),
            format!("case x\nwhen {list}\nend"),
            vec![conditional; count].join("\n"),
        ] {
            super::CHILD_VISITS.with(|n| n.set(0));
            let found = offenses(&source);
            assert_eq!(found.len(), (count - 1) * 2, "{source}");
            super::CHILD_VISITS.with(|n| assert!(n.get() < count * 32, "{} child visits", n.get()));
        }
    }
    #[test]
    fn structural_edges_preserve_grouping_literals_and_line_values() {
        check(
            "if a; work(1); else; work 0x1; end",
            &[(1, 7, "work(1)"), (1, 22, "work 0x1")],
        );
        check(
            "if a; __LINE__; else; __LINE__; end",
            &[(1, 7, "__LINE__"), (1, 23, "__LINE__")],
        );
        check("if a\n  __LINE__\nelse\n  __LINE__\nend", &[]);
        check("if a; (work); else; ((work)); end", &[]);
        let found = offenses(
            "if a\n  begin\n    work\n  rescue\n    recover\n  ensure\n    cleanup\n  end\nelse\n  begin\n    work\n  rescue\n    recover\n  ensure\n    cleanup\n  end\nend",
        );
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 8));
        assert_eq!(
            (found[1].line, found[1].col, found[1].end_line),
            (10, 3, 16)
        );
        assert!(found.iter().all(|o| o.correction.is_none()));
    }
    #[test]
    fn condition_guards_use_direct_reference_children_not_descendants() {
        for condition in [
            "each { x }",
            "(x; y)",
            "begin; x; end",
            "if c; x; else; y; end",
            "unless c; x; else; y; end",
            "/(?<name>.)/ =~ x",
        ] {
            check(
                &format!(
                    "x = 1; y = 2\nif {condition}\n  x += 1\n  first\nelse\n  x += 1\n  second\nend"
                ),
                &[],
            );
        }
        for condition in [
            "each(x) { foo }",
            "each { x; y }",
            "x.foo ||= y",
            "x[0] += y",
            "begin; x; rescue; y; end",
            "begin; x; ensure; y; end",
        ] {
            check(
                &format!(
                    "x = 1; y = 2\nif {condition}\n  x += 1\n  first\nelse\n  x += 1\n  second\nend"
                ),
                &[(3, 3, "x += 1"), (6, 3, "x += 1")],
            );
        }
        check(
            "x = 1\nif x == 1\n  x::C = y\n  first\nelse\n  x::C = y\n  second\nend",
            &[],
        );
    }
    #[test]
    fn case_condition_else_reads_guard_assignment_heads_and_tails() {
        for branch in ["when :ok", "in :ok"] {
            for assignment in ["x += 1", "y = x"] {
                let condition = format!("case kind; {branch}; true; else; x; end");
                for body in [
                    format!("{assignment}\n  first\nelse\n  {assignment}\n  second"),
                    format!("first\n  {assignment}\nelse\n  second\n  {assignment}"),
                ] {
                    check(&format!("x = 0\nif {condition}\n  {body}\nend"), &[]);
                }
            }
            // Reads inside a when/in or multi-expression else are not direct
            // children of the reference case node and must not suppress edges.
            for condition in [
                format!("case kind; {branch}; x; else; false; end"),
                format!("case kind; {branch}; true; else; x; false; end"),
            ] {
                check(
                    &format!(
                        "x = 0\nif {condition}\n  x += 1\n  first\nelse\n  x += 1\n  second\nend"
                    ),
                    &[(3, 3, "x += 1"), (6, 3, "x += 1")],
                );
            }
        }
    }
    #[test]
    fn incomplete_and_different_branches_are_not_common() {
        for source in [
            "if x; work; end",
            "if x; work; elsif y; work; end",
            "if x; work(1); else; work(2); end",
            "if x; # comment\nelse; work; end",
            "if x; (); else; (); end",
        ] {
            check(source, &[]);
        }
    }
}
