//! `Lint/HeredocMethodCallPosition` — opt-in heredoc receiver ordering.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct HeredocMethodCallPosition;

#[cfg(test)]
thread_local! {
    static RECEIVER_EDGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl Cop for HeredocMethodCallPosition {
    fn name(&self) -> &'static str {
        "Lint/HeredocMethodCallPosition"
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
        let (opening, closing) = match node {
            Node::StringNode { .. } => {
                let n = node.as_string_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            Node::InterpolatedStringNode { .. } => {
                let n = node.as_interpolated_string_node().unwrap();
                (n.opening_loc(), n.closing_loc())
            }
            Node::XStringNode { .. } => {
                let n = node.as_x_string_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            Node::InterpolatedXStringNode { .. } => {
                let n = node.as_interpolated_x_string_node().unwrap();
                (Some(n.opening_loc()), Some(n.closing_loc()))
            }
            _ => return,
        };
        let Some(opening) = opening.filter(|l| l.as_slice().starts_with(b"<<")) else {
            return;
        };
        let Some(closing) = closing else { return };
        // Start at the literal once, not a root/receiver rescan for every send.
        // Every upstream send in this receiver chain reports the SAME range;
        // RuboCop deduplicates it, even when the call is several lines later.
        let mut child = node;
        let mut misplaced = false;
        for parent in cx.ancestors() {
            #[cfg(test)]
            RECEIVER_EDGES.with(|n| n.set(n.get() + 1));
            let Some((receiver, end, terminal)) = receiver_send(parent) else {
                break;
            };
            if receiver.location().start_offset() != child.location().start_offset()
                || receiver.location().end_offset() != child.location().end_offset()
            {
                break;
            }
            misplaced |= end >= closing.end_offset();
            if terminal {
                break;
            }
            child = parent;
        }
        if !misplaced {
            return;
        }
        let (line, col) =
            super::array_literal_in_regexp::reference_line_col(closing.end_offset(), file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message:
                "Put a method call with a HEREDOC receiver on the same line as the HEREDOC opening."
                    .into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    closed_reference(node, opening.start_offset(), closing.end_offset(), cx, file)
                })
                .flatten(),
        });
    }
}

// Prism folds an assignment's synthetic send into its write/target node.
// A literal block is a synthetic block receiver upstream: check its send, then
// stop the chain. The RHS of a compound write is never a receiver edge.
fn receiver_send<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, usize, bool)> {
    macro_rules! write {
        ($access:ident) => {{
            let n = node.$access()?;
            (n.receiver()?, n.message_loc()?.end_offset(), true)
        }};
    }
    macro_rules! index_write {
        ($access:ident) => {{
            let n = node.$access()?;
            // The synthetic [] send ends at ], not at the assignment's RHS.
            (n.receiver()?, n.closing_loc().end_offset(), true)
        }};
    }
    Some(match node {
        Node::CallNode { .. } => {
            let n = node.as_call_node()?;
            (
                n.receiver()?,
                crate::cops::ast::source_end(node),
                n.block().is_some_and(|b| b.as_block_node().is_some()),
            )
        }
        Node::CallAndWriteNode { .. } => write!(as_call_and_write_node),
        Node::CallOrWriteNode { .. } => write!(as_call_or_write_node),
        Node::CallOperatorWriteNode { .. } => write!(as_call_operator_write_node),
        Node::CallTargetNode { .. } => {
            let n = node.as_call_target_node()?;
            (n.receiver(), n.message_loc().end_offset(), true)
        }
        Node::IndexOperatorWriteNode { .. } => index_write!(as_index_operator_write_node),
        Node::IndexOrWriteNode { .. } => index_write!(as_index_or_write_node),
        Node::IndexAndWriteNode { .. } => index_write!(as_index_and_write_node),
        Node::IndexTargetNode { .. } => {
            let n = node.as_index_target_node()?;
            (n.receiver(), n.closing_loc().end_offset(), true)
        }
        _ => return None,
    })
}

// Relocating an ordinary invocation changes dispatch/exception/trace locations.
// Only an immediate, unexecuted method query in a closed consumed top-level
// array is proved here. No delayed scope, arguments, interpolation, commands,
// chained calls, comments/header tails or queued heredocs enter this envelope.
// The common main-parse boundary/source/code-object/heap/position proofs still
// filter the actual atomic edit; no alternate parse or global safety exemption.
fn closed_reference(
    node: &Node<'_>,
    start: usize,
    close_end: usize,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let string = node.as_string_node()?;
    if file.data_start().is_some() || close_end.checked_sub(start)? > 4096 {
        return None;
    }
    let mut ancestors = cx.ancestors();
    let call = ancestors.next()?.as_call_node()?;
    if call.block().is_some() || call.arguments().is_some() {
        return None;
    }
    let open_end = string.opening_loc()?.end_offset();
    let end = call.location().end_offset();
    let suffix = file.bytes.get(close_end..end)?;
    if !matches!(
        suffix,
        b".strip_indent" | b".strip" | b".freeze" | b"&.strip_indent" | b"&.strip" | b"&.freeze"
    ) {
        return None;
    }
    let header = file
        .bytes
        .get(open_end..string.content_loc().start_offset())?;
    if !matches!(header, b"\n" | b"\r\n") {
        return None;
    }
    ancestors.next()?.as_defined_node()?;
    let array = ancestors.next()?.as_array_node()?;
    let statements = ancestors.next()?.as_statements_node()?;
    ancestors.next()?.as_program_node()?;
    if ancestors.next().is_some() || array.elements().len() != 1 || statements.body().len() != 1 {
        return None;
    }
    let envelope = file.bytes.get(start..close_end)?;
    let retained = envelope
        .strip_suffix(b"\r\n")
        .or_else(|| envelope.strip_suffix(b"\n"))?;
    let opening_len = open_end - start;
    let mut replacement = retained[..opening_len].to_vec();
    replacement.extend_from_slice(suffix);
    replacement.extend_from_slice(&retained[opening_len..]);
    Some(crate::offense::Correction {
        range: start..end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/heredoc_method_call_position_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str_with;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&HeredocMethodCallPosition], source)
    }
    #[test]
    fn pinned_offenses_are_deduplicated_at_the_first_byte_after_the_terminator() {
        for (prefix, suffix) in [
            ("", ".strip_indent\n"),
            ("", ".foo(bar_baz)\n"),
            ("", ".strip_indent.foo\n"),
            ("", ".abc(1, 2, 3).foo\n"),
            ("bar(", ".abc,\n)\n"),
            ("", ".abc(1, 2,\n3).foo\n"),
            ("", ".abc\n.foo\n"),
            ("", "&.strip_indent\n"),
        ] {
            let source = format!("{prefix}<<-SQL\n  foo\nSQL\n{suffix}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 1, 4));
            assert_eq!(
                found[0].message,
                "Put a method call with a HEREDOC receiver on the same line as the HEREDOC opening."
            );
        }
    }
    #[test]
    fn synthetic_assignment_sends_are_real_heredoc_receivers() {
        for suffix in [
            ".foo += 1",
            ".foo ||= 1",
            ".foo &&= 1",
            ".foo, other = 1, 2",
            "&.foo += 1",
        ] {
            let found = offenses(&format!("<<-SQL\nfoo\nSQL\n{suffix}\n"));
            assert_eq!(found.len(), 1, "{suffix}");
            assert_eq!((found[0].line, found[0].col), (4, 1));
        }
    }
    #[test]
    fn synthetic_index_assignment_sends_exclude_the_rhs() {
        for assignment in ["+=", "||=", "&&=", ", other ="] {
            let source = format!("<<~SQL[\nbody\nSQL\n0] {assignment} 1\n");
            let found =
                crate::cops::testing::correction_offenses(&HeredocMethodCallPosition, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 1, 4));
            assert_eq!(
                found[0].message,
                "Put a method call with a HEREDOC receiver on the same line as the HEREDOC opening."
            );
            assert!(found[0].correction.is_none(), "{source}");
            for source in [
                format!("<<~SQL[0] {assignment} 1\nbody\nSQL\n"),
                format!("<<~SQL[0] {assignment} (\nbody\nSQL\n1)\n"),
                format!("items[<<~SQL] {assignment} 1\nbody\nSQL\n"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn canonical_relocation_only_in_a_closed_consumed_method_reference() {
        crate::cops::testing::assert_correction(
            &HeredocMethodCallPosition,
            "[defined?(<<-SQL\n  foo\nSQL\n.strip_indent\n)]\n",
            "[defined?(<<-SQL.strip_indent\n  foo\nSQL\n)]\n",
        );
    }
    #[test]
    fn wide_sources_walk_each_receiver_chain_only_once() {
        let source = "<<~SQL\nfoo\nSQL\n.strip.freeze\n".repeat(6000);
        RECEIVER_EDGES.with(|n| n.set(0));
        let found = offenses(&source);
        assert_eq!(found.len(), 6000);
        RECEIVER_EDGES.with(|n| assert_eq!(n.get(), 18000));
    }
    #[test]
    fn pinned_correct_cases_and_nonreceiver_lookalikes() {
        for source in [
            "<<~SQL\n  foo\nSQL\n",
            "<<~SQL.bar\n  foo\nSQL\n",
            "<<-SQL\n  foo\nSQL\n",
            "consume(<<-SQL)\nfoo\nSQL\n.strip\n",
            "(<<-SQL\nfoo\nSQL\n).strip\n",
            "'heredoc'.strip\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
