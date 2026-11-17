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
        if !opening.is_some_and(|l| l.as_slice().starts_with(b"<<")) {
            return;
        }
        let Some(closing) = closing else { return };
        // Start at the literal once, not a root/receiver rescan for every send.
        // Every upstream send in this receiver chain reports the SAME range;
        // RuboCop deduplicates it, even when the call is several lines later.
        let mut child = node;
        let mut misplaced = false;
        let mut chain = Vec::new();
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
            chain.push(parent);
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
                .then(|| correction(&chain, &closing, file))
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

/// RuboCop's `autocorrect` for the outermost send of the receiver chain.
fn correction(
    chain: &[&Node<'_>],
    closing: &ruby_prism::Location<'_>,
    file: &SourceFile,
) -> Option<crate::offense::Correction> {
    let outer = *chain.last()?;
    let end = crate::cops::ast::source_end(outer);
    let last_line = file.line_of(end - 1);
    // `calls_on_multiple_lines?`
    for n in chain {
        let call = n.as_call_node()?;
        if file.line_of(crate::cops::ast::source_end(n) - 1) != last_line {
            return None;
        }
        let arguments = crate::cops::ast::call_arguments(&call);
        if let (Some(first), Some(last)) = (arguments.first(), arguments.last())
            && file.line_of(first.location().start_offset()) != file.last_line_of(last.location())
        {
            return None;
        }
    }
    let mut heredoc_end = closing.end_offset();
    while heredoc_end > closing.start_offset()
        && matches!(file.bytes[heredoc_end - 1], b'\n' | b'\r')
    {
        heredoc_end -= 1;
    }
    let call_source = std::str::from_utf8(file.bytes.get(heredoc_end..end)?)
        .ok()?
        .trim();
    let line = file.line_of(end);
    let line_start = file.line_start(line);
    let mut line_end = file.line_start(line + 1);
    while line_end > line_start && matches!(file.bytes[line_end - 1], b'\n' | b'\r') {
        line_end -= 1;
    }
    let line_source = std::str::from_utf8(&file.bytes[line_start..line_end])
        .ok()?
        .trim();
    let range = if call_source == line_source {
        heredoc_end..end
    } else if format!("{call_source},") == line_source {
        heredoc_end..end + 1
    } else {
        return None;
    };
    let start = outer.location().start_offset();
    let begin_line = file.line_of(start);
    let mut insert_at = file.line_start(begin_line + 1);
    while insert_at > file.line_start(begin_line)
        && matches!(file.bytes[insert_at - 1], b'\n' | b'\r')
    {
        insert_at -= 1;
    }
    let moved = range.clone();
    crate::cops::style::correction::combine(
        file,
        vec![
            crate::offense::Correction {
                range,
                replacement: String::new(),
            },
            crate::offense::Correction {
                range: insert_at..insert_at,
                replacement: std::str::from_utf8(&file.bytes[moved])
                    .ok()?
                    .trim()
                    .to_owned(),
            },
        ],
    )
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
