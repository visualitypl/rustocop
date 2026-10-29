//! `Style/HashLookupMethod`: opt-in, fixed brackets policy; unsafe, no edits.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
use std::collections::HashSet;
pub struct HashLookupMethod {
    fetch_env_enabled: bool,
}
impl Default for HashLookupMethod {
    fn default() -> Self {
        Self {
            fetch_env_enabled: true,
        }
    }
}
impl Cop for HashLookupMethod {
    fn name(&self) -> &'static str {
        "Style/HashLookupMethod"
    }
    fn with_fetch_env_var(&self, enabled: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            fetch_env_enabled: enabled,
        }))
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
        if call.name().as_slice() != b"fetch" {
            return;
        }
        let loc = call.message_loc().unwrap();
        let facts =
            file.hash_lookup_facts(cx.ancestors().next_back().unwrap(), self.fetch_env_enabled);
        if !facts.selectors.contains(&loc.start_offset()) {
            return;
        }
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `Hash#[]` instead of `Hash#fetch`.".into(),
            line,
            col,
            end_line: file.line_of(loc.end_offset().saturating_sub(1)),
            correction: None,
        });
    }
}
fn candidate<'pr>(
    call: &CallNode<'pr>,
    file: &SourceFile,
    fetch_env_enabled: bool,
) -> Option<Node<'pr>> {
    if call.name().as_slice() != b"fetch"
        || call.is_safe_navigation()
        || call.block().is_some_and(|b| b.as_block_node().is_some())
    {
        return None;
    }
    let receiver = call.receiver()?;
    let args = ast::call_arguments(call);
    if args.len() != 1 {
        return None;
    }
    if fetch_env_enabled && ast::is_global_const(&receiver, &["ENV"]) {
        return None;
    }
    let start = receiver.location().end_offset();
    let end = call.call_operator_loc()?.end_offset();
    let comments = file.comments();
    let i = comments.partition_point(|r| r.end <= start);
    if comments.get(i).is_some_and(|r| r.start < end) {
        return None;
    }
    args.into_iter().next()
}
fn key(node: &Node<'_>) -> (usize, usize) {
    (node.location().start_offset(), node.location().end_offset())
}
/// Suppression is structural (not textual containment: heredoc keys can extend
/// beyond the call's range). Each node/key edge is processed once per policy.
#[derive(Default)]
pub(crate) struct Facts {
    selectors: HashSet<usize>,
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile, fetch_env_enabled: bool) -> Self {
        struct Builder<'a> {
            file: &'a SourceFile,
            env: bool,
            facts: Facts,
            ignored: HashSet<(usize, usize)>,
            stack: Vec<bool>,
        }
        impl<'pr> NodeVisitor<'pr> for Builder<'_> {
            fn enter(&mut self, node: Node<'pr>) {
                self.facts.visits += 1;
                let blocked = self.stack.last().copied().unwrap_or(false)
                    || self.ignored.contains(&key(&node));
                self.stack.push(blocked);
                if blocked {
                    return;
                }
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if let Some(arg) = candidate(&call, self.file, self.env) {
                    self.facts
                        .selectors
                        .insert(call.message_loc().unwrap().start_offset());
                    self.ignored.insert(key(&arg));
                }
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        let mut builder = Builder {
            file,
            env: fetch_env_enabled,
            facts: Facts::default(),
            ignored: HashSet::new(),
            stack: Vec::new(),
        };
        walk(&mut builder, root);
        builder.facts
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop hash_lookup_method_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str_with;
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str_with(&[&HashLookupMethod::default()], s)
    }
    #[test]
    fn exact_selectors_and_chained_receivers() {
        for (s, col) in [
            ("hash.fetch(key)", 6),
            ("some_method.hash.fetch(key)", 18),
            ("do_something(\n  hash\n    .fetch(key)\n)", 6),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                (o[0].line, o[0].col),
                (if s.contains('\n') { 3 } else { 1 }, col)
            );
            assert_eq!(o[0].message, "Use `Hash#[]` instead of `Hash#fetch`.");
            assert!(o[0].correction.is_none());
        }
        let o = offenses("result.fetch(:foo).fetch(:bar)");
        assert_eq!(o.iter().map(|o| o.col).collect::<Vec<_>>(), [8, 20]);
    }
    #[test]
    fn pinned_and_argument_block_comment_lookalikes() {
        for s in [
            "hash[key]",
            "hash.fetch(key, default)",
            "hash.fetch(key) { default }",
            "fetch(key)",
            "fetch(key) { default }",
            "hash&.fetch(key)",
            "hash # comment\n .fetch(key)",
            "ENV.fetch(key)",
            "::ENV.fetch(key)",
            "hash.fetch(key, &block)",
            "Rails.cache.fetch(name, options) { block }",
            "hash.fetch()",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for s in [
            "hash.fetch(:symbol)",
            "hash.fetch('string')",
            "hash.fetch(effect())",
            "hash.fetch(a: effect())",
            "hash.fetch({a: effect()})",
            "hash.fetch(*keys)",
            "hash.fetch(&callback)",
            "hash.fetch(&nil)",
            "Other::ENV.fetch(key)",
            "object.fetch(key)",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn ignored_key_subtrees_not_receivers() {
        for (s, cols) in [
            ("a.fetch(b.fetch(:y))", vec![3]),
            ("a.fetch(:x).fetch(:y)", vec![3, 13]),
            ("a.fetch(b.fetch(:y), fallback)", vec![11]),
            ("a&.fetch(b.fetch(:y))", vec![12]),
            ("a.fetch(b.fetch(:y)) { default }", vec![11]),
            ("ENV.fetch(b.fetch(:y))", vec![13]),
        ] {
            assert_eq!(
                offenses(s).iter().map(|o| o.col).collect::<Vec<_>>(),
                cols,
                "{s}"
            );
        }
    }
}
