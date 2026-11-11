//! `Lint/UselessRescue` — last rescue branches that only reraise.

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{self, NodeVisitor},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;

pub struct UselessRescue;
impl Cop for UselessRescue {
    fn name(&self) -> &'static str {
        "Lint/UselessRescue"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (body, variable, start, end) = if let Some(rescue) = node.as_rescue_node() {
            if rescue.subsequent().is_some() {
                return;
            }
            let Some(body) = rescue
                .statements()
                .and_then(|s| (s.body().len() == 1).then(|| s.body().iter().next().unwrap()))
            else {
                return;
            };
            (
                body,
                rescue.reference(),
                rescue.keyword_loc().start_offset(),
                ast::source_end(node),
            )
        } else if let Some(rescue) = node.as_rescue_modifier_node() {
            (
                rescue.rescue_expression(),
                None,
                rescue.keyword_loc().start_offset(),
                ast::source_end(&rescue.rescue_expression()),
            )
        } else {
            return;
        };
        let Some(call) = body.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"raise"
            || call.receiver().is_some()
            || call.block().is_some_and(|b| b.as_block_node().is_some())
        {
            return;
        }
        let arguments = ast::call_arguments(&call);
        if !arguments.is_empty() {
            let [argument] = arguments.as_slice() else {
                return;
            };
            let name = source(argument.location().as_slice(), file);
            if name != b"$!"
                && name != b"$ERROR_INFO"
                && !variable
                    .as_ref()
                    .is_some_and(|v| source(v.location().as_slice(), file) == name)
            {
                return;
            }
        }
        // Ruby's ensure node encloses both protected body and branch. Prism
        // represents it as a child of Begin: ask the ancestor stack for the
        // nearest such owner, including when this rescue is in its branch.
        if let Some(variable) = variable {
            let ensure = cx
                .ancestors()
                .find_map(|n| n.as_begin_node().and_then(|b| b.ensure_clause()));
            if let Some(ensure) = ensure {
                let root = cx.ancestors().last().unwrap_or(node);
                if file.useless_rescue_facts(root).uses(
                    &ensure.as_node(),
                    &source(variable.location().as_slice(), file),
                ) {
                    return;
                }
            }
        }
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Useless `rescue` detected.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: None,
        });
    }
}

fn local_read(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableReadNode { .. } | Node::ItLocalVariableReadNode { .. }
    )
}

fn source(bytes: &[u8], file: &SourceFile) -> Vec<u8> {
    let projected = file.match_pattern_source(bytes);
    let mut bytes = projected.iter().copied().peekable();
    let mut result = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != b'\r' || bytes.peek() != Some(&b'\n') {
            result.push(byte);
        }
    }
    result
}

/// One lazy complete main-AST traversal. Ordinals, not physical containment,
/// keep deferred heredoc children inside their real ensure branch. Queries
/// use name-indexed sorted reads, never rescan roots or nested ensure trees.
#[derive(Default)]
pub(crate) struct Facts {
    branches: HashMap<usize, (usize, usize, bool)>,
    reads: HashMap<Vec<u8>, Vec<usize>>,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        struct Builder<'a> {
            facts: Facts,
            ordinal: usize,
            open: Vec<Option<usize>>,
            file: &'a SourceFile,
        }
        impl<'pr> NodeVisitor<'pr> for Builder<'_> {
            fn enter(&mut self, node: Node<'pr>) {
                self.ordinal += 1;
                let key = node.as_ensure_node().map(|ensure| {
                    let key = node.location().start_offset();
                    // each_descendant excludes a sole lvar branch root, but
                    // `(e)` is a begin root and multi-statements have a begin.
                    let excluded = ensure.statements().is_some_and(|s| {
                        s.body().len() == 1 && local_read(&s.body().iter().next().unwrap())
                    });
                    self.facts.branches.insert(key, (self.ordinal, 0, excluded));
                    key
                });
                if local_read(&node) {
                    let mut name = source(node.location().as_slice(), self.file);
                    // Prism includes the label colon in implicit reads (`e:`).
                    // The reference lvar source is only the identifier, as in `e: e`.
                    if name.last() == Some(&b':') {
                        name.pop();
                    }
                    self.facts.reads.entry(name).or_default().push(self.ordinal);
                }
                self.open.push(key);
            }
            fn leave(&mut self) {
                if let Some(key) = self.open.pop().unwrap() {
                    self.facts.branches.get_mut(&key).unwrap().1 = self.ordinal;
                }
            }
        }
        let mut builder = Builder {
            facts: Self::default(),
            ordinal: 0,
            open: Vec::new(),
            file,
        };
        walk::walk(&mut builder, root);
        builder.facts
    }
    fn uses(&self, ensure: &Node<'_>, name: &[u8]) -> bool {
        let Some(&(start, end, excluded)) = self.branches.get(&ensure.location().start_offset())
        else {
            return false;
        };
        if excluded {
            return false;
        }
        let Some(reads) = self.reads.get(name) else {
            return false;
        };
        reads
            .get(reads.partition_point(|&n| n < start))
            .is_some_and(|&n| n <= end)
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_rescue_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessRescue")
            .collect()
    }
    #[test]
    fn pinned_reraise_forms_and_locations() {
        for (header, body) in [
            ("rescue", "raise"),
            ("rescue => e", "raise e"),
            ("rescue", "raise $!"),
            ("rescue", "raise $ERROR_INFO"),
        ] {
            let source = format!("def foo\n  do_something\n{header}\n  {body}\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 4));
            assert_eq!(found[0].message, "Useless `rescue` detected.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_lookalikes_and_exact_source_arguments() {
        for source in [
            "def foo; work; rescue; cleanup; raise; end",
            "def foo; work; rescue => e; raise x; end",
            "def foo; rescue => e; ensure; end",
            "def foo; work; rescue => e; raise; ensure; use(e); end",
            "def foo; work; rescue ArgumentError; raise; rescue; end",
            "def foo; work; rescue; Thread.current.raise; end",
            "work rescue nil",
            "work rescue Kernel.raise",
            "work rescue fail",
            "work rescue (raise)",
            "work rescue raise($!, 'message')",
            "begin; work; rescue => e; raise (e); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn implicit_it_is_a_descendant_lvar_even_in_another_binding_scope() {
        for branch in ["proc { it }", "def other; proc { it }; end"] {
            let source = format!("begin; work; rescue => it; raise; ensure; {branch}; end");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn shorthand_keyword_and_hash_reads_in_direct_and_outer_ensure() {
        for name in ["e", "it", "é", "e\u{301}"] {
            for branch in [
                format!("use({name}:)"),
                format!("{{ {name}: }}"),
                format!("use({name}: {name})"),
                format!("{{ {name}: {name} }}"),
            ] {
                for source in [
                    format!("begin; work; rescue => {name}; raise; ensure; {branch}; end"),
                    format!(
                        "begin; begin; work; rescue => {name}; raise; end; ensure; {branch}; end"
                    ),
                    format!(
                        "begin; work; ensure; begin; work; rescue => {name}; raise; end; {branch}; end"
                    ),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
        }
        for branch in ["use(other:)", "{ other: }", "use(e: other)", "{ e: other }"] {
            let source =
                format!("other = nil; begin; work; rescue => e; raise; ensure; {branch}; end");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }
    #[test]
    fn only_last_branch_but_modifier_raise_is_a_real_branch() {
        for source in [
            "def foo; work; rescue ArgumentError; nil; rescue; raise; end",
            "work rescue raise",
            "begin; work; rescue => e; raise(e); end",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn nearest_any_ensure_descendants_exclude_root_and_include_typed_children() {
        for (branch, n) in [
            ("e", 1),
            ("(e)", 0),
            ("use(e)", 0),
            ("x = e", 0),
            ("def other; e = 1; use(e); end", 0),
            ("nil", 1),
        ] {
            let source =
                format!("begin\n begin; work; rescue => e; raise; end\nensure\n {branch}\nend");
            assert_eq!(offenses(&source).len(), n, "{source}");
        }
        assert_eq!(
            offenses(
                "begin; begin; work; rescue => e; raise; ensure; nil; end; ensure; use(e); end"
            )
            .len(),
            1
        );
    }
}
