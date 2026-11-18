//! `Style/HashFetchChain`: Ruby >=2.3; unsafe suggestions, never corrections.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
pub struct HashFetchChain;
impl Cop for HashFetchChain {
    fn name(&self) -> &'static str {
        "Style/HashFetchChain"
    }
    fn unsafe_fixes(&self) -> bool {
        true
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
        if !ends_in_nil(&call) {
            return;
        }
        let facts = file.hash_fetch_facts(cx.ancestors().next_back().unwrap());
        let Some((range, message)) = facts.offenses.get(&selector(&call)) else {
            return;
        };
        let (line, col) = file.line_col(range.start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: message.clone(),
            line,
            col,
            end_line: file.line_of(range.end.saturating_sub(1)),
            correction: message
                .strip_prefix("Use `")
                .and_then(|m| m.strip_suffix("` instead."))
                .filter(|_| cx.corrections_requested())
                .map(|replacement| Correction {
                    range: range.clone(),
                    replacement: replacement.to_owned(),
                }),
        });
    }
}
fn selector(call: &CallNode<'_>) -> usize {
    call.message_loc().unwrap().start_offset()
}
fn ends_in_nil(call: &CallNode<'_>) -> bool {
    call.name().as_slice() == b"fetch"
        && ast::call_arguments(call)
            .last()
            .is_some_and(|n| n.as_nil_node().is_some())
}
fn diggable<'pr>(call: &CallNode<'pr>) -> Option<Node<'pr>> {
    if call.name().as_slice() != b"fetch" {
        return None;
    }
    let mut args = ast::call_arguments(call).into_iter();
    let key = args.next()?;
    let default = args.next()?;
    if args.next().is_some() {
        return None;
    }
    let empty_hash = default
        .as_hash_node()
        .is_some_and(|h| h.elements().is_empty());
    let hash_new = default.as_call_node().is_some_and(|c| {
        !c.is_safe_navigation()
            && c.name().as_slice() == b"new"
            && c.block().is_none()
            && ast::call_arguments(&c).is_empty()
            && c.receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Hash"]))
    });
    (default.as_nil_node().is_some() || empty_hash || hash_new).then_some(key)
}
/// One lazy main-AST walk. Inspected receiver-chain nodes are marked even for
/// singleton runs, just like upstream ignore_node. Each link is scanned once.
#[derive(Default)]
pub(crate) struct Facts {
    offenses: HashMap<usize, (Range<usize>, String)>,
    pub(crate) links: usize,
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Builder {
            facts: Facts,
            ignored: HashSet<usize>,
        }
        impl<'pr> NodeVisitor<'pr> for Builder {
            fn enter(&mut self, node: Node<'pr>) {
                self.facts.visits += 1;
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if !ends_in_nil(&call) || self.ignored.contains(&selector(&call)) {
                    return;
                }
                let terminal = selector(&call);
                let end = ast::source_end(&node);
                let mut cursor = Some(node);
                let mut keys = Vec::new();
                let mut start = terminal;
                while let Some(n) = cursor {
                    let Some(c) = n.as_call_node() else { break };
                    // A literal block is a parent of its send in RuboCop.
                    // Inspect its own send, but never traverse a block-valued
                    // receiver as if that receiver were the send beneath it.
                    if !keys.is_empty() && c.block().is_some_and(|b| b.as_block_node().is_some()) {
                        break;
                    }
                    let Some(key) = diggable(&c) else { break };
                    self.facts.links += 1;
                    start = selector(&c);
                    self.ignored.insert(start);
                    keys.push(String::from_utf8_lossy(key.location().as_slice()).into_owned());
                    cursor = c.receiver();
                }
                if keys.len() > 1 {
                    keys.reverse();
                    self.facts.offenses.insert(
                        terminal,
                        (
                            start..end,
                            format!("Use `dig({})` instead.", keys.join(", ")),
                        ),
                    );
                }
            }
        }
        let mut builder = Builder::default();
        walk(&mut builder, root);
        builder.facts
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop hash_fetch_chain_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/HashFetchChain")
            .collect()
    }
    #[test]
    fn pinned_chains_and_exact_messages() {
        for default in ["nil", "{}", "Hash.new", "::Hash.new"] {
            for (keys, expected) in [
                (["'foo'", "'bar'"], "'foo', 'bar'"),
                ([":foo", ":bar"], ":foo, :bar"),
                (["foo", "bar"], "foo, bar"),
            ] {
                for (prefix, col) in [("hash.", 6), ("hash&.", 7), ("x.hash.", 8)] {
                    let s = format!(
                        "{prefix}fetch({}, {default})&.fetch({}, nil)",
                        keys[0], keys[1]
                    );
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, col, 1));
                    assert_eq!(o[0].message, format!("Use `dig({expected})` instead."));
                    assert!(o[0].correction.is_none());
                }
            }
            let s =
                format!("hash.fetch('foo', {default})&.fetch(:bar, {default})&.fetch(baz, nil)");
            assert_eq!(
                offenses(&s)[0].message,
                "Use `dig('foo', :bar, baz)` instead."
            );
            assert!(offenses(&format!("hash.fetch(:foo, {default})")).is_empty());
        }
    }
    #[test]
    fn literal_blocks_wrap_sends_and_break_only_receiver_edges() {
        for block in [" { effect }", " do; effect; end"] {
            for suffix in ["", ".fetch(:c,nil)"] {
                let s = format!("h.fetch(:a,nil).fetch(:b,nil){block}{suffix}");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col), (1, 3));
                assert_eq!(o[0].message, "Use `dig(:a, :b)` instead.");
            }
            assert!(offenses(&format!("h.fetch(:a,nil){block}.fetch(:b,nil)")).is_empty());
        }
    }

    #[test]
    fn unrelated_implicit_call_selectors_do_not_break_lazy_facts() {
        let s = "callback.()\nh.fetch(:a,{}).fetch(:b,nil)\n";
        let o = offenses(s);
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col), (2, 3));
    }

    #[test]
    fn multiline_positions() {
        let o = offenses("hash\n  .fetch('foo', nil)\n  .fetch('bar', nil)\n");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 4, 3));
    }
    #[test]
    fn pinned_and_argument_receiver_lookalikes() {
        for s in [
            "hash.fetch('foo').fetch('bar')",
            "hash.fetch('foo') { :default }.fetch('bar') { :default }",
            "hash.fetch",
            "hash.fetch('foo', bar).fetch('baz', quux)",
            "hash.fetch('foo', {}).fetch('bar', {})",
            "hash.fetch(:a, {x: 1}).fetch(:b,nil)",
            "hash.fetch(:a, Other::Hash.new).fetch(:b,nil)",
            "hash.fetch(:a, Hash.new(1)).fetch(:b,nil)",
            "hash.fetch(:a,nil,&b).fetch(:b,nil)",
            "hash.fetch(:a,nil) { effect }.fetch(:b,nil)",
            "hash.fetch(a: nil).fetch(:b,nil)",
            "hash.fetch({a: nil}).fetch(:b,nil)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn partial_runs_ignored_nodes_and_nested_keys() {
        for (s, n, msg) in [
            (
                "h.fetch(:a,nil).fetch(:b,nil).fetch(:c,{})",
                1,
                "Use `dig(:a, :b)` instead.",
            ),
            (
                "h.fetch(:a,nil).fetch(:b,other).fetch(:c,{}).fetch(:d,nil)",
                1,
                "Use `dig(:c, :d)` instead.",
            ),
            (
                "h.fetch(:a,nil).fetch(:b,nil).fetch(:c,nil)",
                1,
                "Use `dig(:a, :b, :c)` instead.",
            ),
            (
                "h.fetch(x.fetch(:a,nil).fetch(:b,nil),{}).fetch(:c,nil)",
                2,
                "Use `dig(x.fetch(:a,nil).fetch(:b,nil), :c)` instead.",
            ),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), n, "{s}");
            assert_eq!(o[0].message, msg, "{s}");
        }
    }
}
