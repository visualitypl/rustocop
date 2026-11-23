//! `Style/MissingRespondToMissing` — lexical companion method presence.
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::{
    collections::HashSet,
    mem::{Discriminant, discriminant},
};
pub struct MissingRespondToMissing;
impl Cop for MissingRespondToMissing {
    fn name(&self) -> &'static str {
        "Style/MissingRespondToMissing"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node
            .as_def_node()
            .filter(|d| d.name().as_slice() == b"method_missing")
        else {
            return;
        };
        let scope = cx
            .ancestors()
            .find(lexical_scope)
            .or_else(|| cx.ancestors().find(search_parent));
        if let Some(scope) = scope
            && file
                .respond_to_missing_index(cx.ancestors().last().unwrap_or(node))
                .responders
                .contains(&key(scope, def.receiver().is_some()))
        {
            return;
        }
        let (line, col) = file.line_col(def.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(def.location()),
            message: "When using `method_missing`, define `respond_to_missing?`.".into(),
            correction: None,
        });
    }
}
fn lexical_scope(node: &&Node<'_>) -> bool {
    matches!(
        node,
        Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. }
    )
}
fn search_parent(node: &&Node<'_>) -> bool {
    // Prism's program/argument/single-statement wrappers are transparent in
    // Parser's AST. Multiple statements correspond to its synthetic begin.
    !matches!(
        node,
        Node::ProgramNode { .. } | Node::ArgumentsNode { .. } | Node::ElseNode { .. }
    ) && !node
        .as_statements_node()
        .is_some_and(|s| s.body().len() == 1)
}
// Implicit begins can share a source range with their enclosing block. Keep
// the node kind too, so these distinct Parser-style scopes never alias.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    start: usize,
    end: usize,
    kind: Discriminant<Node<'static>>,
    singleton: bool,
}
fn key(node: &Node<'_>, singleton: bool) -> Key {
    Key {
        start: node.location().start_offset(),
        end: node.location().end_offset(),
        // SAFETY: Discriminant contains only the enum variant, not a Node or
        // references. Erasing only free lifetimes is explicitly supported by
        // std::mem::discriminant; the cache never retains the parsed AST.
        kind: unsafe {
            std::mem::transmute::<Discriminant<Node<'_>>, Discriminant<Node<'static>>>(
                discriminant(node),
            )
        },
        singleton,
    }
}
// One cached main-AST walk. Each companion contributes only to its lexical
// scope (or Parser-style parent ancestors at top level, bounded by AST depth).
// No project index, reopened-class merging, alias or runtime receiver inference.
#[derive(Default)]
pub(crate) struct CompanionIndex {
    responders: HashSet<Key>,
    stack: Vec<Frame>,
    #[cfg(test)]
    pub(crate) visited: usize,
}
struct Frame {
    key: Key,
    lexical: bool,
    searchable: bool,
    // Parser's blocks/ensures wrap their send/rescue and body as siblings;
    // Prism nests them under the call/begin. Its rescue clauses are likewise
    // linked children rather than Parser's sibling resbody nodes.
    exclude_child: Option<Key>,
    wrapper: Option<Key>,
}
impl CompanionIndex {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut index = Self::default();
        walk(&mut index, root);
        index
    }
}
impl<'pr> NodeVisitor<'pr> for CompanionIndex {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visited += 1;
        }
        if let Some(def) = node
            .as_def_node()
            .filter(|d| d.name().as_slice() == b"respond_to_missing?")
        {
            let singleton = def.receiver().is_some();
            if let Some(scope) = self.stack.iter().rev().find(|frame| frame.lexical) {
                self.responders.insert(Key {
                    singleton,
                    ..scope.key
                });
            } else {
                for (i, parent) in self.stack.iter().enumerate() {
                    let excluded = parent.exclude_child.is_some_and(|block| {
                        self.stack
                            .get(i + 1)
                            .is_some_and(|child| child.key == block)
                    });
                    if parent.searchable && !excluded {
                        self.responders.insert(Key {
                            singleton,
                            ..parent.key
                        });
                    }
                    if let Some(block) = parent.wrapper {
                        self.responders.insert(Key { singleton, ..block });
                    }
                }
            }
        }
        let (exclude_child, wrapper) = if let Some(begin) = node.as_begin_node() {
            let ensure = begin.ensure_clause().map(|n| key(&n.as_node(), false));
            (begin.rescue_clause().and(ensure), ensure)
        } else if let Some(rescue) = node.as_rescue_node() {
            (rescue.subsequent().map(|n| key(&n.as_node(), false)), None)
        } else {
            let block = node
                .as_call_node()
                .and_then(|c| c.block())
                .or_else(|| node.as_super_node().and_then(|n| n.block()))
                .or_else(|| {
                    node.as_forwarding_super_node()
                        .and_then(|n| n.block())
                        .map(|n| n.as_node())
                })
                .filter(|b| b.as_block_node().is_some())
                .map(|b| key(&b, false));
            (block, block)
        };
        self.stack.push(Frame {
            key: key(&node, false),
            lexical: lexical_scope(&&node),
            searchable: search_parent(&&node),
            exclude_child,
            wrapper,
        });
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}

/// Main-AST lexical companion-method presence, indexed at most once.
pub(crate) static RESPOND_TO_MISSING_INDEX: crate::source::Fact<CompanionIndex> =
    crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn respond_to_missing_index(&self, root: &ruby_prism::Node<'_>) -> &CompanionIndex {
        RESPOND_TO_MISSING_INDEX.get(self, || CompanionIndex::build(root))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/missing_respond_to_missing_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn found(source: &str) -> Vec<crate::offense::Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == MissingRespondToMissing.name())
            .collect()
    }
    #[test]
    fn pinned_missing_methods_and_exact_locations() {
        for (source, line, col, end_line) in [
            ("class Test\n  def method_missing\n  end\nend", 2, 3, 3),
            ("class Test\n  def self.method_missing\n  end\nend", 2, 3, 3),
            ("def method_missing(name)\nend", 1, 1, 2),
            (
                "module M\n  private def method_missing(name, *args)\n  end\nend",
                2,
                11,
                3,
            ),
            (
                "class << self\n def method_missing(*) = super\nend",
                2,
                2,
                2,
            ),
        ] {
            let found = found(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line)
            );
            assert_eq!(
                found[0].message,
                "When using `method_missing`, define `respond_to_missing?`."
            );
            assert!(!MissingRespondToMissing.safe_fixes());
            assert!(
                correction_offenses(&MissingRespondToMissing, source)[0]
                    .correction
                    .is_none()
            );
        }
    }
    #[test]
    fn pinned_companions_inline_visibility_and_order() {
        for wrapper in ["", "class Test;", "module Test;", "class << self;"] {
            for receiver in ["", "self."] {
                for modifier in ["", "private "] {
                    for reverse in [false, true] {
                        let a = format!("{modifier}def {receiver}method_missing(name); end");
                        let b = format!(
                            "private def {receiver}respond_to_missing?(name, include_private = false); end"
                        );
                        let body = if reverse {
                            format!("{a}\n{b}")
                        } else {
                            format!("{b}\n{a}")
                        };
                        let source = if wrapper.is_empty() {
                            body
                        } else {
                            format!("{wrapper}\n{body}\nend")
                        };
                        let expected = usize::from(wrapper.is_empty() && !modifier.is_empty());
                        assert_eq!(found(&source).len(), expected, "{source}");
                    }
                }
            }
        }
    }
    #[test]
    fn pinned_scope_and_definition_kind_must_match() {
        for source in [
            "class Test; def self.method_missing; end; def respond_to_missing?; end; end",
            "class Test; def method_missing; end; def self.respond_to_missing?; end; end",
            "class Outer; class A; def method_missing; end; end; class B; def respond_to_missing?; end; end; end",
            "module M; def method_missing; end; module N; def respond_to_missing?; end; end; end",
            "class C; def method_missing; end; class << self; def respond_to_missing?; end; end; end",
            "class C; class << self; def method_missing; end; end; def respond_to_missing?; end; end",
            "class C; def method_missing; end; end; class C; def respond_to_missing?; end; end",
            "def method_missing; end; class C; def respond_to_missing?; end; end",
            "class Base; def respond_to_missing?; end; end; class C < Base; def method_missing; end; end",
        ] {
            assert_eq!(found(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn dynamic_alias_and_string_lookalikes_do_not_introduce_def_nodes() {
        for source in [
            "define_method(:method_missing) { super }",
            "define_singleton_method(:method_missing) { super }",
            "alias method_missing handler",
            "class_eval('def method_missing; end')",
            "def method_missing_else; end",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        for responder in [
            "define_method(:respond_to_missing?) { true }",
            "alias respond_to_missing? handler",
            "class_eval('def respond_to_missing?; end')",
        ] {
            assert_eq!(
                found(&format!(
                    "class C; def method_missing; end; {responder}; end"
                ))
                .len(),
                1
            );
        }
    }
    #[test]
    fn rescue_siblings_else_and_ensure_use_parser_style_parents() {
        for (source, expected) in [
            (
                "begin; nil; rescue A; def method_missing; end; rescue B; def respond_to_missing?; end; end",
                1,
            ),
            (
                "begin; def respond_to_missing?; end; rescue; nil; else; def method_missing; end; end",
                0,
            ),
            (
                "begin; def method_missing; end; rescue; nil; ensure; def respond_to_missing?; end; end",
                1,
            ),
            (
                "begin; def respond_to_missing?; end; rescue; nil; ensure; def method_missing; end; end",
                0,
            ),
            (
                "if x; def respond_to_missing?; end; else; def method_missing; end; end",
                0,
            ),
        ] {
            assert_eq!(found(source).len(), expected, "{source}");
        }
    }

    #[test]
    fn implicit_block_rescue_and_ensure_scopes_do_not_alias_the_block() {
        for (source, expected) in [
            (
                "foo do; def method_missing; end; rescue; nil; ensure; def respond_to_missing?; end; end",
                1,
            ),
            (
                "foo do; def respond_to_missing?; end; rescue; nil; ensure; def method_missing; end; end",
                0,
            ),
            (
                "foo do; def method_missing; end; def respond_to_missing?; end; rescue; nil; ensure; nil; end",
                0,
            ),
            (
                "foo do; def method_missing; end; rescue A; nil; rescue B; def respond_to_missing?; end; ensure; nil; end",
                0,
            ),
        ] {
            let offenses = found(source);
            assert_eq!(offenses.len(), expected, "{source}");
            if expected == 1 {
                assert_eq!((offenses[0].line, offenses[0].col), (1, 9));
            }
        }
    }

    #[test]
    fn literal_block_wrappers_keep_send_arguments_separate_from_the_body() {
        for (source, expected) in [
            (
                "foo(def method_missing; end) { def respond_to_missing?; end }",
                1,
            ),
            (
                "foo(def respond_to_missing?; end) { def method_missing; end }",
                0,
            ),
            (
                "foo(def respond_to_missing?; end) { def method_missing; end; bar }",
                1,
            ),
            (
                "(def method_missing; end).foo { def respond_to_missing?; end }",
                1,
            ),
            (
                "(def respond_to_missing?; end).foo { def method_missing; end }",
                0,
            ),
        ] {
            assert_eq!(found(source).len(), expected, "{source}");
        }
    }

    #[test]
    fn lexical_presence_not_runtime_receiver_or_control_flow_inference() {
        for source in [
            "class C; def a.method_missing; end; def b.respond_to_missing?; end; end",
            "class C; def method_missing; end; if false; private def respond_to_missing?; end; end; end",
            "Class.new { def method_missing; end; def respond_to_missing?; end }",
            "class C; class_eval { def method_missing; end }; def respond_to_missing?; end; end",
        ] {
            assert!(found(source).is_empty(), "{source}");
        }
        // At top level upstream searches the immediate AST parent, not the file.
        assert_eq!(
            found("Class.new { def method_missing; end }; def respond_to_missing?; end").len(),
            1
        );
        assert_eq!(
            found("private def method_missing; end; def respond_to_missing?; end").len(),
            1
        );
    }
}
