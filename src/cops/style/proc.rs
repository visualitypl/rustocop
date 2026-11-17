//! `Style/Proc` — prefer `proc` for literal-block `Proc.new`.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Proc;
impl Cop for Proc {
    fn name(&self) -> &'static str {
        "Style/Proc"
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
        if !proc_new(node) {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().next_back()?;
                let range = start..end;
                if !file.proc_facts(root).proven.contains(&start)
                    || super::correction::contains_comment(file, range.clone())
                    || file.bytes[range.clone()]
                        .iter()
                        .any(|b| matches!(b, b'\n' | b'\r'))
                {
                    return None;
                }
                // `Proc.new()do` has no token gap. The shorter bare selector
                // needs one so the keyword cannot merge into `procdo`.
                let block = node.as_call_node()?.block()?.as_block_node()?;
                let tight_do = block.opening_loc().as_slice() == b"do"
                    && block.opening_loc().start_offset() == end;
                let edit = Correction {
                    range,
                    replacement: if tight_do { "proc " } else { "proc" }.into(),
                };
                Some(edit)
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: "Use `proc` instead of `Proc.new`.".into(),
            correction,
        });
    }
}
fn proc_new(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.name().as_slice() == b"new"
            && !call.is_safe_navigation()
            && call.arguments().is_none()
            && call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["Proc"]))
            && call.block().is_some_and(|b| b.as_block_node().is_some())
    })
}

/// Closed eager top-level creation points, before any unknown executable code.
/// Both constant and Kernel method lookup must remain core. No class/inherited,
/// refined, deferred or dynamic context is inferred safe. This is a single lazy
/// main-AST walk; later redefinitions do not invalidate earlier eager creation.
#[derive(Default)]
pub(crate) struct Facts {
    proven: std::collections::HashSet<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Index {
            facts: Facts,
            stopped: bool,
            deferred: usize,
            stack: Vec<(bool, bool)>,
            begin: bool,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Index {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                self.begin |= node.as_pre_execution_node().is_some();
                let scope = matches!(
                    node,
                    Node::LambdaNode { .. }
                        | Node::BlockNode { .. }
                        | Node::DefNode { .. }
                        | Node::ClassNode { .. }
                        | Node::ModuleNode { .. }
                        | Node::SingletonClassNode { .. }
                );
                let candidate = proc_new(&node);
                let unknown_call = node.as_call_node().is_some() && !candidate;
                if self.deferred == 0 && !self.stopped {
                    if candidate {
                        self.facts.proven.insert(node.location().start_offset());
                    } else if !ast::is_global_const(&node, &["Proc"])
                        && !matches!(
                            node,
                            Node::ProgramNode { .. }
                                | Node::StatementsNode { .. }
                                | Node::LocalVariableWriteNode { .. }
                                | Node::LocalVariableReadNode { .. }
                                | Node::ParenthesesNode { .. }
                                | Node::ArrayNode { .. }
                                | Node::HashNode { .. }
                                | Node::KeywordHashNode { .. }
                                | Node::AssocNode { .. }
                                | Node::ArgumentsNode { .. }
                                | Node::CallNode { .. }
                                | Node::BlockNode { .. }
                                | Node::LambdaNode { .. }
                                | Node::IntegerNode { .. }
                                | Node::FloatNode { .. }
                                | Node::StringNode { .. }
                                | Node::SymbolNode { .. }
                                | Node::NilNode { .. }
                                | Node::TrueNode { .. }
                                | Node::FalseNode { .. }
                        )
                    {
                        self.stopped = true;
                    }
                }
                self.stack.push((scope, unknown_call && self.deferred == 0));
                self.deferred += usize::from(scope);
            }
            fn leave(&mut self) {
                let (scope, unknown) = self.stack.pop().unwrap();
                self.deferred -= usize::from(scope);
                self.stopped |= unknown;
            }
        }
        let mut index = Index::default();
        crate::cops::walk::walk(&mut index, root);
        if index.begin {
            index.facts.proven.clear();
        }
        index.facts
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/proc_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn all_pinned_diagnostics_and_corrections() {
        for (before, after) in [
            ("f = Proc.new { |x| puts x }\n", "f = proc { |x| puts x }\n"),
            (
                "f = ::Proc.new { |x| puts x }\n",
                "f = proc { |x| puts x }\n",
            ),
            ("f = Proc.new { puts _1 }\n", "f = proc { puts _1 }\n"),
            ("f = Proc.new { puts it }\n", "f = proc { puts it }\n"),
        ] {
            let found: Vec<_> = lint_str(before)
                .into_iter()
                .filter(|o| o.cop == Proc.name())
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 5, 1));
            assert_eq!(found[0].message, "Use `proc` instead of `Proc.new`.");
            assert!(found[0].correction.is_none());
            assert_correction(&Proc, before, after);
        }
    }
    #[test]
    fn no_block_or_wrong_dispatch_is_not_a_match() {
        for source in [
            "p = Proc.new",
            "p = ::Proc.new",
            "Proc.new(&block)",
            "Proc.new(1) { 1 }",
            "Other::Proc.new { 1 }",
            "proc.new { 1 }",
            "Proc&.new { 1 }",
            "Proc.allocate { 1 }",
            "proc { _1 }",
            "-> { it }",
            "'Proc.new {}' # Proc.new {}",
        ] {
            assert!(
                !lint_str(source).iter().any(|o| o.cop == Proc.name()),
                "{source}"
            );
        }
        assert_correction(&Proc, "f = Proc.new() { || 1 }", "f = proc { || 1 }");
        assert_correction(&Proc, "f = Proc.new()do 1 end", "f = proc do 1 end");
    }
    #[test]
    fn unknown_method_constant_and_deferred_lookups_stay_reported_but_unfixed() {
        for prefix in [
            "def proc(&b); b; end\n",
            "def self.proc(&b); b; end\n",
            "alias proc lambda\n",
            "undef proc\n",
            "private :proc\n",
            "public :proc\n",
            "define_method(:proc) { |&b| b }\n",
            "attr_reader :proc\n",
            "singleton_class.attr_accessor(:proc)\n",
            "remove_method :proc\n",
            "undef_method :proc\n",
            "Proc = Other\n",
            "Object.const_set(:Proc, Other)\n",
            "class Proc; def self.new(&b); b; end; end\n",
            "class Object; def proc(&b); b; end; end\n",
            "module R; refine Object do; def proc(&b); b; end; end; end; using R\n",
            "eval('def proc; end')\n",
            "unknown\n",
        ] {
            let source = format!("{prefix}f = Proc.new {{ |x| x }}\n");
            let found = correction_offenses(&Proc, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for source in [
            "class C < Parent; def f; Proc.new { 1 }; end; end",
            "class C; Proc = Other; Proc.new { 1 }; end",
            "obj.instance_eval { Proc.new { 1 } }",
            "def f; Proc.new { 1 }; end",
            "f = -> { Proc.new { _1 } }",
            "f = Proc.new { Proc.new { it } }",
            "f = Proc.new { 1 }; BEGIN { def proc(&b); b; end }",
        ] {
            let found = correction_offenses(&Proc, source);
            assert!(!found.is_empty(), "{source}");
            if source == "f = Proc.new { Proc.new { it } }" {
                assert!(found[0].correction.is_some());
                assert!(found[1].correction.is_none());
            } else {
                assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
            }
        }
    }
    #[test]
    fn source_order_proof_is_not_a_whole_file_blanket_disable() {
        assert_correction(
            &Proc,
            "a = Proc.new { 1 }; unknown; b = Proc.new { 2 }",
            "a = proc { 1 }; unknown; b = Proc.new { 2 }",
        );
        assert_correction(
            &Proc,
            "a = Proc.new { 1 }; def proc(&b); b; end; b = Proc.new { 2 }",
            "a = proc { 1 }; def proc(&b); b; end; b = Proc.new { 2 }",
        );
    }
    #[test]
    fn horizontal_selector_only_leaves_comments_literals_and_heredocs_in_place() {
        for (before, after) in [
            (
                "f = Proc.new do |a = __LINE__| # keep\n [a, <<~TEXT]\n literal } | #\nTEXT\nend",
                "f = proc do |a = __LINE__| # keep\n [a, <<~TEXT]\n literal } | #\nTEXT\nend",
            ),
            (
                "f = Proc . new { |x; y| y=x; [y, 'Proc.new'] }",
                "f = proc { |x; y| y=x; [y, 'Proc.new'] }",
            ),
            ("f = Proc. # keep\nnew { 1 }", "f = Proc. # keep\nnew { 1 }"),
            ("f = Proc.\nnew { 1 }", "f = Proc.\nnew { 1 }"),
        ] {
            assert_correction(&Proc, before, after);
        }
    }
}
