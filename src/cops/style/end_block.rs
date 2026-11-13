//! `Style/EndBlock` — prefer at_exit to Perl-style shutdown hooks.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct EndBlock;
impl Cop for EndBlock {
    fn name(&self) -> &'static str {
        "Style/EndBlock"
    }
    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_post_execution_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(hook) = node.as_post_execution_node() else {
            return;
        };
        let keyword = hook.keyword_loc();
        let (line, col) = file.line_col(keyword.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid the use of `END` blocks. Use `Kernel#at_exit` instead.".into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    let root = cx.ancestors().next_back()?;
                    if !file
                        .end_block_facts(root)
                        .proven
                        .contains(&keyword.start_offset())
                        || file.class_method_bindings(root).observes_sources
                    {
                        return None;
                    }
                    let edit = crate::offense::Correction {
                        range: keyword.start_offset()..keyword.end_offset(),
                        replacement: "at_exit".into(),
                    };
                    super::correction::preserves_data_positions(file, cx, &edit).then_some(edit)
                })
                .flatten(),
        });
    }
}

/// One correction-only main-AST pass. Only direct, unused top-level statements
/// in a closed eager prefix have proven one-shot registration and core lookup.
/// END bodies are deferred, so their calls do not close the prefix. BEGIN runs
/// ahead of the prefix regardless of source order and invalidates it entirely.
#[derive(Default)]
pub(crate) struct Facts {
    pub(super) proven: std::collections::HashSet<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Index {
            facts: Facts,
            stopped: bool,
            begin: bool,
            depth: usize,
            hooks: Vec<usize>,
            unsafe_hooks: std::collections::HashSet<usize>,
            stack: Vec<bool>,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Index {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                let hook = node.as_post_execution_node().is_some();
                self.begin |= node.as_pre_execution_node().is_some();
                if self.hooks.is_empty() {
                    if hook && self.depth == 2 && !self.stopped {
                        self.facts.proven.insert(node.location().start_offset());
                    } else if !matches!(
                        node,
                        Node::ProgramNode { .. }
                            | Node::StatementsNode { .. }
                            | Node::LocalVariableWriteNode { .. }
                            | Node::LocalVariableReadNode { .. }
                            | Node::ParenthesesNode { .. }
                            | Node::ArrayNode { .. }
                            | Node::HashNode { .. }
                            | Node::AssocNode { .. }
                            | Node::IntegerNode { .. }
                            | Node::FloatNode { .. }
                            | Node::StringNode { .. }
                            | Node::SymbolNode { .. }
                            | Node::NilNode { .. }
                            | Node::TrueNode { .. }
                            | Node::FalseNode { .. }
                            | Node::SourceLineNode { .. }
                            | Node::SourceFileNode { .. }
                            | Node::SourceEncodingNode { .. }
                    ) {
                        self.stopped = true;
                    }
                }
                if hook {
                    if !self.hooks.is_empty() {
                        self.unsafe_hooks.extend(self.hooks.iter().copied());
                    }
                    self.hooks.push(node.location().start_offset());
                }
                // END has no lexical scope; an ordinary block does. Shared
                // assignment classification catches pattern/rescue/for targets.
                // In Ruby 3.4 END also has an extra synthetic block level in
                // backtraces. Even an unknown call can expose that difference
                // through an unhandled error without an explicit observer node.
                // Prove only closed, non-raising body syntax: no dispatch,
                // implicit conversions, autoloads, writes or control transfers.
                let unsafe_body = crate::cops::variables::is_local_assignment(&node)
                    || !matches!(
                        node,
                        Node::PostExecutionNode { .. }
                            | Node::StatementsNode { .. }
                            | Node::BeginNode { .. }
                            | Node::EnsureNode { .. }
                            | Node::ElseNode { .. }
                            | Node::IfNode { .. }
                            | Node::UnlessNode { .. }
                            | Node::AndNode { .. }
                            | Node::OrNode { .. }
                            | Node::ParenthesesNode { .. }
                            | Node::ArrayNode { .. }
                            | Node::IntegerNode { .. }
                            | Node::FloatNode { .. }
                            | Node::StringNode { .. }
                            | Node::SymbolNode { .. }
                            | Node::NilNode { .. }
                            | Node::TrueNode { .. }
                            | Node::FalseNode { .. }
                            | Node::LocalVariableReadNode { .. }
                            | Node::SourceLineNode { .. }
                            | Node::SourceFileNode { .. }
                            | Node::SourceEncodingNode { .. }
                    );
                if unsafe_body {
                    self.unsafe_hooks.extend(self.hooks.iter().copied());
                }
                self.stack.push(hook);
                self.depth += 1;
            }
            fn leave(&mut self) {
                self.depth -= 1;
                if self.stack.pop().unwrap() {
                    self.hooks.pop();
                }
            }
        }
        let mut index = Index::default();
        crate::cops::walk::walk(&mut index, root);
        if index.begin {
            index.facts.proven.clear();
        }
        index
            .facts
            .proven
            .retain(|start| !index.unsafe_hooks.contains(start));
        index.facts
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/end_block_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};

    #[test]
    fn pinned_keyword_position_message_and_individually_withheld_correction() {
        let found = correction_offenses(&EndBlock, "END { test }\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
        assert_eq!(
            found[0].message,
            "Avoid the use of `END` blocks. Use `Kernel#at_exit` instead."
        );
        // Pinned spec :10's correction can change an unhandled error backtrace.
        assert!(found[0].correction.is_none());
        assert_correction(&EndBlock, "END { :test }\n", "at_exit { :test }\n");
    }
    #[test]
    fn pinned_other_block_and_keyword_lookalikes_are_clean() {
        for source in [
            "end_block { test }\n",
            "at_exit { test }",
            "Kernel.at_exit { test }",
            "begin; test; ensure; cleanup; end",
            "BEGIN { test }",
            "'END {}'\n# END {}\n__END__\nEND {}\n",
        ] {
            assert!(
                correction_offenses(&EndBlock, source).is_empty(),
                "{source}"
            );
        }
    }
    fn assert_withheld(source: &str) {
        assert_eq!(
            crate::parser::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        let found = correction_offenses(&EndBlock, source);
        assert!(!found.is_empty(), "{source}");
        assert!(
            found.iter().all(|o| o.correction.is_none()),
            "{source}: {found:?}"
        );
    }

    #[test]
    fn repeated_registration_consumed_values_and_uncertain_dispatch_are_withheld() {
        for source in [
            "def f; END { :work }; end; f; f",
            "p proc { END { :work } }.call",
            "p -> { END { :work } }.call",
            "2.times { END { :work } }",
            "while condition; END { :work }; end",
            "END { END { :work } }",
            "p(begin; END { :work }; end)",
            "x = (END { :work })",
            "begin; END { :work }; rescue; fallback; end",
            "def at_exit; yield; end; END { :work }",
            "def self.at_exit; yield; end; END { :work }",
            "require 'changes'; END { :work }",
            "object.change; END { :work }",
            "at_exit { :work }; END { :work }",
            "END { :work }; BEGIN { def at_exit; yield; end }",
        ] {
            assert_withheld(source);
        }
    }

    #[test]
    fn local_declarations_and_control_flow_inside_hooks_are_withheld() {
        for source in [
            "END { x = 1 }; p defined?(x)",
            "END { x, y = [1,2] }; p defined?(x)",
            "END { /(?<x>x)/ =~ 'x' }; p defined?(x)",
            "END { 1 => x }; p defined?(x)",
            "END { begin; raise 'x'; rescue => e; p e; end }; p defined?(e)",
            "END { for x in [1]; p x; end }; p defined?(x)",
            "END { p it }",
            "END { p _1 }",
            "END { p _9 }",
            "END { return }",
            "END { break }",
            "END { next }",
        ] {
            assert_withheld(source);
        }
    }

    #[test]
    fn simple_prefix_hooks_and_source_envelopes_have_token_only_corrections() {
        for before in [
            "END{}",
            "  END {\n    :work\n  } # kept\n",
            "END { :first }\nEND { :second }\n",
            "x = 1\nEND { [x,__LINE__,__ENCODING__] }\n",
            "\u{feff}# encoding: UTF-8\r\nEND { 'é' }\r\np __LINE__\r\n__END__\r\nEND { raw }\r\n",
            "END { <<~TEXT }\n  END {}\nTEXT\n",
            "x = true\nEND { begin; if x; :yes; else; :no; end; ensure; nil; end }\n",
        ] {
            let after = if let Some((code, data)) = before.split_once("__END__") {
                format!("{}__END__{data}", code.replace("END", "at_exit"))
            } else if before.contains("<<~TEXT") {
                before.replacen("END", "at_exit", 1)
            } else {
                before.replace("END", "at_exit")
            };
            assert_correction(&EndBlock, before, &after);
        }
    }

    #[test]
    fn source_data_and_code_observers_withhold_but_keep_findings() {
        for source in [
            "END { :value }; p caller",
            "END { :value }; p DATA.pos;\n__END__\npayload",
            "END { p caller }",
            "END { begin; raise 'x'; rescue => e; p e.backtrace; end }",
            "END { p RubyVM::InstructionSequence.of(method(:work)) }",
            "END { p DATA.pos }; __END__\npayload",
        ] {
            // __END__ must be a standalone line.
            assert_withheld(&source.replace("; __END__", ";\n__END__"));
        }
    }

    #[test]
    fn potentially_raising_bodies_keep_offenses_without_backtrace_changing_edits() {
        for source in [
            "END { test }",
            "END { raise 'x' }",
            "END { fail 'x' }",
            "END { work }; def work; raise 'x'; end",
            "END { \"#{object}\" }",
            "END { [*object] }",
            "END { {object => 1} }",
            "END { SomeConstant }",
            "END { object.value = 1 }",
            "END { @value = 1 }; freeze",
        ] {
            assert_withheld(source);
        }
    }

    #[test]
    fn detection_covers_methods_procs_lambdas_loops_and_nested_hooks() {
        for source in [
            "def f\n  END { work }\nend",
            "proc {\n  END { work }\n}",
            "-> {\n  END { work }\n}",
            "2.times {\n  END { work }\n}",
            "END {\n  END { work }\n}",
        ] {
            let found: Vec<_> = crate::pipeline::lint_str(source)
                .into_iter()
                .filter(|o| o.cop == EndBlock.name())
                .collect();
            let offense = found
                .iter()
                .find(|o| o.line == 2)
                .unwrap_or_else(|| panic!("{source}: {found:?}"));
            assert_eq!((offense.col, offense.end_line), (3, 2));
            assert_eq!(
                offense.message,
                "Avoid the use of `END` blocks. Use `Kernel#at_exit` instead."
            );
        }
    }
}
