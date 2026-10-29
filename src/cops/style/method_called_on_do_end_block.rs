//! Detect easily missed calls chained onto keyword-delimited blocks.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct MethodCalledOnDoEndBlock;
impl Cop for MethodCalledOnDoEndBlock {
    fn name(&self) -> &'static str {
        "Style/MethodCalledOnDoEndBlock"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Prism embeds synthetic assignment sends in write/target nodes.
        // Their send range stops at the selector or `]`, before the RHS.
        macro_rules! send {
            ($access:ident, $end:ident) => {{
                let call = node.$access().unwrap();
                (call.receiver(), call.$end()?.end_offset())
            }};
        }
        macro_rules! index {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                (call.receiver(), call.closing_loc().end_offset())
            }};
        }
        let parts = || {
            Some(match node {
                Node::CallNode { .. } => {
                    let call = node.as_call_node().unwrap();
                    // A chained literal block is conspicuous, and belongs to
                    // MultilineBlockChain upstream. A block *argument* is not one.
                    if call
                        .block()
                        .is_some_and(|block| block.as_block_node().is_some())
                    {
                        return None;
                    }
                    (call.receiver(), node.location().end_offset())
                }
                Node::CallAndWriteNode { .. } => send!(as_call_and_write_node, message_loc),
                Node::CallOrWriteNode { .. } => send!(as_call_or_write_node, message_loc),
                Node::CallOperatorWriteNode { .. } => {
                    send!(as_call_operator_write_node, message_loc)
                }
                Node::IndexAndWriteNode { .. } => index!(as_index_and_write_node),
                Node::IndexOrWriteNode { .. } => index!(as_index_or_write_node),
                Node::IndexOperatorWriteNode { .. } => index!(as_index_operator_write_node),
                Node::CallTargetNode { .. } => {
                    let call = node.as_call_target_node().unwrap();
                    (Some(call.receiver()), call.message_loc().end_offset())
                }
                Node::IndexTargetNode { .. } => {
                    let call = node.as_index_target_node().unwrap();
                    (Some(call.receiver()), call.closing_loc().end_offset())
                }
                _ => return None,
            })
        };
        let Some((Some(receiver), end)) = parts() else {
            return;
        };
        let Some(start) = keyword_block_end(&receiver) else {
            return;
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid chaining a method call on a do...end block.".into(),
            line,
            col,
            end_line: file.line_of(end - 1),
            correction: None,
        });
    }
}

// Inspect only the direct receiver: parentheses/brace boundaries must not
// disappear, nor may an argument's or earlier receiver's block own this call.
// Constant work per visited node, using the pipeline's single main parse.
fn keyword_block_end(receiver: &Node<'_>) -> Option<usize> {
    if let Some(lambda) = receiver.as_lambda_node() {
        return (lambda.opening_loc().as_slice() == b"do")
            .then(|| lambda.closing_loc().start_offset());
    }
    let block = match receiver {
        Node::CallNode { .. } => receiver.as_call_node()?.block(),
        Node::SuperNode { .. } => receiver.as_super_node()?.block(),
        Node::ForwardingSuperNode { .. } => receiver
            .as_forwarding_super_node()?
            .block()
            .map(|b| b.as_node()),
        _ => None,
    }?;
    let block = block.as_block_node()?;
    (block.opening_loc().as_slice() == b"do").then(|| block.closing_loc().start_offset())
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/method_called_on_do_end_block_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str_with;

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        // Explicit selection opts in without changing the example's positions.
        lint_str_with(&[&MethodCalledOnDoEndBlock], source)
            .into_iter()
            .filter(|o| o.cop == "Style/MethodCalledOnDoEndBlock")
            .collect()
    }

    #[test]
    fn pinned_chained_calls_report_from_end_keyword() {
        for (source, line, col) in [
            ("a do\n  b\nend.c", 3, 1),
            ("a do\n  b\nend&.c", 3, 1),
            ("a do b end.c", 1, 8),
            ("a do\n  _1\nend.c", 3, 1),
            ("a do\n  it\nend.c", 3, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(
                found[0].message,
                "Avoid chaining a method call on a do...end block."
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn lambda_and_synthetic_assignment_receivers_match_keyword_blocks() {
        for (source, line, col, end_line) in [
            ("-> do 1 end.call", 1, 9, 1),
            ("-> do _1 end&.call", 1, 10, 1),
            ("-> do it end.call", 1, 10, 1),
            ("lambda do 1 end.call", 1, 13, 1),
            ("obj&.a do |x| x end.c", 1, 17, 1),
            ("a do b end.c =\n  1", 1, 8, 2),
            ("a do b end.c ||=\n  1", 1, 8, 1),
            ("a do b end.c &&=\n  1", 1, 8, 1),
            ("a do b end.c +=\n  1", 1, 8, 1),
            ("a do b end[\n  1\n] += 2", 1, 8, 3),
            ("a do b end[1] ||=\n  2", 1, 8, 1),
            ("a do b end[1] &&=\n  2", 1, 8, 1),
            ("a do b end.c, x = 1, 2", 1, 8, 1),
            ("a do b end[1], x = 1, 2", 1, 8, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line),
                "{source}"
            );
        }
    }

    #[test]
    fn command_operator_safe_navigation_arguments_and_comments_keep_block_ownership() {
        for (source, line, col, end_line) in [
            ("outer inner do b end.c", 1, 18, 1),
            ("outer(inner do b end.c)", 1, 18, 1),
            ("a do b end.c.d", 1, 8, 1),
            ("a do b end.c do d end.e", 1, 19, 1),
            ("a do b end.c { d }.e do f end.g", 1, 27, 1),
            ("a do b end.c(&blk)", 1, 8, 1),
            ("a do b end&.c(\n  1, *args, **kw, &blk\n)", 1, 8, 3),
            ("a do b end.c 1, key: 2", 1, 8, 1),
            ("a do b end[1] = 2", 1, 8, 1),
            ("a do b end.+(1)", 1, 8, 1),
            ("a do b end + 1", 1, 8, 1),
            ("!a do b end", 1, 9, 1),
            ("a do b end\n  # continuation\n  .c(\n    1\n  )", 1, 8, 5),
            ("a do\n  b(<<~TXT)\n    content\n  TXT\nend.c", 5, 1, 5),
            ("a do b end.c(<<~TXT)\ncontent\nTXT\n", 1, 8, 1),
            ("a do b end.c <<~TXT\ncontent\nTXT\n", 1, 8, 1),
            ("def f; super do 1 end.c; end", 1, 19, 1),
            ("def f; super() do 1 end.c; end", 1, 21, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line),
                "{source}"
            );
            assert_eq!(
                found[0].message,
                "Avoid chaining a method call on a do...end block."
            );
        }
    }

    #[test]
    fn parentheses_braces_arguments_and_chained_literal_blocks_are_boundaries() {
        for source in [
            "(a do b end).c",
            "((a do b end)).c",
            "(a do b end; other).c",
            "(-> do b end).call",
            "-> { b }.call",
            "lambda { b }.call",
            "outer(a do b end).c",
            "outer a do b end",
            "outer a { b }.c",
            "a do b end.c do |x| x end",
            "a do b end&.c { d }",
            "a do b end.c do _1 end",
            "a do b end.c do it end",
            "a do b end.c { d }.e",
            "a { _1 }.c",
            "a { it }.c",
            "def f; super { 1 }.c; end",
            "def f; super() { 1 }.c; end",
            "value = a do b end; value.c",
            "begin; a do b end; end.c",
            "# a do b end.c\n\"a do b end.c\"",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let found = offenses("a do b end.c; a do d end.e");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 8), (1, 22)]
        );
    }

    #[test]
    fn noncorrectable_even_when_corrections_are_requested() {
        assert!(!MethodCalledOnDoEndBlock.default_enabled());
        assert!(!MethodCalledOnDoEndBlock.safe_fixes());
        for source in ["a do b end.c", "-> do 1 end.call", "a do b end[1] += 2"] {
            let found =
                crate::cops::testing::correction_offenses(&MethodCalledOnDoEndBlock, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn wide_source_uses_only_direct_receiver_edges() {
        let source = "a do b end.c\n".repeat(4000);
        assert_eq!(offenses(&source).len(), 4000);
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for source in [
            "a do\n  b\nend",
            "a do\n  b\nend.c do\n  d\nend",
            "a do b end.c do d end",
            "a {\n  b\n}.c",
            "a { b }.c",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
