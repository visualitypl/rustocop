//! `Style/AutoResourceCleanup` — suggest block-based resource cleanup, never change lifetime.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AutoResourceCleanup;
impl Cop for AutoResourceCleanup {
    fn name(&self) -> &'static str {
        "Style/AutoResourceCleanup"
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
        if call.is_safe_navigation() || call.name().as_slice() != b"open" || call.block().is_some()
        {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        if !["File", "Tempfile"]
            .iter()
            .any(|name| ast::is_global_const(&receiver, &[*name]))
        {
            return;
        }
        // RuboCop's root single statement has no parent. Prism wraps it in
        // Statements/Program; multiple statements instead have a synthetic begin.
        // Everywhere else upstream only reports direct local assignments.
        let mut ancestors = cx.ancestors();
        let mut parent = ancestors.next();
        if let Some(statements) = parent.and_then(|n| n.as_statements_node()) {
            if statements.body().len() != 1 {
                return;
            }
            parent = ancestors.next();
            if parent.is_some_and(|n| n.as_program_node().is_some()) {
                parent = None;
            }
        }
        if parent.is_some_and(|n| n.as_local_variable_write_node().is_none()) {
            return;
        }
        let start = call.location().start_offset();
        let end = ast::source_end(node);
        let current = &file.bytes
            [receiver.location().start_offset()..call.message_loc().unwrap().end_offset()];
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use the block version of `{}`.",
                String::from_utf8_lossy(current)
            ),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/auto_resource_cleanup_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::correction_offenses;

    #[test]
    fn pinned_bare_and_assigned_opens_are_detection_only() {
        for receiver in ["File", "Tempfile", "::File", "::Tempfile"] {
            for prefix in ["", "resource = "] {
                let source = format!("  {prefix}{receiver}.open(\"filename\")\n");
                let found = correction_offenses(&AutoResourceCleanup, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 3 + prefix.len() as u32));
                assert_eq!(
                    found[0].message,
                    format!("Use the block version of `{receiver}.open`.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn blocks_and_resource_consumers_are_not_reported() {
        for source in [
            "File.open('file') { |f| something }",
            "File.open('file', &:read)",
            "File.open('file', 'w', 0o777).close",
            "File.open('file').read",
            "consume(File.open('file'))",
            "return File.open('file')",
            "@f = File.open('file')",
            "F = File.open('file')",
            "[File.open('file')]",
            "foo; File.open('file')",
            "(File.open('file'))",
            "Other::File.open('file')",
            "file.open('file')",
            "open('file')",
            "File&.open('file')",
            "File.new('file')",
            "File.open('file', &cleanup)",
            "File.open('file') do |f| f.read end",
            "def resource; File.open('file'); end",
        ] {
            assert!(
                correction_offenses(&AutoResourceCleanup, source).is_empty(),
                "{source}"
            );
        }
    }
}
