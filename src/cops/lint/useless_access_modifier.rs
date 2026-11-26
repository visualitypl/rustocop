//! `Lint/UselessAccessModifier` — redundant lexical visibility declarations.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::{HashMap, HashSet};

pub struct UselessAccessModifier;
impl Cop for UselessAccessModifier {
    fn name(&self) -> &'static str {
        "Lint/UselessAccessModifier"
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
        let name = if let Some(call) = node.as_call_node() {
            let name = call.name().as_slice();
            if !matches!(
                name,
                b"public"
                    | b"private"
                    | b"protected"
                    | b"module_function"
                    | b"private_class_method"
            ) {
                return;
            }
            name
        } else if synthetic_private_class_end(node).is_some() {
            b"private_class_method"
        } else {
            return;
        };
        let end = report_end(node);
        let root = cx.ancestors().last().unwrap_or(node);
        if !file
            .useless_access_modifiers(root)
            .reports
            .contains(&(node.location().start_offset(), end))
        {
            return;
        }
        let (line, col) = file.reference_line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: format!(
                "Useless `{}` access modifier.",
                String::from_utf8_lossy(name)
            ),
            correction: cx
                .corrections_requested()
                .then(|| whole_lines(file, node.location().start_offset(), end)),
        });
    }
}

/// RuboCop removes the whole lines of the modifier.
fn whole_lines(file: &SourceFile, start: usize, end: usize) -> crate::offense::Correction {
    crate::offense::Correction {
        range: file.line_start(file.line_of(start))
            ..file.line_start(file.line_of(end.saturating_sub(1).max(start)) + 1),
        replacement: String::new(),
    }
}

/// File-local facts, initialized only if a modifier candidate exists. The
/// complete typed-child walk is performed once, including heredoc children
/// outside their ancestors' physical spans. A tiny cop-local projection models
/// the upstream begin/send/block edges; it is not a second parse.
#[derive(Default)]
pub(crate) struct Facts {
    reports: HashSet<(usize, usize)>,
    #[cfg(test)]
    visits: usize,
    #[cfg(test)]
    nodes_visited: usize,
}
struct Raw<'pr> {
    node: Node<'pr>,
    children: Vec<usize>,
    bare: bool,
}
#[derive(Default)]
struct Tree<'pr> {
    raw: Vec<Raw<'pr>>,
    stack: Vec<usize>,
    macro_branches: HashMap<(usize, usize), bool>,
}
impl<'pr> NodeVisitor<'pr> for Tree<'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        let bare = node.as_call_node().is_some_and(|call| {
            call.receiver().is_none()
                && !call.is_safe_navigation()
                && call.arguments().is_none()
                && !call
                    .block()
                    .is_some_and(|b| b.as_block_argument_node().is_some())
                && matches!(
                    call.name().as_slice(),
                    b"public" | b"private" | b"protected" | b"module_function"
                )
                && self.macro_scope(&node)
        });
        let id = self.raw.len();
        if let Some(parent) = self.stack.last() {
            self.raw[*parent].children.push(id);
        }
        self.raw.push(Raw {
            node,
            children: vec![],
            bare,
        });
        self.stack.push(id);
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}
impl Tree<'_> {
    // The same macro-scope predicate as UselessConstantScoping, with branch
    // equality cached by actual AST edge. In a large begin predicate, asking
    // independently for each modifier would re-fingerprint that entire begin.
    fn macro_scope(&mut self, node: &Node<'_>) -> bool {
        let mut child = node;
        let mut child_id = self.raw.len();
        let mut skip_call = false;
        for index in (0..self.stack.len()).rev() {
            let parent_id = self.stack[index];
            let parent = &self.raw[parent_id].node;
            if constructor(parent)
                || parent.as_block_node().is_some()
                    && index > 0
                    && constructor(&self.raw[self.stack[index - 1]].node)
            {
                return true;
            }
            match parent {
                Node::ProgramNode { .. }
                | Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::SingletonClassNode { .. } => return true,
                Node::StatementsNode { .. }
                | Node::ArgumentsNode { .. }
                | Node::ParenthesesNode { .. }
                | Node::ElseNode { .. }
                | Node::LambdaNode { .. } => {}
                Node::BlockNode { .. } => skip_call = true,
                Node::CallNode { .. } if skip_call => skip_call = false,
                Node::IfNode { .. } | Node::UnlessNode { .. } => {
                    let allowed = self
                        .macro_branches
                        .entry((parent_id, child_id))
                        .or_insert_with(|| {
                            let (predicate, body, subsequent) = if let Some(n) = parent.as_if_node()
                            {
                                (
                                    n.predicate(),
                                    n.statements().map(|s| s.as_node()),
                                    n.subsequent(),
                                )
                            } else {
                                let n = parent.as_unless_node().unwrap();
                                (
                                    n.predicate(),
                                    n.statements().map(|s| s.as_node()),
                                    n.else_clause().map(|s| s.as_node()),
                                )
                            };
                            super::useless_constant_scoping::conditional_macro_child(
                                child, predicate, body, subsequent,
                            )
                        });
                    if !*allowed {
                        return false;
                    }
                }
                Node::BeginNode { .. }
                    if parent.as_begin_node().is_some_and(|n| {
                        n.rescue_clause().is_none() && n.ensure_clause().is_none()
                    }) => {}
                _ => return false,
            }
            if !parent
                .as_statements_node()
                .is_some_and(|s| s.body().len() == 1)
                && parent.as_else_node().is_none()
            {
                child = parent;
                child_id = parent_id;
            }
        }
        true
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Other,
    Begin,
    Send,
    Def,
    Defs,
    Scope,
    Block,
}
struct Entry {
    raw: usize,
    kind: Kind,
    children: Vec<usize>,
    new_scope: bool,
}
struct Analysis<'a, 'pr> {
    raw: &'a [Raw<'pr>],
    entries: Vec<Entry>,
    scanned: HashSet<usize>,
    reports: HashSet<(usize, usize)>,
    #[cfg(test)]
    visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut tree = Tree::default();
        walk::walk(&mut tree, root);
        let mut analysis = Analysis {
            raw: &tree.raw,
            entries: vec![],
            scanned: HashSet::new(),
            reports: HashSet::new(),
            #[cfg(test)]
            visits: 0,
        };
        let mut projected = vec![vec![]; tree.raw.len()];
        let mut callbacks = vec![];
        for id in (0..tree.raw.len()).rev() {
            let raw = &tree.raw[id];
            let node = &raw.node;
            let mut children: Vec<_> = raw
                .children
                .iter()
                .flat_map(|i| projected[*i].iter().copied())
                .collect();
            // parser_prism represents `unless` as if(predicate, else, body).
            if node.as_unless_node().is_some() && children.len() == 3 {
                children.swap(1, 2);
            }
            let kind = match node {
                Node::ProgramNode { .. } | Node::ArgumentsNode { .. } | Node::BlockNode { .. } => {
                    None
                }
                Node::StatementsNode { .. } if children.len() <= 1 => None,
                // Upstream folds an unparenthesized compound-statement begin
                // into these parentheses, rather than nesting two begins.
                // Keep explicit inner parentheses/keyword begins intact.
                Node::ParenthesesNode { .. }
                    if node
                        .as_parentheses_node()
                        .and_then(|p| p.body())
                        .and_then(|b| b.as_statements_node())
                        .is_some_and(|b| b.body().len() > 1) =>
                {
                    None
                }
                Node::StatementsNode { .. } | Node::ParenthesesNode { .. } => Some(Kind::Begin),
                Node::CallNode { .. } => Some(Kind::Send),
                Node::DefNode { .. } => Some(if node.as_def_node().unwrap().receiver().is_some() {
                    Kind::Defs
                } else {
                    Kind::Def
                }),
                Node::ClassNode { .. }
                | Node::ModuleNode { .. }
                | Node::SingletonClassNode { .. } => Some(Kind::Scope),
                Node::LambdaNode { .. } => Some(Kind::Block),
                _ => Some(Kind::Other),
            };
            let Some(kind) = kind else {
                projected[id] = children;
                continue;
            };
            let call = node.as_call_node();
            let block = call
                .as_ref()
                .and_then(|c| c.block())
                .filter(|b| b.as_block_node().is_some());
            // Prism nests the literal block under send. Upstream makes the
            // send (receiver/arguments) and block parameters/body siblings.
            let block_children = if block.is_some() {
                let block_id = *raw.children.last().unwrap();
                let n = projected[block_id].len();
                children.split_off(children.len() - n)
            } else {
                vec![]
            };
            // A compound attribute assignment has a read-side send before
            // its RHS. Only private_class_method can match that explicit send.
            if synthetic_private_class_end(node).is_some() {
                let rhs = children.pop();
                let getter = analysis.entries.len();
                analysis.entries.push(Entry {
                    raw: id,
                    kind: Kind::Send,
                    children,
                    new_scope: false,
                });
                children = vec![getter];
                children.extend(rhs);
            }
            let send_id = analysis.entries.len();
            analysis.entries.push(Entry {
                raw: id,
                kind,
                children,
                new_scope: kind == Kind::Scope || constructor(node),
            });
            let final_id = if block.is_some() {
                let mut children = vec![send_id];
                children.extend(block_children);
                let block_id = analysis.entries.len();
                analysis.entries.push(Entry {
                    raw: id,
                    kind: Kind::Block,
                    children,
                    new_scope: constructor(node) || eval_block(node),
                });
                block_id
            } else {
                send_id
            };
            projected[id] = vec![final_id];
            if kind == Kind::Scope || block.is_some() && analysis.entries[final_id].new_scope {
                // The callback checks only body, not the class name, superclass,
                // send receiver/arguments or block parameters.
                let body = if let Some(n) = node.as_class_node() {
                    n.body()
                } else if let Some(n) = node.as_module_node() {
                    n.body()
                } else if let Some(n) = node.as_singleton_class_node() {
                    n.body()
                } else {
                    block.and_then(|b| b.as_block_node()).and_then(|b| b.body())
                };
                if let Some(body) = body {
                    let raw_body = raw
                        .children
                        .iter()
                        .copied()
                        .find(|i| same_node(&tree.raw[*i].node, &body));
                    let raw_body = raw_body.or_else(|| {
                        raw.children.last().and_then(|i| {
                            tree.raw[*i]
                                .children
                                .iter()
                                .copied()
                                .find(|j| same_node(&tree.raw[*j].node, &body))
                        })
                    });
                    if let Some(body_id) = raw_body.and_then(|i| projected[i].first().copied()) {
                        callbacks.push(body_id);
                    }
                }
            }
        }
        // Root `on_begin` only checks direct sends, and only a real begin
        // (a sequence or parentheses), never a lone modifier or kwbegin.
        if let [id] = projected[0].as_slice()
            && analysis.entries[*id].kind == Kind::Begin
        {
            for child in analysis.entries[*id].children.clone() {
                if analysis.entries[child].kind == Kind::Send {
                    let raw = &tree.raw[analysis.entries[child].raw];
                    if raw.bare || private_class(&raw.node) {
                        analysis.send(child, None, Some(child));
                    }
                }
            }
        }
        for body in callbacks {
            if analysis.entries[body].kind == Kind::Begin {
                analysis.scope(body);
            } else if analysis.entries[body].kind == Kind::Send
                && tree.raw[analysis.entries[body].raw].bare
            {
                analysis.report(body);
            }
        }
        Self {
            reports: analysis.reports,
            #[cfg(test)]
            visits: analysis.visits,
            #[cfg(test)]
            nodes_visited: tree.raw.len(),
        }
    }
}
impl Analysis<'_, '_> {
    fn report(&mut self, id: usize) {
        let node = &self.raw[self.entries[id].raw].node;
        self.reports
            .insert((node.location().start_offset(), report_end(node)));
    }
    fn scope(&mut self, id: usize) {
        // Both the recursive scope check and independent callbacks can reach
        // the same scope. Memoization bounds work to at most two passes over
        // ordinary children, regardless of lexical nesting depth.
        if !self.scanned.insert(id) {
            return;
        }
        let (_, unused) = self.children(id, Some(b"public"), None);
        if let Some(unused) = unused {
            self.report(unused);
        }
    }
    fn children(
        &mut self,
        id: usize,
        mut visibility: Option<&'static [u8]>,
        mut unused: Option<usize>,
    ) -> (Option<&'static [u8]>, Option<usize>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        for child in self.entries[id].children.clone() {
            let entry = &self.entries[child];
            let raw = &self.raw[entry.raw];
            if entry.kind == Kind::Send && (raw.bare || private_class(&raw.node)) {
                (visibility, unused) = self.send(child, visibility, unused);
            } else if entry.kind == Kind::Def || method_creator(&raw.node, entry.kind) {
                unused = None;
            } else if entry.new_scope {
                self.scope(child);
            } else if entry.kind != Kind::Defs {
                (visibility, unused) = self.children(child, visibility, unused);
            }
        }
        (visibility, unused)
    }
    fn send(
        &mut self,
        id: usize,
        visibility: Option<&'static [u8]>,
        unused: Option<usize>,
    ) -> (Option<&'static [u8]>, Option<usize>) {
        let raw = &self.raw[self.entries[id].raw];
        if synthetic_private_class_end(&raw.node).is_some() {
            self.report(id);
            return (visibility, unused);
        }
        let call = raw.node.as_call_node().unwrap();
        if raw.bare {
            let new: &'static [u8] = match call.name().as_slice() {
                b"public" => b"public",
                b"private" => b"private",
                b"protected" => b"protected",
                _ => b"module_function",
            };
            if visibility == Some(new) {
                self.report(id);
                (Some(new), unused)
            } else {
                if let Some(old) = unused {
                    self.report(old);
                }
                (Some(new), Some(id))
            }
        } else if call.arguments().is_none()
            && !call
                .block()
                .is_some_and(|b| b.as_block_argument_node().is_some())
        {
            self.report(id);
            (visibility, unused)
        } else {
            // Pinned check_send_node returns nil for an argument-bearing
            // private_class_method, resetting BOTH destructured state slots.
            (None, None)
        }
    }
}
fn same_node(a: &Node<'_>, b: &Node<'_>) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
        && a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}
fn report_end(node: &Node<'_>) -> usize {
    synthetic_private_class_end(node).unwrap_or_else(|| ast::source_end(node))
}
fn synthetic_private_class_end(node: &Node<'_>) -> Option<usize> {
    macro_rules! getter {
        ($access:ident) => {{
            let n = node.$access().unwrap();
            if n.read_name().as_slice() == b"private_class_method" && !n.is_safe_navigation() {
                n.message_loc().map(|l| l.end_offset())
            } else {
                None
            }
        }};
    }
    match node {
        Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
        Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
        Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
        _ => None,
    }
}
fn private_class(node: &Node<'_>) -> bool {
    synthetic_private_class_end(node).is_some()
        || node.as_call_node().is_some_and(|c| {
            !c.is_safe_navigation() && c.name().as_slice() == b"private_class_method"
        })
}
fn method_creator(node: &Node<'_>, kind: Kind) -> bool {
    node.as_call_node().is_some_and(|c| {
        c.receiver().is_none()
            && !c.is_safe_navigation()
            && (c.name().as_slice() == b"define_method"
                || kind == Kind::Send
                    && matches!(
                        c.name().as_slice(),
                        b"attr" | b"attr_reader" | b"attr_writer" | b"attr_accessor"
                    ))
    })
}
fn constructor(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|c| {
        !c.is_safe_navigation()
            && c.receiver().is_some_and(|r| match c.name().as_slice() {
                b"new" => ["Class", "Module", "Struct"]
                    .iter()
                    .any(|n| ast::is_global_const(&r, &[*n])),
                b"define" => ast::is_global_const(&r, &["Data"]),
                _ => false,
            })
    })
}
fn eval_block(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|c| {
        !c.is_safe_navigation()
            && c.arguments().is_none()
            && matches!(c.name().as_slice(), b"class_eval" | b"instance_eval")
    })
}

pub(crate) static USELESS_ACCESS_MODIFIERS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn useless_access_modifiers(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        USELESS_ACCESS_MODIFIERS.get(self, || Facts::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_access_modifier_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;

    #[test]
    fn removes_the_modifier_lines() {
        crate::cops::testing::assert_correction(
            &UselessAccessModifier,
            "class SomeClass\n  def some_method\n    puts 10\n  end\n  protected\nend\n",
            "class SomeClass\n  def some_method\n    puts 10\n  end\nend\n",
        );
    }
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessAccessModifier")
            .collect()
    }
    #[test]
    fn pinned_unused_and_repeated_modifiers() {
        for (source, line, col, name) in [
            ("class C\n  private\nend\n", 2, 3, "private"),
            (
                "class C\n  protected\n  def self.f; end\nend\n",
                2,
                3,
                "protected",
            ),
            ("class C\n  public\n  def f; end\nend\n", 2, 3, "public"),
            (
                "class C\n private\n private\n def f; end\nend\n",
                3,
                2,
                "private",
            ),
            ("def f;end\nprivate\ndef g;end\n", 2, 1, "private"),
            (
                "class C\n private_class_method\n def self.f;end\nend",
                2,
                2,
                "private_class_method",
            ),
            ("class C\n module_function\nend", 2, 2, "module_function"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(
                found[0].message,
                format!("Useless `{name}` access modifier.")
            );
        }
    }
    #[test]
    fn parenthesized_root_sequences_check_direct_modifiers() {
        for (source, line, col) in [
            ("(private; nil)\n", 1, 2),
            ("(\n  private\n  nil\n)\n", 2, 3),
            ("(private;)\n", 1, 2),
            ("(private; (protected; nil))\n", 1, 2),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(found[0].message, "Useless `private` access modifier.");
            assert!(found[0].correction.is_none());
        }
        // Explicit nested grouping and keyword begin are not root direct sends.
        for source in [
            "private\n",
            "((private))\n",
            "((private; nil))\n",
            "(begin;private;nil;end)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn canonical_whole_line_removal_in_a_closed_inactive_constructor() {
        for name in ["public", "protected", "private", "module_function"] {
            crate::cops::testing::assert_correction(
                &UselessAccessModifier,
                &format!("if false\n  Class.new do\n    {name}\n  end\nend\n"),
                "if false\n  Class.new do\n  end\nend\n",
            );
        }
    }
    #[test]
    fn original_byte_reference_matrix() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/useless_access_modifier.json"
        ))
        .unwrap();
        let mut mismatches = vec![];
        for row in rows {
            let bytes: Vec<u8> = crate::fixture::value(&row["bytes"]);
            let Ok(source) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let mut found: Vec<_> = offenses(source)
                .into_iter()
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            found.sort();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            if found != expected {
                mismatches.push(format!("{source:?}: {found:?} != {expected:?}"));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} mismatches:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
    #[test]
    fn linear_cached_scope_work_and_file_isolation() {
        let mut source = String::new();
        for i in 0..20 {
            source.push_str(&format!("class C{i}; private;"));
        }
        for _ in 0..1000 {
            source.push_str("private; ordinary_call;");
        }
        source.push_str("nil;");
        for _ in 0..20 {
            source.push_str("end;");
        }
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        let root = parsed.node();
        let facts = file.useless_access_modifiers(&root);
        assert_eq!(facts.reports.len(), 1020);
        assert!(facts.nodes_visited > 2000 && facts.nodes_visited < 2500);
        assert!(facts.visits <= 2 * facts.nodes_visited, "{}", facts.visits);
        for _ in 0..1000 {
            assert!(std::ptr::eq(facts, file.useless_access_modifiers(&root)));
        }
        assert!(offenses("class C;private;def f;end;end").is_empty());
    }
    #[test]
    fn large_macro_predicate_equality_is_cached_per_real_edge() {
        let sequence = "private;".repeat(1000);
        let source = format!("class C; if ({sequence}); nil; else; ({sequence});end;end");
        let parsed = ruby_prism::parse(source.as_bytes());
        assert_eq!(parsed.errors().count(), 0);
        let mut tree = Tree::default();
        walk::walk(&mut tree, &parsed.node());
        assert!(tree.raw.len() > 2000);
        assert!(
            tree.macro_branches.len() < 10,
            "{}",
            tree.macro_branches.len()
        );
        assert_eq!(tree.raw.iter().filter(|r| r.bare).count(), 2000);
    }
    #[test]
    fn receiver_collisions_and_synthetic_getters_keep_distinct_report_ranges() {
        // A lone private_class_method (or compound assignment) is not a bare
        // modifier, so the pinned single-expression callback does not flag it.
        assert!(offenses("class C;private_class_method.private_class_method;end").is_empty());
        let found = offenses("class C;nil;private_class_method.private_class_method;end");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 13, 1));
        let found = offenses("class C\n nil\n  obj.\n    private_class_method +=\n    value\nend");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 3, 4));
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn pinned_method_creators_and_lookalikes() {
        for body in [
            "private; def f;end",
            "protected; attr_reader :x",
            "private; attr_accessor :x",
            "private; attr :x, true",
            "private; attr_writer :x",
            "private; helper def f;end",
            "private; define_method(:f) {}",
            "private; [1,2].each { define_method(:f) {} }",
            "private; if condition; def f;end;end",
            "private :f",
            "private def f;end",
            "private_class_method def self.f;end",
            "self.private",
            "private = 1; private",
        ] {
            let source = format!("class C; {body}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn singleton_eval_constructor_and_default_dsl_scopes() {
        for opening in [
            "class C",
            "module C",
            "class << self",
            "C.class_eval do",
            "C.instance_eval do",
            "Class.new do",
            "::Module.new do",
            "Struct.new(:x) do",
            "Data.define(:x) do",
            "::Data.define(:x) do",
        ] {
            let source = format!("{opening}\n private\nend\n");
            assert_eq!(offenses(&source).len(), 1, "{source}");
            let source = format!("{opening}\n private\n def f;end\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        let source = "class C\n included do\n private\n def f;end\n end\n private\n def g;end\nend";
        let found = offenses(source);
        assert_eq!(found.iter().map(|o| o.line).collect::<Vec<_>>(), [6]);
    }
}
