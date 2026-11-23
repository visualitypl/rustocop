//! `Style/MultilineBlockChain` — avoid chaining after a multiline block.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;
pub struct MultilineBlockChain;
impl Cop for MultilineBlockChain {
    fn name(&self) -> &'static str {
        "Style/MultilineBlockChain"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_block_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(block) = node.as_block_node() else {
            return;
        };
        let Some(root) = cx.ancestors().next_back() else {
            return;
        };
        if let Some(&(start, end)) = file
            .block_chain_facts(root)
            .findings
            .get(&block.opening_loc().start_offset())
        {
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: "Avoid multi-line chains of blocks.".into(),
                line,
                col,
                end_line: file.line_of(end.saturating_sub(1)),
                correction: None,
            });
        }
    }
}
fn literal_block<'pr>(node: &Node<'pr>) -> Option<ruby_prism::BlockNode<'pr>> {
    if let Some(call) = node.as_forwarding_super_node() {
        return call.block();
    }
    if let Some(call) = node.as_super_node() {
        return call.block()?.as_block_node();
    }
    node.as_call_node()?.block()?.as_block_node()
}
fn bad_receiver(node: &Node<'_>, file: &SourceFile) -> Option<usize> {
    let receiver = node.as_call_node()?.receiver()?;
    let (open, close) = if let Some(l) = receiver.as_lambda_node() {
        (l.opening_loc(), l.closing_loc())
    } else {
        let b = literal_block(&receiver)?;
        (b.opening_loc(), b.closing_loc())
    };
    (file.line_of(open.start_offset()) != file.line_of(close.start_offset()))
        .then_some(close.start_offset())
}
/// One complete main-parse pass. A call's receiver/arguments have been visited
/// when its BlockNode is entered; its body has not. This is precisely RuboCop's
/// send subtree, without repeatedly walking large argument lists or block bodies.
/// Bottom-up first matches also support sends nested in arguments. RuboCop
/// deduplicates equal line/column offenses; keep the outermost highlight.
#[derive(Default)]
pub(crate) struct Facts {
    findings: HashMap<usize, (usize, usize)>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        struct Frame {
            first: Option<usize>,
            end: usize,
        }
        struct Index<'a> {
            file: &'a SourceFile,
            frames: Vec<Frame>,
            chosen: HashMap<usize, (usize, usize)>,
            visits: usize,
        }
        impl<'pr> NodeVisitor<'pr> for Index<'_> {
            fn enter(&mut self, node: Node<'pr>) {
                self.visits += 1;
                if let Some(b) = node.as_block_node()
                    && let Some(parent) = self.frames.last()
                    && let Some(start) = parent.first
                {
                    let candidate = (b.opening_loc().start_offset(), parent.end);
                    self.chosen
                        .entry(start)
                        .and_modify(|old| {
                            if candidate.1 > old.1 {
                                *old = candidate;
                            }
                        })
                        .or_insert(candidate);
                }
                self.frames.push(Frame {
                    first: bad_receiver(&node, self.file),
                    end: ast::source_end(&node),
                });
            }
            fn leave(&mut self) {
                let frame = self.frames.pop().unwrap();
                if let Some(parent) = self.frames.last_mut()
                    && parent.first.is_none()
                {
                    parent.first = frame.first;
                }
            }
        }
        let mut index = Index {
            file,
            frames: Vec::new(),
            chosen: HashMap::new(),
            visits: 0,
        };
        walk(&mut index, root);
        Self {
            findings: index
                .chosen
                .into_iter()
                .map(|(start, (key, end))| (key, (start, end)))
                .collect(),
            #[cfg(test)]
            visits: index.visits,
        }
    }
}

pub(crate) static BLOCK_CHAIN_FACTS: crate::source::Fact<Facts> = crate::source::Fact::new();

impl crate::source::SourceFile {
    pub(crate) fn block_chain_facts(&self, root: &ruby_prism::Node<'_>) -> &Facts {
        BLOCK_CHAIN_FACTS.get(self, || Facts::build(root, self))
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/multiline_block_chain_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;
    #[test]
    fn pinned_offenses_and_implicit_parameters() {
        for op in [".", "&."] {
            for body in ["b", "_1", "it", "next 1", "break 1"] {
                let source = format!("a do\n {body}\nend{op}c1.c2 do\n d\nend");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == MultilineBlockChain.name())
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (3, 1));
                assert_eq!(found[0].message, "Avoid multi-line chains of blocks.");
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn nested_argument_chains_deduplicate_at_the_original_closing_delimiter() {
        let source = "a.b(a do\n b\nend.c do\n d\nend) do z end";
        let found: Vec<_> = lint_str(source)
            .into_iter()
            .filter(|o| o.cop == MultilineBlockChain.name())
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 1));
        assert!(
            !lint_str("a do\n b\nend.c do x end.d { y }")
                .iter()
                .any(|o| o.cop == MultilineBlockChain.name() && o.col > 1)
        );
        assert_eq!(
            lint_str("a do\n b\nend.c do x end.d { y }")
                .iter()
                .filter(|o| o.cop == MultilineBlockChain.name())
                .count(),
            1
        );
    }
    #[test]
    fn three_blocks_and_brace_receiver() {
        for source in [
            "a do\n b\nend.c do\n d\nend.e do\n f\nend",
            "a {\n b\n}.c {\n d\n}.e { f }",
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == MultilineBlockChain.name())
                .collect();
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(3, 1), (5, 1)]
            );
        }
    }
    #[test]
    fn pinned_negative_lookalikes() {
        for source in [
            "a { b }.c { d }",
            "w do x end.y do z end",
            "a do\n b\nend.c.d",
            "a1.a2.a3 do\n b\nend",
            "a { b }.c {\n d\n}",
            "(a do\n b\nend).c { d }",
            "a do\n b\nend.c(&other)",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == MultilineBlockChain.name()),
                "{source}"
            );
        }
    }
}
