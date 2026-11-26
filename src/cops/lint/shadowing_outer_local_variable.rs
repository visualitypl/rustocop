//! `Lint/ShadowingOuterLocalVariable` — explicit declarations hiding visible locals.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        variables::{ScopeAnalysis, Variable},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::{cell::RefCell, collections::HashMap, mem::Discriminant};

/// Opt-in, detection-only local variable shadowing.
pub struct ShadowingOuterLocalVariable;
impl Cop for ShadowingOuterLocalVariable {
    fn name(&self) -> &'static str {
        "Lint/ShadowingOuterLocalVariable"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(program) = node.as_program_node() else {
            return;
        };
        let analysis = ScopeAnalysis::for_outer_shadowing(&program, file);
        let syntax = Syntax::of(node);
        for (inner, outer) in analysis.shadowing_declarations() {
            let variable = analysis.variable(inner);
            if variable.should_be_unused() {
                continue;
            }
            let scope = syntax.indices[&key(&analysis.scope(variable.scope).node)];
            if syntax.ractor(scope) {
                continue;
            }
            let outer = syntax.declaration(analysis.variable(outer));
            // Structural ancestry, NOT physical byte containment: a heredoc
            // interpolation can be outside the declaration's source range.
            if (matches!(
                syntax.entries[outer].node,
                Node::LocalVariableWriteNode { .. }
                    | Node::MatchWriteNode { .. }
                    | Node::OptionalParameterNode { .. }
                    | Node::OptionalKeywordParameterNode { .. }
            ) && syntax.ancestors(scope).any(|i| syntax.eq(i, outer, file)))
                || syntax.exclusive(scope, outer, file)
            {
                continue;
            }
            let range = &variable.declaration_range;
            let (line, col) = file.reference_line_col(range.start);
            let name: String = file
                .match_pattern_source(variable.name)
                .utf8_chunks()
                .map(|chunk| {
                    let mut text = chunk.valid().to_owned();
                    for byte in chunk.invalid() {
                        text.push_str(&format!("\\x{byte:02X}"));
                    }
                    text
                })
                .collect();
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!("Shadowing outer local variable - `{name}`."),
                line,
                col,
                end_line: file.line_of(range.end.saturating_sub(1).max(range.start)),
            });
        }
    }
}

type Key<'pr> = (usize, usize, Discriminant<Node<'pr>>);
fn key<'pr>(node: &Node<'pr>) -> Key<'pr> {
    let loc = node.location();
    (
        loc.start_offset(),
        loc.end_offset(),
        std::mem::discriminant(node),
    )
}
struct Entry<'pr> {
    node: Node<'pr>,
    parent: Option<usize>,
}
/// One complete syntax walk retaining actual edges and declaration identities.
struct Syntax<'pr> {
    entries: Vec<Entry<'pr>>,
    indices: HashMap<Key<'pr>, usize>,
    declarations: HashMap<(usize, usize), usize>,
    fingerprints: RefCell<HashMap<usize, ast::Fingerprint<'pr>>>,
}
impl<'pr> Syntax<'pr> {
    fn of(root: &Node<'pr>) -> Self {
        struct Builder<'pr> {
            syntax: Syntax<'pr>,
            stack: Vec<usize>,
        }
        impl<'pr> NodeVisitor<'pr> for Builder<'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                let i = self.syntax.entries.len();
                let loc = if let Some(n) = node.as_local_variable_write_node() {
                    Some(n.name_loc())
                } else if let Some(n) = node.as_local_variable_operator_write_node() {
                    Some(n.name_loc())
                } else if let Some(n) = node.as_local_variable_or_write_node() {
                    Some(n.name_loc())
                } else if let Some(n) = node.as_local_variable_and_write_node() {
                    Some(n.name_loc())
                } else if let Some(n) = node.as_match_write_node() {
                    n.call().receiver().map(|n| n.location())
                } else if matches!(
                    node,
                    Node::LocalVariableTargetNode { .. }
                        | Node::RequiredParameterNode { .. }
                        | Node::OptionalParameterNode { .. }
                        | Node::RestParameterNode { .. }
                        | Node::RequiredKeywordParameterNode { .. }
                        | Node::OptionalKeywordParameterNode { .. }
                        | Node::KeywordRestParameterNode { .. }
                        | Node::BlockParameterNode { .. }
                        | Node::BlockLocalVariableNode { .. }
                ) {
                    Some(node.location())
                } else {
                    None
                };
                if let Some(loc) = loc {
                    self.syntax
                        .declarations
                        .insert((loc.start_offset(), loc.end_offset()), i);
                }
                self.syntax.indices.insert(key(&node), i);
                self.syntax.entries.push(Entry {
                    node,
                    parent: self.stack.last().copied(),
                });
                self.stack.push(i);
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        let mut builder = Builder {
            syntax: Self {
                entries: Vec::new(),
                indices: HashMap::new(),
                declarations: HashMap::new(),
                fingerprints: RefCell::new(HashMap::new()),
            },
            stack: Vec::new(),
        };
        walk(&mut builder, root);
        builder.syntax
    }
    fn declaration(&self, variable: &Variable<'pr>) -> usize {
        self.declarations[&(
            variable.declaration_range.start,
            variable.declaration_range.end,
        )]
    }
    fn ancestors(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.entries[i].parent, |i| self.entries[*i].parent)
    }
    fn ractor(&self, scope: usize) -> bool {
        let node = &self.entries[scope].node;
        let Some(block) = node.as_block_node() else {
            return false;
        };
        if matches!(
            block.parameters(),
            Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
        ) {
            return false;
        }
        self.entries[scope]
            .parent
            .and_then(|p| self.entries[p].node.as_call_node())
            .is_some_and(|call| {
                !call.is_safe_navigation()
                    && call.name().as_slice() == b"new"
                    && call
                        .receiver()
                        .and_then(|n| n.as_constant_read_node())
                        .is_some_and(|n| n.name().as_slice() == b"Ractor")
            })
    }
    /// RuboCop has no Program/Arguments/Else wrappers; a single statement is
    /// its body, while a multi-statement body is a real begin node. A literal
    /// block replaces its Call as the expression's outermost node.
    fn parent(&self, mut i: usize) -> Option<usize> {
        if self.entries[i].node.as_block_node().is_some() {
            i = self.entries[i].parent?;
        }
        let mut p = self.entries[i].parent?;
        loop {
            let n = &self.entries[p].node;
            let transparent = matches!(
                n,
                Node::ProgramNode { .. } | Node::ArgumentsNode { .. } | Node::ElseNode { .. }
            ) || n.as_statements_node().is_some_and(|n| {
                let parent = self.entries[p].parent.map(|p| &self.entries[p].node);
                let handler = parent
                    .and_then(|p| p.as_begin_node())
                    .is_some_and(|n| n.rescue_clause().is_some() || n.ensure_clause().is_some());
                !handler
                    && (n.body().len() == 1
                        || parent.is_some_and(|p| {
                            p.as_parentheses_node().is_some() || p.as_begin_node().is_some()
                        }))
            });
            if !transparent {
                return Some(p);
            }
            p = self.entries[p].parent?;
        }
    }
    fn nearest(&self, mut i: usize, predicate: impl Fn(&Node<'pr>) -> bool) -> Option<usize> {
        while let Some(p) = self.parent(i) {
            if predicate(&self.entries[p].node) && !self.guard(p) {
                return Some(p);
            }
            i = p;
        }
        None
    }
    fn guard(&self, i: usize) -> bool {
        self.entries[i].parent.is_some_and(|p| {
            self.entries[p]
                .node
                .as_in_node()
                .is_some_and(|n| key(&n.pattern()) == key(&self.entries[i].node))
        })
    }
    fn expression(&self, i: usize) -> usize {
        if self.entries[i].node.as_block_node().is_some() {
            self.entries[i].parent.unwrap_or(i)
        } else {
            i
        }
    }
    fn eq(&self, a: usize, b: usize, file: &SourceFile) -> bool {
        let (a, b) = (self.expression(a), self.expression(b));
        if a == b {
            return true;
        }
        let (left, right) = (&self.entries[a].node, &self.entries[b].node);
        let if_node = |n: &Node<'_>| matches!(n, Node::IfNode { .. } | Node::UnlessNode { .. });
        if std::mem::discriminant(left) != std::mem::discriminant(right)
            && !(if_node(left) && if_node(right))
        {
            return false;
        }
        let mut fingerprints = self.fingerprints.borrow_mut();
        for i in [a, b] {
            fingerprints.entry(i).or_insert_with(|| {
                ast::fingerprint_for_file_operation(&self.entries[i].node, file, None)
            });
        }
        fingerprints[&a].source_buffer_scope_eq(&fingerprints[&b], file)
    }
    fn exclusive(&self, scope: usize, outer: usize, file: &SourceFile) -> bool {
        if let (Some(a), Some(b)) = (
            self.nearest(scope, |n| n.as_in_node().is_some()),
            self.nearest(outer, |n| n.as_in_node().is_some()),
        ) && !self.eq(a, b, file)
            && self
                .parent(a)
                .zip(self.parent(b))
                .is_some_and(|(a, b)| self.eq(a, b, file))
        {
            return true;
        }
        let Some(mut variable) = self.parent(scope) else {
            return false;
        };
        if self.entries[variable].node.as_when_node().is_some() {
            let Some(p) = self.parent(variable) else {
                return false;
            };
            variable = p;
        }
        if (!conditional(&self.entries[variable].node) || self.guard(variable))
            && self.nearest(variable, conditional).is_none()
        {
            return false;
        }
        let Some(condition) = self.nearest(outer, conditional) else {
            return false;
        };
        if self.eq(variable, condition, file) {
            return true;
        }
        let node = &self.entries[condition].node;
        let branch = if let Some(n) = node.as_if_node() {
            n.subsequent()
        } else if let Some(n) = node.as_unless_node() {
            n.else_clause().map(|n| n.as_node())
        } else {
            None
        };
        let Some(mut branch) = branch else {
            return false;
        };
        if let Some(n) = branch.as_else_node() {
            let Some(n) = n.statements() else {
                return false;
            };
            branch = n.as_node();
        }
        if let Some(n) = branch.as_statements_node()
            && n.body().len() == 1
        {
            branch = n.body().iter().next().unwrap();
        }
        self.eq(variable, self.indices[&key(&branch)], file)
    }
}
fn conditional(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::CaseNode { .. }
            | Node::CaseMatchNode { .. }
    ) || n.as_while_node().is_some_and(|n| !n.is_begin_modifier())
        || n.as_until_node().is_some_and(|n| !n.is_begin_modifier())
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's shadowing_outer_local_variable_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(&format!(
            "# rustocop:enable Lint/ShadowingOuterLocalVariable\n{source}"
        ))
        .into_iter()
        .filter(|o| o.cop == "Lint/ShadowingOuterLocalVariable")
        .collect()
    }
    #[test]
    fn reports_explicit_parameter_and_block_local_declarations() {
        for (param, col) in [("foo", 15), ("*foo", 15), ("&foo", 15), ("i; foo", 18)] {
            let source = format!("def f(foo)\n  1.times do |{param}|\n  end\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, col));
            assert_eq!(found[0].message, "Shadowing outer local variable - `foo`.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn conditional_exemptions_use_exact_parents_not_general_branch_flow() {
        for source in [
            "if flag; foo=1; else; xs.each { |foo| }; end",
            "if flag; foo=1; else; nil; xs.each { |foo| }; end",
            "if flag; foo=1; else; consume { xs.each { |foo| } }; end",
            "unless flag; foo=1; else; xs.each { |foo| }; end",
            "case flag; when 1; foo=1; when 2; xs.each { |foo| }; end",
            "case flag; in [foo]; nil; in Integer; xs.each { |foo| }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "if flag; foo=1; xs.each { |foo| }; end",
            "unless flag; foo=1; xs.each { |foo| }; end",
            "if flag; foo=1; else; consume([xs.each { |foo| }]); end",
            "if flag; foo=1; else; begin; xs.each { |foo| }; rescue; nil; end; end",
            "case flag; in [foo] if xs.each { |foo| }; nil; end",
            "case flag; in [foo]; xs.each { |foo| }; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn ractor_exemption_is_only_an_ordinary_unrooted_constructor_block() {
        assert!(offenses("foo=1; Ractor.new { |foo| }").is_empty());
        for call in [
            "::Ractor.new",
            "Ractor&.new",
            "Other::Ractor.new",
            "ractor.new",
        ] {
            assert_eq!(
                offenses(&format!("foo=1; {call} {{ |foo| }}")).len(),
                1,
                "{call}"
            );
        }
        for implicit in ["it", "_1"] {
            assert_eq!(
                offenses(&format!(
                    "foo=1; Ractor.new {{ {implicit}; xs.each {{ |foo| }} }}"
                ))
                .len(),
                1
            );
            assert!(offenses(&format!("xs.each {{ {implicit}; xs.each {{ |it| }} }}")).is_empty());
        }
    }
    #[test]
    fn rhs_declaration_ownership_is_structural_including_heredocs() {
        for source in [
            "foo=consume(<<TEXT)\n#{xs.each { |foo| }}\nTEXT\n",
            "foo=xs.each { |foo| }; foo=xs.each { |foo| }",
            "foo,bar=xs.each { |foo| }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "foo=1; consume(<<TEXT)\n#{xs.each { |foo| }}\nTEXT\n",
            "foo ||= xs.each { |foo| }",
            "foo += xs.each { |foo| }",
            "foo=1; foo,bar=xs.each { |foo| }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn full_optional_declaration_ranges_preserve_multiline_end_lines() {
        let found = offenses("foo=1\nxs.each { |foo=(\n1\n)| }\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 12, 5));
        assert_eq!(found[0].message, "Shadowing outer local variable - `foo`.");
    }
    #[test]
    fn defaults_see_only_earlier_declarations_and_closures_do_not_leak() {
        assert!(offenses("xs.each { |bar=xs.each { |foo| },foo=1| }").is_empty());
        assert_eq!(
            offenses("xs.each { |foo=1,bar=xs.each { |foo| }| }").len(),
            1
        );
        for source in [
            "xs.each { foo=1 }.each { |foo| }",
            "foo=1; class A; xs.each { |foo| }; end",
            "foo=1; module A; xs.each { |foo| }; end",
            "foo=1; class << self; xs.each { |foo| }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn preserves_nonshadowing_scope_and_rhs_lookalikes() {
        for source in [
            "foo=1; xs.each { |bar| bar }",
            "foo=1; xs.each { foo=2 }",
            "xs.each { |foo| }; foo=1",
            "foo=xs.each { |foo| foo }",
            "_foo=1; xs.each { |_foo| }",
            "foo=1; def f(foo); end",
            "def f(*foo); Ractor.new(*foo) { |*foo| }; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
