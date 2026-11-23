//! Opt-in: a final suspiciously named empty positional hash default.
use crate::{
    cops::{
        Cop, NodeContext,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashSet;

pub struct OptionHash;
impl Cop for OptionHash {
    fn name(&self) -> &'static str {
        "Style/OptionHash"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_parameters_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(params) = node.as_parameters_node() else {
            return;
        };
        // Only the *last* RuboCop argument may be the hash; keyword, rest,
        // block and shadow-local arguments all follow positional defaults.
        if params.rest().is_some()
            || !params.posts().is_empty()
            || !params.keywords().is_empty()
            || params.keyword_rest().is_some()
            || params.block().is_some()
            || cx
                .parent()
                .and_then(Node::as_block_parameters_node)
                .is_some_and(|p| !p.locals().is_empty())
        {
            return;
        }
        let Some(last) = params.optionals().iter().last() else {
            return;
        };
        let Some(optional) = last.as_optional_parameter_node() else {
            return;
        };
        if !matches!(
            optional.name().as_slice(),
            b"options" | b"opts" | b"args" | b"params" | b"parameters"
        ) || !optional
            .value()
            .as_hash_node()
            .is_some_and(|h| h.elements().is_empty())
        {
            return;
        }
        let Some(owner) = cx.ancestors().find(|a| {
            matches!(
                a,
                Node::DefNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
            )
        }) else {
            return;
        };
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        if file
            .option_hash_supers(root)
            .owners
            .contains(&owner.location().start_offset())
        {
            return;
        }
        let loc = last.location();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            message: "Prefer keyword arguments to options hashes.".into(),
            correction: None,
        });
    }
}

/// Lazy, single postorder pass over the main AST, not one subtree scan per
/// parameter. Upstream exempts *any* descendant super, even in nested defs.
/// A Prism block's call is its parent, but is a child of the RuboCop block:
/// supers in the receiver/arguments must exempt that block too.
#[derive(Default)]
pub(crate) struct SuperIndex {
    owners: HashSet<usize>,
    stack: Vec<(Option<usize>, Option<usize>, bool)>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl SuperIndex {
    pub(crate) fn of(root: &Node<'_>) -> Self {
        let mut index = Self::default();
        walk(&mut index, root);
        index
    }
}
impl<'pr> NodeVisitor<'pr> for SuperIndex {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        let owner = matches!(
            node,
            Node::DefNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
        )
        .then(|| node.location().start_offset());
        let call_block = node
            .as_call_node()
            .and_then(|c| c.block())
            .or_else(|| node.as_super_node().and_then(|s| s.block()))
            .or_else(|| {
                node.as_forwarding_super_node()
                    .and_then(|s| s.block())
                    .map(|b| b.as_node())
            })
            .filter(|b| b.as_block_node().is_some())
            .map(|b| b.location().start_offset());
        let is_super = matches!(
            node,
            Node::SuperNode { .. } | Node::ForwardingSuperNode { .. }
        );
        self.stack.push((owner, call_block, is_super));
    }
    fn leave(&mut self) {
        let (owner, call_block, has_super) = self.stack.pop().unwrap();
        if has_super {
            self.owners.extend(owner);
            self.owners.extend(call_block);
            if let Some(parent) = self.stack.last_mut() {
                parent.2 = true;
            }
        }
    }
}

pub(crate) static OPTION_HASH_SUPERS: crate::source::Fact<SuperIndex> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn option_hash_supers(&self, root: &ruby_prism::Node<'_>) -> &SuperIndex {
        OPTION_HASH_SUPERS.get(self, || SuperIndex::of(root))
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop option_hash_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Custom `config` and allowlist
    //! examples are adapted to the fixed suspicious names / empty allowlist.
    use super::*;
    use crate::cops::testing::correction_offenses;
    fn offenses(source: &str) -> Vec<Offense> {
        correction_offenses(&OptionHash, source)
    }

    #[test]
    fn pinned_option_hash_and_fixed_names() {
        for name in ["options", "opts", "args", "params", "parameters"] {
            let source = format!("def some_method({name} = {{}})\n  puts some_arg\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 17, 1));
            assert_eq!(
                found[0].message,
                "Prefer keyword arguments to options hashes."
            );
            assert!(found[0].correction.is_none());
        }
        assert_eq!(offenses("def to_json(options = {}); end").len(), 1);
    }
    #[test]
    fn pinned_and_structural_negative_lookalikes() {
        for source in [
            "def steep(flavor, duration, config={}); end",
            "def meditate; puts true; end",
            "def cook(instructions, ingredients = { hot: [], cold: [] }); end",
            "def f(options = { hot: [] }); end",
            "def f(options = ({})); end",
            "def f(options = Hash.new); end",
            "def f(options: {}); end",
            "def f(_options = {}); end",
            "def f(options = {}, last); end",
            "def f(options = {}, other = {}); end",
            "def f(options = {}, *rest); end",
            "def f(options = {}, *); end",
            "def f(options = {}, key:); end",
            "def f(options = {}, key: 1); end",
            "def f(options = {}, **kw); end",
            "def f(options = {}, **nil); end",
            "def f(options = {}, &block); end",
            "def f(options = {}, &); end",
            "proc { |options = {}; local| }",
            "def f(...); g(...); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn pinned_super_exemption_covers_all_descendants() {
        for body in [
            "super",
            "bar; super",
            "5.times do; super; end",
            "super(options)",
            "super()",
            "super(**options)",
            "super(*options, &callback)",
            "-> { super }",
            "def other; super; end",
            "\"#{super}\"",
            "begin; effect; rescue; super; ensure; nil; end",
        ] {
            let source = format!("def allowed(foo, options = {{}})\n  {body}\nend\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for source in [
            "def f(options = {}); :super; end",
            "def f(options = {}); obj.super; end",
            "def f(options = {}); 'super'; end",
            "def f(options = {}); end; def g; super; end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn singleton_endless_blocks_and_lambdas_have_distinct_owners() {
        for (source, col) in [
            ("def self.f(options={}); end", 12),
            ("def obj.f(options={}) = options", 11),
            ("proc { |options={}| options }", 9),
            ("->(options={}) { options }", 4),
            ("define_method(:f) { |options={}| options }", 22),
            ("def f((a,b), options={}); end", 14),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
        }
        for source in [
            "proc { |options={}| super }",
            "->(options={}) { super() }",
            "foo(super) { |options={}| }",
            "super.foo { |options={}| }",
            "super { |options={}| }",
            "super() { |options={}| }",
            "def f(x=super, options={}); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // A super in a sibling scope must not leak into the block.
        assert_eq!(
            offenses("def f; super; proc { |options={}| }; end").len(),
            1
        );
    }
    #[test]
    fn multiline_empty_hash_location_and_no_correction() {
        let found = offenses("def f(\n  options = {\n    # default\n  }\n)\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
        assert!(found[0].correction.is_none());
    }
}
