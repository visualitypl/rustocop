//! `Style/FileOpen` — potentially unmanaged file descriptors (detection only).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct FileOpen;
impl Cop for FileOpen {
    fn name(&self) -> &'static str {
        "Style/FileOpen"
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
        if call.name().as_slice() != b"open"
            || call.is_safe_navigation()
            || call.block().is_some()
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["File"]))
        {
            return;
        }
        let local_or_chain = cx.parent().is_some_and(|p| {
            p.as_local_variable_write_node().is_some()
                || p.as_call_node()
                    .and_then(|c| c.receiver())
                    .is_some_and(|r| same(&r, node))
        });
        if !local_or_chain && value_used(node, cx) {
            return;
        }
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "`File.open` without a block may leak a file descriptor; use the block form."
                .into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction: None,
        });
    }
}
fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    a.location().start_offset() == b.location().start_offset()
        && a.location().end_offset() == b.location().end_offset()
}

/// RuboCop AST's value_used? adapted to Prism's transparent statement lists.
/// Unknown contexts conservatively consume the value. Ancestor depth is bounded
/// by the pipeline; no descendant scans or reparsing are needed per offense.
fn value_used(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut child = node;
    for parent in cx.ancestors() {
        if let Some(statements) = parent.as_statements_node() {
            if !statements
                .body()
                .last()
                .is_some_and(|last| same(&last, child))
            {
                return false;
            }
        } else if parent.as_program_node().is_some() {
            return false;
        } else if let Some(n) = parent.as_if_node() {
            if same(&n.predicate(), child) {
                return true;
            }
        } else if let Some(n) = parent.as_unless_node() {
            if same(&n.predicate(), child) {
                return true;
            }
        } else if let Some(n) = parent.as_while_node() {
            return same(&n.predicate(), child);
        } else if let Some(n) = parent.as_until_node() {
            return same(&n.predicate(), child);
        } else if let Some(n) = parent.as_for_node() {
            if same(&n.collection(), child) {
                return true;
            }
        } else if let Some(n) = parent.as_case_node() {
            if n.predicate().is_some_and(|p| same(&p, child)) {
                return true;
            }
        } else if let Some(n) = parent.as_begin_node() {
            if n.rescue_clause().is_some() || n.ensure_clause().is_some() {
                return true;
            }
        } else if !matches!(
            parent,
            Node::ParenthesesNode { .. }
                | Node::EmbeddedStatementsNode { .. }
                | Node::ElseNode { .. }
                | Node::WhenNode { .. }
                | Node::ArrayNode { .. }
                | Node::HashNode { .. }
                | Node::KeywordHashNode { .. }
                | Node::AssocNode { .. }
                | Node::DefinedNode { .. }
                | Node::InterpolatedStringNode { .. }
                | Node::InterpolatedSymbolNode { .. }
                | Node::InterpolatedRegularExpressionNode { .. }
                | Node::InterpolatedXStringNode { .. }
                | Node::RangeNode { .. }
                | Node::FlipFlopNode { .. }
        ) {
            return true;
        }
        child = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/file_open_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<crate::offense::Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/FileOpen")
            .collect()
    }
    #[test]
    fn pinned_offenses_positions_messages_and_detection_only() {
        for call in [
            "File.open('file')",
            "::File.open('file')",
            "File.open('file', 'w')",
            "File.open('file').read",
            "File.open('file', encoding: 'UTF-8')",
            "File.open(*paths)",
        ] {
            for prefix in ["", "f = "] {
                let source = format!("# header\n  {prefix}{call}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 3 + prefix.len() as u32));
                assert_eq!(
                    found[0].message,
                    "`File.open` without a block may leak a file descriptor; use the block form."
                );
                assert!(
                    correction_offenses(&FileOpen, &source)[0]
                        .correction
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn pinned_managed_usage_and_receiver_lookalikes() {
        for source in [
            "@ivar = File.open('file')",
            "@@cvar = File.open('file')",
            "$gvar = File.open('file')",
            "CONST = File.open('file')",
            "process(File.open('file'))",
            "process(io: File.open('file'))",
            "def json_key_io; File.open('file'); end",
            "File.open('file') { |f| f.read }",
            "File.open('file') do |f|\nf.read\nend",
            "File.open('file', &:read)",
            "File.open('file', &block)",
            "File.read('file')",
            "open('file')",
            "Foo.open('file')",
            "Foo::File.open('file')",
            "File&.open('file')",
            "return File.open('file')",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn many_sibling_values_use_constant_time_last_child_lookup() {
        let source = format!(
            "def f\n{}File.open('last')\nend\n",
            "File.open('discarded')\n".repeat(5000)
        );
        let found = offenses(&source);
        assert_eq!(found.len(), 5000);
        assert_eq!(found.last().unwrap().line, 5001);
    }

    #[test]
    fn enclosing_value_contexts_and_discarded_branch_values() {
        for source in [
            "def f; File.open('x'); nil; end",
            "while ready; File.open('x'); end",
            "if ready; File.open('x'); end",
            "(File.open('x'); nil)",
            "[File.open('x')]",
            "\"#{File.open('x')}\"",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "def f; if ready; File.open('x'); end; end",
            "f = (File.open('x'))",
            "p([File.open('x')])",
            "File.open('x') && other",
            "while File.open('x'); end",
            "p(\"#{File.open('x')}\")",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
