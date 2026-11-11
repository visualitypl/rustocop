//! `Lint/TopLevelReturnWithArgument` — arguments discarded by a top-level return.
//!
//! Detection only: the 2026-11-04 no-source-edit exception preserves
//! the observable default/-W1/-w compiler warning, even for primitive arguments.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct TopLevelReturnWithArgument;
impl Cop for TopLevelReturnWithArgument {
    fn name(&self) -> &'static str {
        "Lint/TopLevelReturnWithArgument"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(ret) = node.as_return_node() else {
            return;
        };
        let Some(arguments) = ret.arguments() else {
            return;
        };
        let args = arguments.arguments();
        if args.is_empty()
            || (args.len() == 1
                && args
                    .iter()
                    .next()
                    .unwrap()
                    .as_parentheses_node()
                    .is_some_and(|p| p.body().is_none()))
        {
            return;
        }
        // RuboCop's any_block wraps the entire send, including receiver and
        // arguments. Prism instead makes a literal block a child of that send.
        if cx.ancestors().any(|a| {
            matches!(
                a,
                Node::DefNode { .. } | Node::BlockNode { .. } | Node::LambdaNode { .. }
            ) || a
                .as_call_node()
                .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()))
                || a.as_super_node()
                    .is_some_and(|s| s.block().is_some_and(|b| b.as_block_node().is_some()))
        }) {
            return;
        }
        let start = ret.keyword_loc().start_offset();
        let end = ast::source_end(node);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: "Top level return with argument detected.".into(),
            // Even primitive arguments produce a compile-time warning in
            // Ruby's default mode. The -W0 probe does not authorize
            // dropping that observation. The explicit fixed-policy
            // no-source-edit exception requires findings without corrections.
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/top_level_return_with_argument_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/TopLevelReturnWithArgument")
            .collect()
    }
    #[test]
    fn primitive_returns_retain_findings_under_warning_preserving_no_edit_exception() {
        use crate::cops::testing::correction_offenses;
        for (before, _reviewed_after) in [
            ("return 1, 2, 3\n", "return\n"),
            (
                "END { puts :finished }\nreturn 1, 2, 3 # keep\nraise 'unreachable'\n",
                "END { puts :finished }\nreturn # keep\nraise 'unreachable'\n",
            ),
            ("return nil, true, false, 42\r\n", "return\r\n"),
            ("return\t1,\t2, 3\n", "return\n"),
        ] {
            let found = correction_offenses(&super::TopLevelReturnWithArgument, before);
            assert_eq!(found.len(), 1, "{before}");
            assert!(found[0].correction.is_none(), "{before}: {found:?}");
        }
    }
    #[test]
    fn uncertain_edits_keep_findings_and_bytes() {
        use crate::cops::testing::correction_offenses;
        for s in [
            "return foo()",
            "return x = 1",
            "return *[1]",
            "return [1,2]",
            "return \"#{foo}\"",
            "return 1.0",
            "return 999999999999999999999999999999",
            "return 1, # keep\n2",
            "return (1)",
            "return 1\n__END__\nraw",
            "File.read(__FILE__)\nreturn 1",
            "binding.eval('local_variables')\nreturn 1",
            "RubyVM::InstructionSequence.of(method(:f))\nreturn 1",
            "ObjectSpace.count_objects\nreturn 1",
        ] {
            let found = correction_offenses(&super::TopLevelReturnWithArgument, s);
            assert!(!found.is_empty(), "{s}");
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{s}: {found:?}"
            );
        }
    }
    #[test]
    fn pinned_top_level_arguments_and_positions() {
        for (source, positions) in [
            ("return 1, 2, 3\n", vec![(1, 1)]),
            (
                "return 1, 2, 3\n\nreturn 1, 2, 3\n\nreturn 1, 2, 3\n",
                vec![(1, 1), (3, 1), (5, 1)],
            ),
            (
                "foo\n[1,2,3].each { |n| return n }\nreturn 1, 2, 3\nbar\n",
                vec![(3, 1)],
            ),
            (
                "def method\n return 'Hello World'\nend\nreturn 1, 2, 3\n",
                vec![(4, 1)],
            ),
            (
                "foo\nreturn 1, 2, 3 if 1 == 1\nbar\nreturn 2\nreturn 3\ndef method\n return 'Hello World' if 1 == 1\nend\n",
                vec![(2, 1), (4, 1), (5, 1)],
            ),
            (
                "foo\n\nif a == b; warn 'hey'; return 42; end\nbar\n",
                vec![(3, 24)],
            ),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{source}"
            );
            for o in found {
                assert_eq!(o.message, "Top level return with argument detected.");
                assert!(o.correction.is_none());
            }
        }
    }
    #[test]
    fn pinned_bare_returns_and_all_method_block_lookalikes() {
        for source in [
            "return",
            "return ()",
            "return()",
            "return (;)",
            "[1, 2, 3].each { return _1 }",
            "[1, 2, 3].each { return it }",
            "foo\n[1,2,3,4,5].each { |n| return n }\nreturn\nbar",
            "foo\nreturn if 1 == 1\nbar\ndef method\n return 'Hello World' if 1 == 1\nend",
            "foo\nif a == b; warn 'hey'; return; end\nbar",
            "def self.f; return 1; end",
            "-> { return 1 }",
            "lambda { return 1 }",
            "proc { return 1 }",
            "Proc.new { return 1 }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn detects_effectful_and_multiline_arguments_without_corrections() {
        for value in [
            "foo()",
            "x = 1",
            "*values",
            "\"#{foo}\"",
            "(())",
            "(begin; end)",
            "(\n 1\n)",
            "<<~DOC\ntext\nDOC\n",
        ] {
            let found = offenses(&format!("nil\n  return {value}\n"));
            assert_eq!(found.len(), 1, "{value}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
        }
    }
    #[test]
    fn super_literal_blocks_wrap_arguments_but_block_pass_does_not() {
        for source in [
            "super(\"#{return 1}\") {}",
            "super(\"#{return 1}\") do end",
            "super(\"#{return 1}\") { it }",
            "super(\"#{return 1}\") { _1 }",
            "super(*[\"#{return 1}\"]) {}",
            "super(key: \"#{return 1}\") do end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "super(\"#{return 1}\")",
            "super(\"#{return 1}\", &block)",
            "super(\"#{return 1}\", &nil)",
            "super(&\"#{return 1}\")",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].line, 1);
            assert_eq!(found[0].col, source.find("return").unwrap() as u32 + 1);
            assert_eq!(found[0].message, "Top level return with argument detected.");
        }
    }
    #[test]
    fn synthetic_blocks_wrap_receivers_and_arguments_not_just_bodies() {
        for source in [
            "consume(\"#{return 1}\") {}",
            "\"#{return 1}\".consume {}",
            "consume(\"#{return 1}\") { it }",
            "consume(\"#{return 1}\") { _1 }",
            "def (\"#{return 1}\").foo; end",
            "def f(x = (\"#{return 1}\")); end",
            "->(x = (\"#{return 1}\")) {}",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("\"#{return 1}\".consume(&block)").len(), 1);
    }
}
