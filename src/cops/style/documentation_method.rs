//! `Style/DocumentationMethod` — fixed public-method documentation policy.
use std::collections::HashMap;

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};

pub struct DocumentationMethod;
impl Cop for DocumentationMethod {
    fn name(&self) -> &'static str {
        "Style/DocumentationMethod"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some(def) = node.as_def_node() {
            if def.name().as_slice() == b"initialize"
                || cx
                    .ancestors()
                    .find(|n| n.as_arguments_node().is_none())
                    .is_some_and(|parent| modifier(parent).is_some())
            {
                return;
            }
        } else if let Some(call) = modifier(node) {
            // Report the wrapper once, even with multiple direct def args.
            // Its docs/visibility do not depend on which def triggered it.
            if !ast::call_arguments(&call).iter().any(|arg| {
                arg.as_def_node()
                    .is_some_and(|def| def.name().as_slice() != b"initialize")
            }) {
                return;
            }
        } else {
            return;
        }
        if inline_non_public(cx)
            || file
                .method_visibility()
                .lock()
                .unwrap()
                .non_public(node, cx)
            || super::documentation::documentation_comment(node, cx, file)
        {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Missing method documentation comment.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}

fn modifier<'pr>(node: &Node<'pr>) -> Option<CallNode<'pr>> {
    node.as_call_node().filter(|call| {
        call.receiver().is_none()
            && matches!(
                call.name().as_slice(),
                b"module_function" | b"ruby2_keywords"
            )
    })
}

fn inline_non_public(cx: &NodeContext<'_, '_>) -> bool {
    cx.ancestors().find(|n| n.as_arguments_node().is_none()).is_some_and(|parent| {
        parent.as_call_node().is_some_and(|call| {
            call.receiver().is_none()
                && matches!(call.name().as_slice(), b"private" | b"protected" | b"private_class_method")
                && matches!(ast::call_arguments(&call).as_slice(), [arg] if arg.as_def_node().is_some())
        })
    })
}

fn visibility_call<'pr>(node: &Node<'pr>, block_send: bool) -> Option<CallNode<'pr>> {
    node.as_call_node().filter(|call| {
        call.receiver().is_none()
            && matches!(
                call.name().as_slice(),
                b"private" | b"protected" | b"public"
            )
            && (block_send
                || !call
                    .block()
                    .is_some_and(|block| block.as_block_node().is_some()))
    })
}

// A lazy per-container index, not a left/right sibling scan for every def.
// Normal statement/argument lists are enumerated directly. The fallback for
// fixed-arity expression parents uses the shared complete walker; summed work
// is bounded by the pipeline's fixed AST-depth budget, never by sibling count
// squared. It does not reparse or infer runtime visibility across definitions.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum SiblingScope {
    Direct,
    Rescue,
    RescueExceptions,
}

#[derive(Default)]
pub(crate) struct MethodVisibility {
    containers: HashMap<(String, usize, usize, SiblingScope), HashMap<usize, bool>>,
    #[cfg(test)]
    sibling_visits: usize,
}
impl MethodVisibility {
    fn non_public(&mut self, node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
        let mut ancestors = cx.ancestors();
        let mut in_ensure = false;
        let Some(parent) = ancestors.find(|ancestor| {
            if ancestor.as_ensure_node().is_some() {
                in_ensure = true;
                return false;
            }
            ancestor.as_arguments_node().is_none()
                && ancestor.as_else_node().is_none()
                && !ancestor
                    .as_statements_node()
                    .is_some_and(|s| s.body().len() == 1)
        }) else {
            return false;
        };
        // Prism stores rescue and ensure on the same BeginNode. Parser nests
        // the rescue inside the ensure, so they need separate cached indexes.
        let scope = if parent
            .as_begin_node()
            .is_some_and(|begin| begin.rescue_clause().is_some())
            && !in_ensure
        {
            SiblingScope::Rescue
        } else if parent.as_rescue_node().is_some_and(|rescue| {
            // O(1) bounds check: direct exception children belong to Parser's
            // synthetic array, not to the resbody containing that array.
            let exceptions = rescue.exceptions();
            exceptions.first().is_some_and(|first| {
                first.location().start_offset() <= node.location().start_offset()
            }) && exceptions
                .last()
                .is_some_and(|last| node.location().end_offset() <= last.location().end_offset())
        }) {
            SiblingScope::RescueExceptions
        } else {
            SiblingScope::Direct
        };
        let enclosing = ancestors.next();
        let block_send = parent.as_block_node().is_some();
        // Discriminant's lifetime is tied to Prism's parse; retain only its
        // unique debug tag, not a node/pointer borrowed from the parse.
        let key = (
            format!("{:?}", std::mem::discriminant(parent)),
            parent.location().start_offset(),
            parent.location().end_offset(),
            scope,
        );
        let index = self.containers.entry(key).or_insert_with(|| {
            let siblings = siblings(parent, enclosing, scope);
            #[cfg(test)]
            {
                self.sibling_visits += siblings.len();
            }
            let mut preceding = false;
            let mut index = HashMap::new();
            for sibling in &siblings {
                index.insert(sibling.location().start_offset(), preceding);
                if let Some(call) = visibility_call(sibling, block_send)
                    && ast::call_arguments(&call).is_empty()
                {
                    preceding = call.name().as_slice() != b"public";
                }
            }
            // Named visibility is considered only for instance defs at this
            // pin. Last matching *right* sibling wins, even over a preceding
            // blanket public/private declaration. Strings are not symbols.
            let mut named = HashMap::new();
            for sibling in siblings.iter().rev() {
                if let Some(def) = sibling.as_def_node()
                    && def.receiver().is_none()
                {
                    let inline = siblings_inline_visibility(sibling, parent);
                    if let Some(non_public) =
                        inline.or_else(|| named.get(def.name().as_slice()).copied())
                    {
                        index.insert(sibling.location().start_offset(), non_public);
                    }
                }
                if let Some(call) = visibility_call(sibling, block_send) {
                    for arg in ast::call_arguments(&call) {
                        if let Some(symbol) = arg.as_symbol_node() {
                            named
                                .entry(symbol.unescaped().to_vec())
                                .or_insert(call.name().as_slice() != b"public");
                        }
                    }
                }
            }
            index
        });
        index
            .get(&node.location().start_offset())
            .copied()
            .unwrap_or(false)
    }
}

fn siblings_inline_visibility(node: &Node<'_>, parent: &Node<'_>) -> Option<bool> {
    // Inline public/private/protected instance defs outrank named siblings.
    // Singleton defs deliberately do not use this VisibilityHelp branch.
    parent.as_call_node().filter(|call| call.receiver().is_none()
        && matches!(call.name().as_slice(), b"public" | b"protected" | b"private")
        && matches!(ast::call_arguments(call).as_slice(), [arg] if arg.location().start_offset() == node.location().start_offset() && arg.as_def_node().is_some_and(|d| d.receiver().is_none())))
        .map(|call| call.name().as_slice() != b"public")
}

fn body_sibling<'pr>(body: ruby_prism::StatementsNode<'pr>) -> Node<'pr> {
    if body.body().len() == 1 {
        body.body().first().unwrap()
    } else {
        body.as_node()
    }
}

fn siblings<'pr>(
    parent: &Node<'pr>,
    enclosing: Option<&Node<'pr>>,
    scope: SiblingScope,
) -> Vec<Node<'pr>> {
    if let Some(statements) = parent.as_statements_node() {
        return statements.body().iter().collect();
    }
    if let Some(call) = parent.as_call_node() {
        return call
            .receiver()
            .into_iter()
            .chain(ast::call_arguments(&call))
            .collect();
    }
    if let Some(array) = parent.as_array_node() {
        return array.elements().iter().collect();
    }
    if let Some(block) = parent.as_block_node() {
        // Parser's block has [send, args, body]; Prism puts the block under
        // its call. Keep the send (without treating its block as a sibling).
        return enclosing
            .and_then(|node| node.as_call_node())
            .map(|call| call.as_node())
            .into_iter()
            .chain(block.parameters())
            .chain(
                block
                    .body()
                    .map(|body| body.as_statements_node().map_or(body, body_sibling)),
            )
            .collect();
    }
    if let Some(rescue) = parent.as_rescue_node() {
        if scope == SiblingScope::RescueExceptions {
            return rescue.exceptions().iter().collect();
        }
        // Parser groups exceptions in an array. None of its expressions can
        // act as a visibility send beside the resbody's reference and body.
        return rescue
            .reference()
            .into_iter()
            .chain(rescue.statements().map(body_sibling))
            .collect();
    }
    if let Some(begin) = parent.as_begin_node()
        && scope == SiblingScope::Rescue
    {
        return begin
            .statements()
            .map(body_sibling)
            .into_iter()
            .chain(begin.rescue_clause().map(|rescue| rescue.as_node()))
            .chain(
                begin
                    .else_clause()
                    .and_then(|clause| clause.statements())
                    .map(body_sibling),
            )
            .collect();
    }
    if let Some(begin) = parent.as_begin_node()
        && let Some(ensure) = begin.ensure_clause()
    {
        let mut nodes = Vec::new();
        // Parser's ensure has the complete protected expression as one child,
        // not its statement/rescue children flattened into sibling sends.
        if begin.rescue_clause().is_none()
            && begin.else_clause().is_none()
            && let Some(body) = begin.statements()
        {
            if body.body().len() == 1 {
                nodes.extend(body.body().iter());
            } else {
                nodes.push(body.as_node());
            }
        }
        if let Some(body) = ensure.statements() {
            if body.body().len() == 1 {
                nodes.extend(body.body().iter());
            } else {
                nodes.push(body.as_node());
            }
        }
        return nodes;
    }
    struct Children<'pr> {
        depth: usize,
        nodes: Vec<Node<'pr>>,
    }
    impl<'pr> NodeVisitor<'pr> for Children<'pr> {
        fn enter(&mut self, node: Node<'pr>) {
            // Single statement lists and argument lists have no Parser node.
            if self.depth == 1 {
                if let Some(s) = node.as_statements_node() {
                    if s.body().len() == 1 {
                        self.nodes.extend(s.body().iter());
                    } else {
                        self.nodes.push(node);
                    }
                } else if let Some(otherwise) = node.as_else_node() {
                    if let Some(body) = otherwise.statements() {
                        if body.body().len() == 1 {
                            self.nodes.extend(body.body().iter());
                        } else {
                            self.nodes.push(body.as_node());
                        }
                    }
                } else if let Some(args) = node.as_arguments_node() {
                    self.nodes.extend(args.arguments().iter());
                } else {
                    self.nodes.push(node);
                }
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
    walk(&mut visitor, parent);
    visitor.nodes
}

/// Per-container method visibility, indexed lazily from NodeContext parents.
pub(crate) static METHOD_VISIBILITY: crate::source::Fact<std::sync::Mutex<MethodVisibility>> =
    crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn method_visibility(&self) -> &std::sync::Mutex<MethodVisibility> {
        METHOD_VISIBILITY.get(self, Default::default)
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/documentation_method_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str_with;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&DocumentationMethod], source)
            .into_iter()
            .filter(|o| o.cop == "Style/DocumentationMethod")
            .collect()
    }
    fn assert_offense(source: &str, line: u32, col: u32, end_line: u32) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!(
            (found[0].line, found[0].col, found[0].end_line),
            (line, col, end_line),
            "{source}"
        );
        assert_eq!(found[0].message, "Missing method documentation comment.");
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn accepts_attached_comments_and_initialize_but_not_annotation_directives_or_nodoc() {
        for source in [
            "def initialize; end",
            "def self.initialize; end",
            "# Documentation\ndef foo; end",
            "# Documentation\ndef self.foo; end",
            "# Documentation\n(def foo; end)",
            "# Documentation\n;def foo; end",
            "\u{feff}\x0b;# Documentation\r\n\x0b;def foo = __LINE__\r\n",
            "# doc\n# FIXME: fix\ndef foo; end",
            "# # rubocop:disable Style/For\ndef foo; end",
            "# TODO\ndef foo; end",
            "# typed: strict\ndef foo; end",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
        for comment in [
            "# TODO: fix",
            "# frozen_string_literal: true",
            "# encoding: utf-8",
            "# rubocop:disable Style/For",
            "# rustocop:disable Style/For",
            ";# warn_indent: true",
            ";# coding: utf-8",
            ";#! /usr/bin/ruby",
            "work; # doc",
            "# prose # rubocop:pop\u{200c}",
        ] {
            assert_offense(&format!("{comment}\ndef foo; end"), 2, 1, 2);
        }
        for source in [
            "def foo; end #:nodoc:",
            "def foo; end # docs",
            "# docs\n\ndef foo; end",
            "\"# docs\"\ndef foo; end",
            "=begin\ndocs\n=end\ndef foo; end",
            "class C #:nodoc: all\n def foo; end\nend",
        ] {
            let line = source
                .lines()
                .position(|line| line.trim_start().starts_with("def "))
                .unwrap() as u32
                + 1;
            let col = if source.starts_with("class") { 2 } else { 1 };
            assert_offense(source, line, col, line);
        }
    }
    #[test]
    fn visibility_uses_inline_modifiers_and_only_immediate_siblings() {
        for source in [
            "private\ndef foo; end",
            "protected\ndef foo; end",
            "private def foo; end",
            "protected def foo; end",
            "private_class_method def self.foo; end",
            "private def object.foo; end",
            "private\ndef object.foo; end",
            "class C; def foo; end; def bar; end; private :foo, :bar; end",
            "class C; def foo; end; protected :foo; end",
            "class C; private; ruby2_keywords def foo; end; end",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
        for source in [
            "public def foo; end",
            "class C; public def foo; end; private :foo; end",
            "def foo; end; private_class_method :foo",
            "def self.foo; end; private :foo",
            "private; public; def foo; end",
            "private :other; def foo; end",
            "self.private; def foo; end",
            "private; class C; def foo; end; end",
            "class C; private; end; class C; def foo; end; end",
            "class C; private; class << self; def foo; end; end; end",
            "private; Class.new { def foo; end }",
            "private; if condition; def foo; work; end; end",
            "def foo; end; private :foo; public :foo",
        ] {
            assert_eq!(
                offenses(source).len(),
                1,
                "{source}: {:?}",
                offenses(source)
            );
        }
        assert_offense(
            "class Foo\n private\n def baz; end\n public\n def foo; end\nend",
            5,
            2,
            5,
        );
    }
    #[test]
    fn modifier_wrappers_are_checked_and_documented_at_the_call() {
        for modifier in ["module_function", "ruby2_keywords"] {
            assert_offense(
                &format!("module M\n  {modifier} def foo\n  end\nend"),
                2,
                3,
                3,
            );
            assert_offense(
                &format!("module M\n  {modifier} :other, def foo; end\nend"),
                2,
                3,
                2,
            );
            assert!(
                offenses(&format!(
                    "module M\n  # Documentation\n  {modifier} def foo; end\nend"
                ))
                .is_empty()
            );
            assert!(offenses(&format!("{modifier} def initialize; end")).is_empty());
        }
        assert_offense(
            "module M\n def foo; end\n module_function :foo\nend",
            2,
            2,
            2,
        );
        assert_offense("object.module_function def foo; end", 1, 24, 1);
        assert_offense("private :other\nruby2_keywords def foo; end", 2, 1, 2);
    }
    #[test]
    fn named_visibility_matches_symbols_last_declaration_and_not_other_scopes() {
        for source in [
            "def foo; end; private :other, :foo",
            "def foo; end; private :foo; public :foo; protected :foo",
            "public; def foo; end; private :foo",
            "[private, def foo; end]",
            "consume(private, def foo; end)",
            "if private; def foo; end; end",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
        for source in [
            "def foo; end; private 'foo'",
            "def foo; end; self.private :foo",
            "def foo; end; private :foo; public :foo",
            "def foo; end; nested { private :foo }",
            "Class.new { private }; def foo; end",
            "private do; work; end; def foo; end",
            "private(def foo; end, :other)",
            "# doc\nClass.new { def foo; end }",
            "# doc\nmodule M; def foo; end; end",
            "define_method(:foo) { def nested; end }",
            "def method_missing(name, *args); end",
        ] {
            assert_eq!(
                offenses(source).len(),
                1,
                "{source}: {:?}",
                offenses(source)
            );
        }
    }

    #[test]
    fn nested_reopened_singleton_and_dynamic_scopes_do_not_share_visibility_or_docs() {
        for source in [
            "# class docs\nclass C; private; class D; def foo; end; end; end",
            "module M; private; end; module M; def foo; end; end",
            "class C; private; class << self; def foo; end; end; end",
            "Class.new { private; Class.new { def foo; end } }",
            "Module.new { private; Module.new { def foo; end } }",
            "class C; private; def self.foo; end; end; class C; def self.bar; end; end",
        ] {
            assert_eq!(
                offenses(source).len(),
                1,
                "{source}: {:?}",
                offenses(source)
            );
        }
        for source in [
            "define_method(:foo) { work }",
            "define_singleton_method(:foo) { work }",
            "alias foo bar",
            "alias_method :foo, :bar",
            "Class.new { private; def foo; end }",
            "class << object; protected; def foo; end; end",
            "# documentation\ndef outer\n private\n def inner; end\nend",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
    }

    #[test]
    fn cached_visibility_is_linear_for_many_siblings_and_reused_in_correction_mode() {
        use crate::cops::run_cops_with_corrections;
        let source = format!(
            "class C\n{}{}end\n",
            "def foo; end\n".repeat(4000),
            "private :foo\n".repeat(4000)
        );
        let file = SourceFile::new("lib/a.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        file.record_comments(&parsed);
        for corrections in [false, true] {
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(
                    &[&DocumentationMethod],
                    &parsed,
                    &file,
                    &mut found,
                    corrections
                )
                .is_empty()
            );
            assert!(found.is_empty());
            let index = file.method_visibility().lock().unwrap();
            assert_eq!(index.containers.len(), 1);
            assert_eq!(index.sibling_visits, 8000);
        }
    }

    #[test]
    fn long_comment_groups_and_ruby_boundaries_use_the_shared_cached_association() {
        for comments in [
            ";#! /usr/bin/ruby\n;# warn_indent: true\n;# coding: utf-8\n;# Documentation.\n",
            ";# warn_indent: true\n;# warn_indent: true\n",
            ";# Documentation.\n;# prose # rubocop:popé\n",
            ";# # rustocop:pop\u{200c}\n",
        ] {
            assert!(offenses(&format!("{comments};def foo; end\n")).is_empty());
        }
        assert!(
            offenses(&format!(
                "# Documentation.\n{};def foo; end\n",
                ";# TODO: fix\n".repeat(2000)
            ))
            .is_empty()
        );
        assert_offense(
            "text = <<~H\n# doc\nH\ndef foo; end\n__END__\ndef data; end",
            4,
            1,
            4,
        );
        assert_offense("# doc\n;work; def foo; end", 2, 8, 2);
    }

    #[test]
    fn modifier_calls_report_once_and_singleton_else_ensure_bodies_keep_parser_siblings() {
        for modifier in ["module_function", "ruby2_keywords"] {
            assert_offense(&format!("{modifier}(def foo; end, def bar; end)"), 1, 1, 1);
            assert_offense(
                &format!("{modifier}(def initialize; end, def foo; end)"),
                1,
                1,
                1,
            );
            assert!(
                offenses(&format!("# docs\n{modifier}(def foo; end, def bar; end)")).is_empty()
            );
        }
        for source in [
            "if true; private; else; def foo; end; end",
            "begin; private; ensure; def foo; end; end",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
        for source in [
            "if true; work; private; else; def foo; end; end",
            "begin; work; private; ensure; def foo; end; end",
            "begin; private; rescue; work; ensure; def foo; end; end",
        ] {
            assert_eq!(
                offenses(source).len(),
                1,
                "{source}: {:?}",
                offenses(source)
            );
        }
    }

    #[test]
    fn singleton_block_bodies_keep_the_block_send_as_a_sibling() {
        for source in [
            "private { def foo; end }",
            "protected { |x| def foo; end }",
            "private do; def foo; end; end",
            "protected do |x; local| def foo; end; end",
            "private { |x, y = work| def self.foo; end }",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
        for (source, col) in [
            ("public { def foo; end }", 10),
            ("self.private { def foo; end }", 16),
            ("private(:foo) { def foo; end }", 17),
            ("private { work; def foo; end }", 17),
            ("private do |x| work; def foo; end; end", 22),
        ] {
            assert_offense(source, 1, col, 1);
        }
    }

    #[test]
    fn rescue_else_and_ensure_have_independent_logical_containers() {
        for source in [
            "begin; private; rescue; work; else; def foo; end; ensure; work; end",
            "begin; protected; rescue; work; else; def foo; end; ensure; work; end",
            "begin; private; rescue; work; else; def foo; end; ensure; private; end",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
        for (source, col) in [
            (
                "begin; work; private; rescue; work; else; def foo; end; ensure; work; end",
                43,
            ),
            (
                "begin; public; rescue; work; else; def foo; end; ensure; private; end",
                36,
            ),
            (
                "begin; private; rescue; work; else; def foo; end; ensure; def bar; end; end",
                59,
            ),
        ] {
            assert_offense(source, 1, col, 1);
        }
    }

    #[test]
    fn rescued_exception_expressions_are_not_visibility_siblings_of_the_body() {
        for (source, col) in [
            ("begin; work; rescue private; def foo; end; end", 30),
            ("begin; work; rescue protected; def foo; end; end", 32),
            (
                "begin; work; rescue private, protected; def foo; end; end",
                41,
            ),
            (
                "begin; work; rescue private => error; def foo; end; end",
                39,
            ),
            ("begin; work; rescue private; def self.foo; end; end", 30),
            (
                "begin; work; rescue private, def foo; end; def bar; end; end",
                44,
            ),
        ] {
            assert_offense(source, 1, col, 1);
        }
        for source in [
            "begin; work; rescue private; private; def foo; end; end",
            "begin; work; rescue Error; protected; def foo; end; end",
            "begin; work; rescue private, def foo; end; work; end",
        ] {
            assert!(
                offenses(source).is_empty(),
                "{source}: {:?}",
                offenses(source)
            );
        }
    }

    #[test]
    fn exception_array_visibility_is_cached_linearly_and_separate_from_the_rescue_body() {
        use crate::cops::run_cops_with_corrections;
        let source = format!(
            "begin; work; rescue private, {}; def bar; end; end",
            vec!["def foo; end"; 2000].join(", ")
        );
        let file = SourceFile::new("lib/a.rb".into(), source.into_bytes());
        let parsed = crate::parser::parse(&file.bytes);
        assert_eq!(parsed.errors().count(), 0);
        file.record_comments(&parsed);
        for corrections in [false, true] {
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(
                    &[&DocumentationMethod],
                    &parsed,
                    &file,
                    &mut found,
                    corrections
                )
                .is_empty()
            );
            assert_eq!(found.len(), 1);
            let index = file.method_visibility().lock().unwrap();
            assert_eq!(index.containers.len(), 2);
            assert_eq!(index.sibling_visits, 2002);
        }
    }

    #[test]
    fn reports_public_methods_and_singletons_at_definition_start() {
        for (source, line, col, end) in [
            ("def foo\n puts 'bar'\nend", 1, 1, 3),
            ("  def method; end", 1, 3, 1),
            ("class Foo\n  def bar; end\nend", 2, 3, 2),
            ("module Foo\n  def bar; end\nend", 2, 3, 2),
            ("foo = Object.new\ndef foo.bar\n puts 'baz'\nend", 2, 1, 4),
            ("def self.foo = 1", 1, 1, 1),
        ] {
            assert_offense(source, line, col, end);
        }
    }
}
