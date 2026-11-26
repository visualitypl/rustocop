//! `Lint/UnreachableLoop` — syntactic loops with at most one iteration.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct UnreachableLoop;
impl Cop for UnreachableLoop {
    fn name(&self) -> &'static str {
        "Lint/UnreachableLoop"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !loop_node(node, file) || cx.ancestors().any(|a| loop_node(a, file)) {
            return;
        }
        // One complete postorder per disjoint outermost eligible loop. Nested
        // loops share these summaries; no per-loop subtree or root rescans.
        let mut projection = Projection {
            file,
            stack: Vec::new(),
            nodes: Vec::new(),
            out,
        };
        walk::walk(&mut projection, node);
    }
}
// Rubocop-ast's fixed Enumerable + enumerator tables at Ruby3.4. The
// enumerator predicate additionally accepts *any* selector starting each_.
const METHODS: &[&[u8]] = &[
    b"all?",
    b"any?",
    b"chain",
    b"chunk",
    b"chunk_while",
    b"collect",
    b"collect_concat",
    b"compact",
    b"count",
    b"cycle",
    b"detect",
    b"drop",
    b"drop_while",
    b"each",
    b"each_cons",
    b"each_entry",
    b"each_slice",
    b"each_with_index",
    b"each_with_object",
    b"entries",
    b"filter",
    b"filter_map",
    b"find",
    b"find_all",
    b"find_index",
    b"first",
    b"flat_map",
    b"grep",
    b"grep_v",
    b"group_by",
    b"include?",
    b"inject",
    b"lazy",
    b"map",
    b"max",
    b"max_by",
    b"member?",
    b"min",
    b"min_by",
    b"minmax",
    b"minmax_by",
    b"none?",
    b"one?",
    b"partition",
    b"reduce",
    b"reject",
    b"reverse_each",
    b"select",
    b"slice_after",
    b"slice_before",
    b"slice_when",
    b"sort",
    b"sort_by",
    b"sum",
    b"take",
    b"take_while",
    b"tally",
    b"to_a",
    b"to_h",
    b"to_set",
    b"uniq",
    b"zip",
    b"downto",
    b"loop",
    b"map!",
    b"reject!",
    b"select!",
    b"times",
    b"upto",
];
fn loop_node(node: &Node<'_>, file: &SourceFile) -> bool {
    if matches!(
        node,
        Node::WhileNode { .. } | Node::UntilNode { .. } | Node::ForNode { .. }
    ) {
        return true;
    }
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.block().is_none_or(|b| b.as_block_node().is_none()) {
        return false;
    }
    let name = call.name().as_slice();
    if !METHODS.contains(&name) && !name.starts_with(b"each_") {
        return false;
    }
    let start = call.location().start_offset();
    let text = file.match_pattern_source(&file.bytes[start..ast::source_end(node)]);
    !allowed(&text)
}
fn allowed(source: &[u8]) -> bool {
    // The default regex is unanchored, ASCII digits, with exact punctuation.
    [b"exactly(".as_slice(), b"at_least(", b"at_most("]
        .iter()
        .any(|prefix| {
            source.windows(prefix.len()).enumerate().any(|(i, w)| {
                if w != *prefix {
                    return false;
                }
                let rest = &source[i + prefix.len()..];
                let n = rest.iter().take_while(|b| b.is_ascii_digit()).count();
                n > 0 && rest[n..].starts_with(b").times")
            })
        })
}
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Other,
    Begin,
    Flow,
    Continue,
    Or,
}
// A small cop-local projection, not a CFG or variable/type analysis. `flow`
// means upstream break_statement?, independent of syntactic root kind: e.g.
// kwbegin may be breaking but must NOT be flattened as begin by statements().
struct Entry {
    kind: Kind,
    children: Vec<usize>,
    flow: bool,
    continuation: bool,
    descendant_continuation: bool,
    // Last OR in AST preorder (not source order), including/excluding self.
    last_or: Option<bool>,
    descendant_last_or: Option<bool>,
    loop_node: bool,
}
struct Frame<'pr> {
    node: Node<'pr>,
    children: Vec<(Node<'pr>, Vec<usize>)>,
}
struct Projection<'a, 'pr> {
    file: &'a SourceFile,
    stack: Vec<Frame<'pr>>,
    nodes: Vec<Entry>,
    out: &'a mut Vec<Offense>,
}
impl Frame<'_> {
    fn get(&self, node: &Node<'_>) -> Option<usize> {
        self.children
            .iter()
            .find(|(n, _)| {
                std::mem::discriminant(n) == std::mem::discriminant(node)
                    && n.location().start_offset() == node.location().start_offset()
                    && n.location().end_offset() == node.location().end_offset()
            })
            .and_then(|(_, ids)| ids.first().copied())
    }
}
impl<'pr> NodeVisitor<'pr> for Projection<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        self.stack.push(Frame {
            node,
            children: Vec::new(),
        });
    }
    fn leave(&mut self) {
        let frame = self.stack.pop().unwrap();
        let ids = self.project(&frame);
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push((frame.node, ids));
        }
    }
}
impl Projection<'_, '_> {
    fn sequence_flow(&self, ids: &[usize]) -> bool {
        self.candidate(ids)
            .is_some_and(|(id, preceded)| self.nodes[id].flow && !preceded)
    }
    fn candidate(&self, ids: &[usize]) -> Option<(usize, bool)> {
        let mut preceded = false;
        for &id in ids {
            let n = &self.nodes[id];
            if n.flow {
                return Some((id, preceded));
            }
            // each_descendant excludes the sibling itself. A bare next/redo
            // does not suppress, and only direct loop siblings are skipped.
            preceded |= !n.loop_node && n.descendant_continuation;
        }
        None
    }
    fn body_flow(&self, id: Option<usize>) -> bool {
        id.is_some_and(|i| self.nodes[i].flow)
    }
    fn project(&mut self, f: &Frame<'_>) -> Vec<usize> {
        let node = &f.node;
        let mut children: Vec<_> = f
            .children
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        // Prism-only lists and else wrappers are transparent. A one-expression
        // Statements body is not a Ruby begin; explicit parentheses always are.
        if node.as_arguments_node().is_some()
            || node.as_else_node().is_some()
            || (node.as_statements_node().is_some() && children.len() < 2)
        {
            return children;
        }
        if node.as_program_node().is_some() {
            return children;
        }
        // A guarded pattern has separate pattern/guard children in Ruby,
        // not Prism's modifier If/Unless (predicate before pattern). This
        // matters to the LAST descendant OR even inside a returned case.
        if self
            .stack
            .last()
            .is_some_and(|p| p.node.as_in_node().is_some())
        {
            let guarded = node
                .as_if_node()
                .map(|n| (n.statements(), n.predicate()))
                .or_else(|| {
                    node.as_unless_node()
                        .map(|n| (n.statements(), n.predicate()))
                });
            if let Some((pattern, guard)) = guarded {
                return [pattern.and_then(|s| f.get(&s.as_node())), f.get(&guard)]
                    .into_iter()
                    .flatten()
                    .collect();
            }
        }
        let mut kind = Kind::Other;
        let mut flow = false;
        let is_loop = loop_node(node, self.file);
        let mut body = None;
        if let Some(n) = node.as_while_node() {
            body = n.statements().and_then(|s| f.get(&s.as_node()));
        }
        if let Some(n) = node.as_until_node() {
            body = n.statements().and_then(|s| f.get(&s.as_node()));
        }
        if let Some(n) = node.as_for_node() {
            body = n.statements().and_then(|s| f.get(&s.as_node()));
        }
        if let Some(call) = node.as_call_node() {
            if let Some(block) = call.block().and_then(|b| b.as_block_node()) {
                let block_id = f.get(&block.as_node()).unwrap();
                body = block
                    .body()
                    .and_then(|_| self.nodes[block_id].children.last().copied());
                // Ruby's block owns a send, parameters and body. The send
                // does NOT own the body (or its next/redo descendants).
                let block_children = self.nodes[block_id].children.clone();
                children.retain(|&i| i != block_id);
                let descendant_continuation = children.iter().any(|&i| self.nodes[i].continuation);
                let last_or = children.iter().rev().find_map(|&i| self.nodes[i].last_or);
                let send_id = self.nodes.len();
                self.nodes.push(Entry {
                    kind: Kind::Other,
                    children,
                    flow: false,
                    continuation: descendant_continuation,
                    descendant_continuation,
                    last_or,
                    descendant_last_or: last_or,
                    loop_node: false,
                });
                children = vec![send_id];
                children.extend(block_children);
            } else if !call.is_safe_navigation()
                && matches!(
                    call.name().as_slice(),
                    b"raise" | b"fail" | b"throw" | b"exit" | b"exit!" | b"abort"
                )
                && call
                    .receiver()
                    .is_none_or(|r| ast::is_global_const(&r, &["Kernel"]))
            {
                kind = Kind::Flow;
                flow = true;
            }
        }
        if matches!(node, Node::ReturnNode { .. } | Node::BreakNode { .. }) {
            kind = Kind::Flow;
            flow = true;
        }
        if matches!(node, Node::NextNode { .. } | Node::RedoNode { .. }) {
            kind = Kind::Continue;
        }
        if node.as_statements_node().is_some() {
            kind = Kind::Begin;
            flow = self.sequence_flow(&children);
        }
        if let Some(n) = node.as_parentheses_node() {
            kind = Kind::Begin;
            if let Some(id) = n.body().and_then(|n| f.get(&n))
                && self.nodes[id].kind == Kind::Begin
            {
                children = self.nodes[id].children.clone();
            }
            flow = self.sequence_flow(&children);
        }
        if let Some(n) = node.as_begin_node() {
            // rescue/ensure wrap the body's begin/kwbegin in nonbreaking nodes.
            if n.rescue_clause().is_none() && n.ensure_clause().is_none() {
                if let Some(id) = n.statements().and_then(|s| f.get(&s.as_node()))
                    && self.nodes[id].kind == Kind::Begin
                {
                    children = self.nodes[id].children.clone();
                }
                flow = self.sequence_flow(&children);
                // Explicit begin/end is kwbegin, NOT flattened by statements().
                if n.begin_keyword_loc().is_none() {
                    kind = Kind::Begin;
                }
            }
        }
        if let Some(n) = node.as_if_node() {
            flow = self.body_flow(n.statements().and_then(|s| f.get(&s.as_node())))
                && self.body_flow(n.subsequent().and_then(|s| f.get(&s)));
        }
        if let Some(n) = node.as_unless_node() {
            flow = self.body_flow(n.statements().and_then(|s| f.get(&s.as_node())))
                && self.body_flow(n.else_clause().and_then(|s| f.get(&s.as_node())));
            // Parser stores (if predicate ELSE BODY).
            let predicate = f.get(&n.predicate());
            let yes = n.else_clause().and_then(|s| f.get(&s.as_node()));
            let no = n.statements().and_then(|s| f.get(&s.as_node()));
            children = [predicate, yes, no].into_iter().flatten().collect();
        }
        if let Some(n) = node.as_when_node() {
            flow = self.body_flow(n.statements().and_then(|s| f.get(&s.as_node())));
        }
        if let Some(n) = node.as_in_node() {
            flow = self.body_flow(n.statements().and_then(|s| f.get(&s.as_node())));
        }
        if let Some(n) = node.as_case_node() {
            flow = self.body_flow(n.else_clause().and_then(|s| f.get(&s.as_node())))
                && f.children
                    .iter()
                    .filter(|(b, _)| b.as_when_node().is_some())
                    .all(|(_, ids)| self.body_flow(ids.first().copied()));
        }
        if let Some(n) = node.as_case_match_node() {
            flow = self.body_flow(n.else_clause().and_then(|s| f.get(&s.as_node())))
                && f.children
                    .iter()
                    .filter(|(b, _)| b.as_in_node().is_some())
                    .all(|(_, ids)| self.body_flow(ids.first().copied()));
        }
        if node.as_or_node().is_some() {
            kind = Kind::Or;
        }
        let descendant_continuation = children.iter().any(|&i| self.nodes[i].continuation);
        let descendant_last_or = children.iter().rev().find_map(|&i| self.nodes[i].last_or);
        let last_or = descendant_last_or.or_else(|| {
            (kind == Kind::Or).then(|| {
                children
                    .last()
                    .is_some_and(|&i| self.nodes[i].kind == Kind::Continue)
            })
        });
        // An OR is visited before its descendants; the last descendant wins.
        let id = self.nodes.len();
        self.nodes.push(Entry {
            kind,
            children,
            flow,
            continuation: kind == Kind::Continue || descendant_continuation,
            descendant_continuation,
            last_or,
            descendant_last_or,
            loop_node: is_loop,
        });
        if is_loop && let Some(body) = body {
            let ids = if self.nodes[body].kind == Kind::Begin {
                self.nodes[body].children.clone()
            } else {
                vec![body]
            };
            if let Some((candidate, mut preceded)) = self.candidate(&ids) {
                // A single body expression has the loop's predicate/collection/
                // send/parameters as left siblings, unlike a begin sequence.
                if self.nodes[body].kind != Kind::Begin {
                    for &sibling in &self.nodes[id].children {
                        if sibling == body {
                            break;
                        }
                        let n = &self.nodes[sibling];
                        preceded |= !n.loop_node && n.descendant_continuation;
                    }
                }
                if !preceded && self.nodes[candidate].descendant_last_or != Some(true) {
                    let loc = node.location();
                    let (line, col) = self.file.reference_line_col(loc.start_offset());
                    self.out.push(Offense {
                        cop: "Lint/UnreachableLoop",
                        severity: UnreachableLoop.severity(),
                        line,
                        col,
                        end_line: self.file.last_line_of(loc),
                        message: "This loop will have at most one iteration.".into(),
                        correction: None,
                    });
                }
            }
        }
        vec![id]
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/unreachable_loop_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        assert!(
            ruby_prism::parse(s.as_bytes()).errors().next().is_none(),
            "{s}"
        );
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnreachableLoop")
            .collect()
    }
    #[test]
    fn keyword_loops_and_every_flow_command() {
        for body in [
            "break",
            "return value",
            "raise MyError",
            "fail",
            "throw :exit",
            "exit",
            "exit!",
            "abort",
            "Kernel.raise",
            "::Kernel.fail",
            "(break)",
            "begin; break; end",
        ] {
            for s in [
                format!("while test\n {body}\nend\n"),
                format!("until test\n {body}\nend\n"),
                format!("for x in items\n {body}\nend\n"),
                format!("begin\n {body}\nend while test\n"),
                format!("begin\n {body}\nend until test\n"),
            ] {
                let found = offenses(&s);
                assert_eq!(found.len(), 1, "{s}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 3));
                assert_eq!(
                    found[0].message,
                    "This loop will have at most one iteration."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn all_branches_must_exit() {
        for body in [
            "if c; break; else; raise MyError; end",
            "if c; break; elsif d; return; else; abort; end",
            "case x; when 1; break; else; raise MyError; end",
            "case x; in 1; break; else; raise MyError; end",
        ] {
            assert_eq!(
                offenses(&format!("while test; {body}; end")).len(),
                1,
                "{body}"
            );
        }
        for body in [
            "if c; break; elsif d; raise MyError; end",
            "if c; break; elsif d; work; else; raise MyError; end",
            "case x; when 1; break; end",
            "case x; in 1; break; end",
            "case x; when 1; break; when 2; work; else; raise MyError; end",
            "case x; in 1; break; in 2; work; else; raise MyError; end",
            "if c; next if d; break; else; break; end",
        ] {
            assert!(
                offenses(&format!("while test; {body}; end")).is_empty(),
                "{body}"
            );
        }
    }
    #[test]
    fn preceding_continue_and_last_descendant_or() {
        for body in [
            "next if x.odd?; x += 1; break",
            "redo if x.odd?; if c; break; else; raise; end",
            "redo if x.odd?; case x; when 1; break; else; raise; end",
            "next if x.odd?; case x; in 1; break; else; raise; end",
            "return work(value) || next",
            "return work(value) || redo",
        ] {
            assert!(
                offenses(&format!("items.each do |value|; {body}; end")).is_empty(),
                "{body}"
            );
        }
        for body in [
            "return work(value) || break",
            "next; break",
            "return (a || next), (b || c)",
        ] {
            assert_eq!(
                offenses(&format!("items.each do |value|; {body}; end")).len(),
                1,
                "{body}"
            );
        }
    }
    #[test]
    fn block_methods_and_fixed_unanchored_allowed_pattern() {
        for s in [
            "string.split('-').map { raise StandardError }",
            "string.map { raise StandardError }",
            "2.times { raise StandardError }",
            "2.times { raise _1 }",
            "2.times { raise it }",
            "items.each { return work || break }",
            "items.each { return _1.odd? || break }",
            "items.each { return it.odd? || break }",
            "object&.loop { break }",
            "each_custom { break }",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "exactly(2).times.with(x) { raise StandardError }",
            "exactly(2).times { raise StandardError }",
            "at_least(3).times { break }",
            "at_most(4).times { break }",
            "not_exactly(12).times.foo.each { break }",
            "object.foo { break }",
            "object.eachish { break }",
            "items.each { self.raise }",
            "items.each { Kernel&.raise }",
            "items.each { Other::Kernel.raise }",
            "items.each { raise {} }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn guard_and_unless_descendants_keep_ruby_ast_order() {
        for keyword in ["if", "unless"] {
            let s = format!(
                "items.each {{ return case x; in \"#{{a || next}}\" {keyword} (b || c); 1; else; 2; end }}"
            );
            assert_eq!(offenses(&s).len(), 1, "{s}");
            let s = format!(
                "items.each {{ return case x; in \"#{{a || b}}\" {keyword} (c || next); 1; else; 2; end }}"
            );
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }

    #[test]
    fn wide_case_and_nested_sibling_loops_have_bounded_projection_storage() {
        use super::*;
        fn count(n: usize) -> usize {
            let branches: String = (0..n).map(|i| format!("when {i}; break\n")).collect();
            let siblings = "items.each { next if test; break }\n".repeat(n);
            let text =
                format!("while test\n{siblings}case value\n{branches}else; return\nend\nend");
            let file = SourceFile::new("example.rb".into(), text.as_bytes().to_vec());
            let parsed = ruby_prism::parse(text.as_bytes());
            assert!(parsed.errors().next().is_none());
            let mut out = Vec::new();
            let mut projection = Projection {
                file: &file,
                stack: Vec::new(),
                nodes: Vec::new(),
                out: &mut out,
            };
            walk::walk(&mut projection, &parsed.node());
            let count = projection.nodes.len();
            assert!(projection.stack.is_empty());
            assert_eq!(out.len(), 1);
            count
        }
        let small = count(400);
        let large = count(800);
        assert!(large <= small * 2 && large > small);
    }

    #[test]
    fn inner_loops_are_skipped_only_as_direct_left_siblings() {
        let s = "until x > 0\n items.each do |item|\n  next if item.odd?\n  break\n end\n if x > 0\n  break some_value\n else\n  raise MyError\n end\n loop do\n  case y\n  when 1; return something\n  when 2; break\n  else; throw :exit\n  end\n end\nend";
        assert_eq!(
            offenses(s)
                .iter()
                .map(|o| (o.line, o.col))
                .collect::<Vec<_>>(),
            [(1, 1), (11, 2)]
        );
    }
}
