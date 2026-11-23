//! `Lint/UselessRuby2Keywords` — unnecessary method keyword forwarding marks.

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Node, ParametersNode};
use std::{
    collections::{HashMap, HashSet},
    mem::discriminant,
};

pub struct UselessRuby2Keywords;
impl Cop for UselessRuby2Keywords {
    fn name(&self) -> &'static str {
        "Lint/UselessRuby2Keywords"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.is_safe_navigation() || call.name().as_slice() != b"ruby2_keywords" {
            return;
        }
        let arguments = ast::call_arguments(&call);
        let Some(first) = arguments.first() else {
            return;
        };
        let (name, start, end) = if let Some(def) =
            first.as_def_node().filter(|d| d.receiver().is_none())
        {
            if allowed(def.parameters()) {
                return;
            }
            let Some(selector) = call.message_loc() else {
                return;
            };
            (
                file.match_pattern_source(def.name().as_slice())
                    .into_owned(),
                selector.start_offset(),
                selector.end_offset(),
            )
        } else if let Some(name) = symbol(first, file) {
            let facts = file.ruby2_keywords_facts(cx.ancestors().last().unwrap_or(node));
            // A send with a literal block has that block as its own parent.
            let own_block = call.block().filter(|b| b.as_block_node().is_some());
            let mut child = node;
            let found = own_block
                .iter()
                .chain(cx.ancestors())
                .find_map(|parent| {
                    // Prism nests the block under its send; the reference block's
                    // body and send are siblings. Never search the send from its body.
                    let skip = (child.as_block_node().is_some()
                        && matches!(
                            parent,
                            Node::CallNode { .. }
                                | Node::SuperNode { .. }
                                | Node::ForwardingSuperNode { .. }
                        ))
                        || (child.as_rescue_node().is_some() && parent.as_rescue_node().is_some());
                    let found = (!skip)
                        .then(|| {
                            facts.get(parent, child, &name).or_else(|| {
                                // Ascending from a receiver/argument visits the send
                                // before its synthetic block parent. From the body,
                                // that block was already visited and the send is skipped.
                                literal_block(parent)
                                    .and_then(|block| facts.get(&block, parent, &name))
                            })
                        })
                        .flatten();
                    child = parent;
                    found.map(Some).or_else(|| boundary(parent).then_some(None))
                })
                .flatten();
            if found != Some(false) {
                return;
            }
            (name, call.location().start_offset(), ast::source_end(node))
        } else {
            return;
        };
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        let name: String = name
            .utf8_chunks()
            .map(|c| {
                let mut s = c.valid().to_owned();
                for b in c.invalid() {
                    s.push_str(&format!("\\x{b:02X}"));
                }
                s
            })
            .collect();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("`ruby2_keywords` is unnecessary for method `{name}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: None,
        });
    }
}

fn literal_block<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    match node {
        Node::CallNode { .. } => node.as_call_node()?.block(),
        Node::SuperNode { .. } => node.as_super_node()?.block(),
        Node::ForwardingSuperNode { .. } => {
            return node
                .as_forwarding_super_node()?
                .block()
                .map(|b| b.as_node());
        }
        _ => None,
    }
    .filter(|b| b.as_block_node().is_some())
}
fn boundary(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. }
    )
}
fn allowed(parameters: Option<ParametersNode<'_>>) -> bool {
    parameters.is_some_and(|p| {
        p.rest().is_some()
            && p.keywords().is_empty()
            && !p
                .keyword_rest()
                .is_some_and(|k| k.as_keyword_rest_parameter_node().is_some())
    })
}
fn block_allowed(block: &ruby_prism::BlockNode<'_>) -> bool {
    allowed(
        block
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
            .and_then(|p| p.parameters()),
    )
}
fn symbol(node: &Node<'_>, file: &SourceFile) -> Option<Vec<u8>> {
    let s = node.as_symbol_node()?;
    let raw = s.value_loc();
    // Empty quoted symbols project to dsym, not sym, in the reference AST.
    if s.unescaped().is_empty()
        && raw
            .as_ref()
            .is_none_or(|l| l.as_slice().is_empty() || l.as_slice().contains(&b'\n'))
    {
        return None;
    }
    let single = s
        .opening_loc()
        .is_some_and(|l| l.as_slice() == b":'" || l.as_slice().starts_with(b"%s"));
    let parts = ast::require_literal::parts(
        raw.as_ref().map_or(s.unescaped(), |l| l.as_slice()),
        s.unescaped(),
        single,
        file,
    );
    Some(parts.into_iter().flatten().collect())
}

// Last component distinguishes reference-only array/ensure layers from
// real Prism nodes at the same location. No parse pointers escape the walk.
type Key = (usize, usize, String, u8);
fn key(node: &Node<'_>) -> Key {
    (
        node.location().start_offset(),
        node.location().end_offset(),
        format!("{:?}", discriminant(node)),
        0,
    )
}
fn transparent(node: &Node<'_>) -> bool {
    node.as_arguments_node().is_some()
        || node.as_program_node().is_some()
        || node.as_else_node().is_some()
        || node.as_ensure_node().is_some()
        || node
            .as_statements_node()
            .is_some_and(|s| s.body().len() == 1)
        || node.as_begin_node().is_some_and(|b| {
            b.begin_keyword_loc().is_none()
                && b.rescue_clause().is_none()
                && b.ensure_clause().is_none()
        })
}
/// One lazy typed-child-complete main-tree walk indexes only direct method
/// children. Queries are bounded by NodeContext depth, with O(1) name lookup;
/// repeated calls never scan a root or an ancestor subtree. First child wins,
/// regardless of whether it physically precedes the ruby2_keywords call.
#[derive(Default)]
pub(crate) struct Facts {
    children: HashMap<Key, HashMap<Vec<u8>, bool>>,
    exception_owners: HashMap<Key, Key>,
    ensure_owners: HashMap<Key, Key>,
    modifier_resbody_owners: HashMap<Key, Key>,
    singleton_receiver_wrappers: HashSet<Key>,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        struct Builder<'a> {
            facts: Facts,
            parents: Vec<Option<Key>>,
            file: &'a SourceFile,
        }
        impl<'pr> NodeVisitor<'pr> for Builder<'_> {
            fn enter(&mut self, node: Node<'pr>) {
                if let Some(rescue) = node.as_rescue_modifier_node() {
                    let mut owner = key(&node);
                    owner.3 = 3; // rescue(body, resbody(nil, nil, fallback), nil)
                    self.facts
                        .modifier_resbody_owners
                        .insert(key(&rescue.rescue_expression()), owner);
                }
                if let Some(receiver) = node.as_def_node().and_then(|d| d.receiver())
                    && receiver.as_parentheses_node().is_some()
                {
                    // Only the required outer receiver parentheses disappear.
                    // Nested and ordinary parentheses remain reference begin nodes.
                    self.facts
                        .singleton_receiver_wrappers
                        .insert(key(&receiver));
                }
                if let Some(rescue) = node.as_rescue_node() {
                    let mut owner = key(&node);
                    owner.3 = 1;
                    for exception in rescue.exceptions().iter() {
                        self.facts
                            .exception_owners
                            .insert(key(&exception), owner.clone());
                    }
                }
                if let Some(begin) = node.as_begin_node()
                    && let Some(ensure) = begin.ensure_clause()
                {
                    let mut owner = key(&node);
                    if begin.rescue_clause().is_some() {
                        owner.3 = 2;
                    }
                    self.facts
                        .ensure_owners
                        .insert(key(&ensure.as_node()), owner);
                }
                let definition =
                    if let Some(def) = node.as_def_node().filter(|d| d.receiver().is_none()) {
                        Some((
                            self.file
                                .match_pattern_source(def.name().as_slice())
                                .into_owned(),
                            allowed(def.parameters()),
                        ))
                    } else if let Some(call) = node.as_call_node().filter(|c| {
                        !c.is_safe_navigation() && c.name().as_slice() == b"define_method"
                    }) {
                        call.block().and_then(|b| b.as_block_node()).and_then(|b| {
                            let args = ast::call_arguments(&call);
                            let [arg] = args.as_slice() else { return None };
                            Some((symbol(arg, self.file)?, block_allowed(&b)))
                        })
                    } else {
                        None
                    };
                if let Some((name, allowed)) = definition
                    && let Some(parent) = self
                        .facts
                        .exception_owners
                        .get(&key(&node))
                        .or_else(|| self.facts.modifier_resbody_owners.get(&key(&node)))
                        .or_else(|| self.parents.last().and_then(Option::as_ref))
                {
                    self.facts
                        .children
                        .entry(parent.clone())
                        .or_default()
                        .entry(name)
                        .or_insert(allowed);
                }
                let scope = self
                    .facts
                    .ensure_owners
                    .get(&key(&node))
                    .cloned()
                    .or_else(|| {
                        if transparent(&node)
                            || self.facts.singleton_receiver_wrappers.contains(&key(&node))
                        {
                            self.facts
                                .modifier_resbody_owners
                                .get(&key(&node))
                                .cloned()
                                .or_else(|| self.parents.last().cloned().flatten())
                        } else {
                            Some(key(&node))
                        }
                    });
                self.parents.push(scope);
            }
            fn leave(&mut self) {
                self.parents.pop();
            }
        }
        let mut builder = Builder {
            facts: Self::default(),
            parents: Vec::new(),
            file,
        };
        walk::walk(&mut builder, root);
        builder.facts
    }
    fn get(&self, node: &Node<'_>, child: &Node<'_>, name: &[u8]) -> Option<bool> {
        let lookup = |key: &Key| self.children.get(key)?.get(name).copied();
        if node.as_rescue_modifier_node().is_some()
            && let Some(owner) = self.modifier_resbody_owners.get(&key(child))
            && let Some(found) = lookup(owner)
        {
            return Some(found);
        }
        if node.as_rescue_node().is_some()
            && let Some(owner) = self.exception_owners.get(&key(child))
            && let Some(found) = lookup(owner)
        {
            return Some(found);
        }
        if let Some(owner) = self.ensure_owners.get(&key(node)) {
            return lookup(owner);
        }
        let key = key(node);
        if node
            .as_begin_node()
            .is_some_and(|b| b.rescue_clause().is_some() && b.ensure_clause().is_some())
        {
            // ensure(rescue(...), ensure_body): a branch inside the rescue sees
            // rescue children first; the ensure body must not see through it.
            let mut ensure = key.clone();
            ensure.3 = 2;
            if child.as_ensure_node().is_some() {
                return lookup(&ensure);
            }
            return lookup(&key).or_else(|| lookup(&ensure));
        }
        lookup(&key)
    }
}

pub(crate) static RUBY2_KEYWORDS_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn ruby2_keywords_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        RUBY2_KEYWORDS_FACTS.get(self, || Facts::build(root, self))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_ruby2_keywords_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessRuby2Keywords")
            .collect()
    }
    #[test]
    fn pinned_inline_signatures() {
        for args in [
            "",
            "arg",
            "**kwargs",
            "i:,j:",
            "*args,i:,j:",
            "*args,i: 1",
            "*args,**kwargs",
        ] {
            let source = format!("ruby2_keywords def foo({args})\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!(
                found[0].message,
                "`ruby2_keywords` is unnecessary for method `foo`."
            );
            assert!(found[0].correction.is_none());
        }
        for args in [
            "*args",
            "a,b,*rest",
            "a=5,*rest",
            "*rest,&block",
            "*",
            "*,**nil",
        ] {
            let source = format!("ruby2_keywords def foo({args}); end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_symbol_definitions_and_blocks() {
        for definition in [
            "def foo(**kw); end",
            "define_method(:foo) { |**kw| }",
            "define_method(:foo) { |x; y| }",
            "define_method(:foo) { _1 }",
            "define_method(:foo) { it }",
        ] {
            let source = format!("{definition}\nruby2_keywords :foo\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 2));
            assert_eq!(
                found[0].message,
                "`ruby2_keywords` is unnecessary for method `foo`."
            );
        }
        for definition in [
            "def foo(*args); end",
            "define_method(:foo) { |*args| }",
            "define_method(:foo) { |*args; y| }",
        ] {
            assert!(offenses(&format!("{definition}\nruby2_keywords :foo")).is_empty());
        }
    }
    #[test]
    fn pinned_lexical_boundaries_and_lookalikes() {
        for source in [
            "ruby2_keywords :foo",
            "work { ruby2_keywords :foo }",
            "class C; class D; def foo(**kw); end; end; ruby2_keywords :foo; end",
            "class Outer; def foo(**kw); end; class Inner; ruby2_keywords :foo; end; end",
            "block = proc { |_,*args| klass.new(*args) }; block.ruby2_keywords if block.respond_to?(:ruby2_keywords)",
            "ruby2_keywords def self.foo; end",
            "def foo;end; ruby2_keywords 'foo'",
            "def foo;end; ruby2_keywords :\"#{name}\"",
            "object&.ruby2_keywords def foo; end",
            "define_method(:foo, extra) {}; ruby2_keywords :foo",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn synthetic_rescue_arrays_ensure_layers_and_else_edges() {
        for (source, n) in [
            (
                "begin; work; rescue def foo;end; ruby2_keywords :foo;end",
                0,
            ),
            (
                "begin; work; rescue def foo;end, ruby2_keywords(:foo);nil;end",
                1,
            ),
            (
                "begin; work;rescue A;def foo;end;rescue B;ruby2_keywords :foo;end",
                0,
            ),
            (
                "begin; def foo;end;rescue;nil;ensure;ruby2_keywords :foo;end",
                0,
            ),
            (
                "begin; ruby2_keywords :foo;rescue;nil;ensure;def foo;end;end",
                1,
            ),
            ("begin; ruby2_keywords :foo;ensure;def foo;end;end", 1),
            (
                "begin; ruby2_keywords :foo;rescue;nil;else;def foo;end;end",
                1,
            ),
            ("if flag; ruby2_keywords :foo;else;def foo;end;end", 1),
        ] {
            assert_eq!(offenses(source).len(), n, "{source}");
        }
    }

    #[test]
    fn enclosing_call_and_super_blocks_participate_in_nearest_lookup() {
        for (source, col) in [
            ("wrap(ruby2_keywords(:foo)) { def foo; end }", Some(6)),
            (
                "def foo;end; wrap(ruby2_keywords(:foo)) { def foo(*args);end }",
                None,
            ),
            (
                "def run; super(ruby2_keywords(:foo)) { def foo;end };end",
                Some(16),
            ),
            (
                "wrap(def foo;end, ruby2_keywords(:foo)) { def foo(*args);end }",
                Some(19),
            ),
            ("wrap(def foo;end) { ruby2_keywords :foo }", None),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), usize::from(col.is_some()), "{source}");
            if let Some(col) = col {
                assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
                assert_eq!(
                    found[0].message,
                    "`ruby2_keywords` is unnecessary for method `foo`."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn rescue_modifier_fallback_is_a_synthetic_resbody() {
        for (source, col) in [
            ("ruby2_keywords :foo rescue def foo;end", None),
            ("def foo;end rescue ruby2_keywords :foo", Some(20)),
            ("(def foo;end; ruby2_keywords :foo) rescue nil", Some(15)),
            ("nil rescue (def foo;end; ruby2_keywords :foo)", Some(26)),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), usize::from(col.is_some()), "{source}");
            if let Some(col) = col {
                assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            }
        }
    }

    #[test]
    fn only_outer_singleton_receiver_parentheses_are_transparent() {
        for (source, col) in [
            ("def (def foo;end).bar; ruby2_keywords :foo;end", Some(24)),
            ("def (ruby2_keywords :foo).bar; def foo;end;end", Some(6)),
            ("def ((def foo;end)).bar; ruby2_keywords :foo;end", None),
            ("(def foo;end); ruby2_keywords :foo", None),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), usize::from(col.is_some()), "{source}");
            if let Some(col) = col {
                assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            }
        }
    }

    #[test]
    fn nearest_direct_child_and_first_match_not_dispatch_inference() {
        for (source, n) in [
            ("ruby2_keywords :foo; def foo; end", 1),
            ("def foo; end; def foo(*args); end; ruby2_keywords :foo", 1),
            ("def foo(*args); end; def foo; end; ruby2_keywords :foo", 0),
            (
                "def foo; end; work { def foo(*args);end; ruby2_keywords :foo }",
                0,
            ),
            ("def foo; end; work { ruby2_keywords :foo }", 1),
            (
                "def foo; end; ruby2_keywords(:foo) if respond_to?(:ruby2_keywords, true)",
                1,
            ),
            ("ruby2_keywords(:foo) { def foo;end }", 1),
            (
                "def foo(*args); end; ruby2_keywords(:foo) { def foo;end }",
                1,
            ),
            ("def foo;end; class << self; ruby2_keywords :foo;end", 0),
            ("(def foo;end); ruby2_keywords :foo", 0),
            ("def other; def foo;end; ruby2_keywords :foo; end", 1),
            ("obj.define_method(:foo) { }; obj.ruby2_keywords :foo", 1),
            ("ruby2_keywords(:foo, :bar); define_method(:foo) {}", 1),
        ] {
            assert_eq!(offenses(source).len(), n, "{source}");
        }
    }
}
