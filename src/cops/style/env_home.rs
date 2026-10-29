//! `Style/EnvHome` — prefer Dir.home (detection-only: not equivalent when HOME is unset).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct EnvHome;
impl Cop for EnvHome {
    fn name(&self) -> &'static str {
        "Style/EnvHome"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism represents compound index assignments without a CallNode;
        // RuboCop visits their synthetic read-side send as well.
        macro_rules! index_read {
            ($access:ident) => {{
                let n = node.$access().unwrap();
                if n.is_safe_navigation() {
                    return;
                }
                let mut args: Vec<_> = n
                    .arguments()
                    .map(|a| a.arguments().iter().collect())
                    .unwrap_or_default();
                if let Some(block) = n.block() {
                    args.push(block.as_node());
                }
                (n.receiver(), args, n.closing_loc().end_offset())
            }};
        }
        let (receiver, args, end) = match node {
            Node::IndexAndWriteNode { .. } => index_read!(as_index_and_write_node),
            Node::IndexOrWriteNode { .. } => index_read!(as_index_or_write_node),
            Node::IndexOperatorWriteNode { .. } => index_read!(as_index_operator_write_node),
            _ => {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if call.is_safe_navigation()
                    || !matches!(call.name().as_slice(), b"[]" | b"fetch")
                    || call.block().is_some_and(|b| b.as_block_node().is_some())
                {
                    return;
                }
                (
                    call.receiver(),
                    ast::call_arguments(&call),
                    ast::source_end(node),
                )
            }
        };
        if !receiver.is_some_and(|r| ast::is_global_const(&r, &["ENV"])) {
            return;
        }
        if !args
            .first()
            .is_some_and(|a| a.as_string_node().is_some_and(|s| s.unescaped() == b"HOME"))
            || (args.len() == 2 && args[1].as_nil_node().is_none())
        {
            return;
        }
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `Dir.home` instead.".into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/env_home_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    #[test]
    fn pinned_offenses_are_detection_only() {
        for receiver in ["ENV", "::ENV"] {
            for suffix in [
                "['HOME']",
                ".fetch('HOME')",
                ".fetch('HOME', nil)",
                "[\"H\\x4fME\"]",
            ] {
                let source = format!("# header\n  {receiver}{suffix}\n");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/EnvHome")
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3));
                assert_eq!(found[0].message, "Use `Dir.home` instead.");
                assert!(
                    correction_offenses(&EnvHome, &source)[0]
                        .correction
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn lexical_constant_rebinding_is_not_inferred_for_detection() {
        let found = correction_offenses(&EnvHome, "module M; ENV = Custom; ENV['HOME']; end");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 25));
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn compound_assignment_reads_are_reported_but_never_rewritten() {
        for source in [
            "ENV['HOME'] ||= value",
            "::ENV['HOME'] &&= value",
            "ENV['HOME'] += value",
        ] {
            let found = correction_offenses(&EnvHome, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].message, "Use `Dir.home` instead.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn fallbacks_blocks_assignments_and_lookalikes() {
        for source in [
            "Dir.home",
            "ENV['HOME'] = '/home/foo'",
            "ENV.fetch('HOME', default)",
            "ENV.fetch('HOME') { default }",
            "ENV.fetch('HOME', nil) do default end",
            "ENV.fetch('HOME', &fallback)",
            "ENV['OTHER']",
            "Other::ENV['HOME']",
            "env['HOME']",
            "ENV&.fetch('HOME')",
            "ENV[\"#{name}\"]",
            "ENV[:HOME]",
        ] {
            assert!(correction_offenses(&EnvHome, source).is_empty(), "{source}");
        }
    }
}
