//! `Style/SingleLineDoEndBlock` — expand keyword blocks without changing binding.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SingleLineDoEndBlock;
impl Cop for SingleLineDoEndBlock {
    fn name(&self) -> &'static str {
        "Style/SingleLineDoEndBlock"
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
        let (owner, open, close, parameters) = if let Some(b) = node.as_block_node() {
            let Some(owner) = cx.parent() else {
                return;
            };
            (owner, b.opening_loc(), b.closing_loc(), b.parameters())
        } else if let Some(l) = node.as_lambda_node() {
            (node, l.opening_loc(), l.closing_loc(), None)
        } else {
            return;
        };
        if open.as_slice() != b"do"
            || file.line_of(open.start_offset()) != file.line_of(close.start_offset())
        {
            return;
        }
        // Upstream offenses compare only line/column, not highlight length.
        // Receiver blocks can share their owner's start with an outer block.
        // Keep the outer highlight; its atomic fix defers the inner edit.
        if cx.ancestors().any(|ancestor| {
            if ancestor.location().start_offset() != owner.location().start_offset() {
                return false;
            }
            let block = if let Some(call) = ancestor.as_call_node() {
                call.block().and_then(|b| b.as_block_node())
            } else if let Some(call) = ancestor.as_super_node() {
                call.block().and_then(|b| b.as_block_node())
            } else if let Some(call) = ancestor.as_forwarding_super_node() {
                call.block()
            } else {
                None
            };
            block.is_some_and(|b| {
                b.opening_loc().start_offset() != open.start_offset()
                    && b.opening_loc().as_slice() == b"do"
                    && file.line_of(b.opening_loc().start_offset())
                        == file.line_of(b.closing_loc().start_offset())
            })
        }) {
            return;
        }
        let (line, col) = file.line_col(owner.location().start_offset());
        let block_line = file.line_of(open.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().next_back()?;
                if open.start_offset() < file.line_value_boundary()
                    || cx.ancestors().any(|n| {
                        matches!(
                            n,
                            Node::InterpolatedStringNode { .. }
                                | Node::InterpolatedXStringNode { .. }
                                | Node::InterpolatedRegularExpressionNode { .. }
                                | Node::InterpolatedSymbolNode { .. }
                        )
                    })
                {
                    return None;
                }
                // Preserve header/comment identity on every touched line.
                let line_range = file.line_start(block_line)..file.line_start(block_line + 1);
                let comments = file.comments();
                let first = comments.partition_point(|r| r.end <= line_range.start);
                if comments[first..]
                    .iter()
                    .take_while(|r| r.start < line_range.end)
                    .any(|r| !super::correction::comment_relocation_safe(&file.bytes[r.clone()]))
                {
                    return None;
                }
                let start = parameters
                    .and_then(|p| p.as_block_parameters_node())
                    .filter(|p| p.parameters().is_some() || !p.locals().is_empty())
                    .map_or(open.end_offset(), |p| p.location().end_offset());
                // Parameter-default heredocs open after `do` but before this
                // insertion. A newline here would absorb the block body into
                // their literal contents, even if the result still parses.
                if file.heredoc_pending_at(root, start) {
                    return None;
                }
                let newline = if file
                    .bytes
                    .get(file.line_start(block_line + 1).saturating_sub(2))
                    == Some(&b'\r')
                {
                    "\r\n"
                } else {
                    "\n"
                };
                let mut edits = vec![Correction {
                    range: start..start,
                    replacement: newline.into(),
                }];
                if let Some(&end) = file
                    .do_end_facts(root)
                    .heredoc_ends
                    .get(&(node.location().start_offset(), node.location().end_offset()))
                {
                    // Moving a closing keyword past a heredoc is safe only when it
                    // owns the rest of the opener line. Never move a chained send,
                    // sibling expression, parenthesis or enclosing closing token.
                    let suffix_end = file.line_start(block_line + 1).min(file.bytes.len());
                    if !file.bytes[close.end_offset()..suffix_end]
                        .iter()
                        .all(u8::is_ascii_whitespace)
                    {
                        return None;
                    }
                    edits.push(Correction {
                        range: close.start_offset()..close.end_offset(),
                        replacement: String::new(),
                    });
                    edits.push(Correction {
                        range: end..end,
                        replacement: format!("{newline}end"),
                    });
                } else {
                    edits.push(Correction {
                        range: close.start_offset()..close.start_offset(),
                        replacement: newline.into(),
                    });
                }
                super::correction::combine(file, edits)
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Prefer multiline `do`...`end` block.".into(),
            line,
            col,
            end_line: block_line,
            correction,
        });
    }
}

/// Bottom-up heredoc ownership from the main AST, built only for corrections.
/// Closing locations include the terminator newline; insert before that newline.
#[derive(Default)]
pub(crate) struct Facts {
    heredoc_ends: std::collections::HashMap<(usize, usize), usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Facts {
    pub(crate) fn build(root: &Node<'_>, file: &SourceFile) -> Self {
        struct Index<'a> {
            file: &'a SourceFile,
            facts: Facts,
            stack: Vec<((usize, usize), Option<usize>)>,
        }
        impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Index<'_> {
            fn enter(&mut self, node: Node<'pr>) {
                #[cfg(test)]
                {
                    self.facts.visits += 1;
                }
                let closing = if let Some(n) = node.as_string_node() {
                    n.opening_loc()
                        .filter(|l| l.as_slice().starts_with(b"<<"))
                        .and(n.closing_loc())
                } else if let Some(n) = node.as_interpolated_string_node() {
                    n.opening_loc()
                        .filter(|l| l.as_slice().starts_with(b"<<"))
                        .and(n.closing_loc())
                } else if let Some(n) = node.as_x_string_node() {
                    n.opening_loc()
                        .as_slice()
                        .starts_with(b"<<")
                        .then(|| n.closing_loc())
                } else if let Some(n) = node.as_interpolated_x_string_node() {
                    n.opening_loc()
                        .as_slice()
                        .starts_with(b"<<")
                        .then(|| n.closing_loc())
                } else {
                    None
                };
                let end = closing.map(|l| {
                    let mut end = l.end_offset();
                    while end > l.start_offset()
                        && matches!(self.file.bytes[end - 1], b'\n' | b'\r')
                    {
                        end -= 1;
                    }
                    end
                });
                self.stack.push((
                    (node.location().start_offset(), node.location().end_offset()),
                    end,
                ));
            }
            fn leave(&mut self) {
                let (key, end) = self.stack.pop().unwrap();
                if let Some(end) = end {
                    self.facts.heredoc_ends.insert(key, end);
                    if let Some((_, parent)) = self.stack.last_mut() {
                        *parent = Some(parent.unwrap_or(0).max(end));
                    }
                }
            }
        }
        let mut index = Index {
            file,
            facts: Self::default(),
            stack: Vec::new(),
        };
        crate::cops::walk::walk(&mut index, root);
        index.facts
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/single_line_do_end_block_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_correction_pairs() {
        for (before, after) in [
            ("foo do bar end\n", "foo do\n bar \nend\n"),
            ("foo do end\n", "foo do\n \nend\n"),
            (
                "foo do |arg| bar(arg) end\n",
                "foo do |arg|\n bar(arg) \nend\n",
            ),
            ("foo do bar(_1) end\n", "foo do\n bar(_1) \nend\n"),
            ("foo do bar(it) end\n", "foo do\n bar(it) \nend\n"),
            ("->(arg) do foo arg end\n", "->(arg) do\n foo arg \nend\n"),
            (
                "lambda do |arg| foo(arg) end\n",
                "lambda do |arg|\n foo(arg) \nend\n",
            ),
            (
                "foo do <<~EOS end\n  text\nEOS\n",
                "foo do\n <<~EOS \n  text\nEOS\nend\n",
            ),
            (
                "foo do bar(<<~EOS) end\n  text\nEOS\n",
                "foo do\n bar(<<~EOS) \n  text\nEOS\nend\n",
            ),
            (
                "foo do bar(<<~ONE, <<~TWO) end\n  one\nONE\n  two\nTWO\n",
                "foo do\n bar(<<~ONE, <<~TWO) \n  one\nONE\n  two\nTWO\nend\n",
            ),
            (
                "aaaaaaaaaaaa do bbbbbbbbbbbbb end\n",
                "aaaaaaaaaaaa do\n bbbbbbbbbbbbb \nend\n",
            ),
        ] {
            assert_correction(&SingleLineDoEndBlock, before, after);
        }
    }
    #[test]
    fn canonical_newlines_preserve_ast_binding_control_flow_and_parameter_shapes() {
        for block in [
            "foo do |x| return x end",
            "foo do |x| break x end",
            "foo do |x| next x end",
            "foo do |x,y; z| inner { x+y } end",
            "foo do || 1 end",
            "foo do |*x, **kw, &b| x end",
            "foo do x; rescue StandardError; fallback; ensure; done end",
            "foo do x rescue fallback end",
            "->(x,y) do x+y end",
            "lambda do |x,y| x+y end",
            "proc do |x,y| x+y end",
            "foo do _1 end",
            "foo do it end",
            "foo do |it| inner { it } end",
            "-> do _1 end",
            "-> do it end",
        ] {
            for template in [
                "%",
                "def f; %; end",
                "outer %",
                "outer(%)",
                "[%]",
                "object.x = %",
                "%.to_s",
                "-> { % }",
            ] {
                let source = template.replace('%', block);
                let before = ruby_prism::parse(source.as_bytes());
                if before.errors().next().is_some() {
                    continue;
                }
                let found =
                    crate::cops::testing::correction_offenses(&SingleLineDoEndBlock, &source);
                assert_eq!(found.len(), 1, "{source}");
                let after = crate::corrections::apply(source.as_bytes(), &found)
                    .unwrap()
                    .bytes;
                assert_ne!(source.as_bytes(), after, "{source}");
                let parsed = ruby_prism::parse(&after);
                assert_eq!(parsed.errors().count(), 0, "{source}");
                assert_eq!(
                    crate::cops::ast::fingerprint_ignoring_grouping(
                        &before.node(),
                        source.as_bytes()
                    ),
                    crate::cops::ast::fingerprint_ignoring_grouping(&parsed.node(), &after),
                    "{source} => {}",
                    String::from_utf8_lossy(&after)
                );
            }
        }
    }
    #[test]
    fn comments_bom_crlf_and_raw_data_are_byte_exact() {
        for (before, after) in [
            (
                "foo do |x| x end # comment\n",
                "foo do |x|\n x \nend # comment\n",
            ),
            (
                "\u{feff}# encoding: UTF-8\r\nfoo do |é| é end\r\n",
                "\u{feff}# encoding: UTF-8\r\nfoo do |é|\r\n é \r\nend\r\n",
            ),
            (
                "foo do bar end\n__END__\nraw do end\n",
                "foo do\n bar \nend\n__END__\nraw do end\n",
            ),
            (
                "foo do <<~TEXT end\r\n text\r\nTEXT\r\n",
                "foo do\r\n <<~TEXT \r\n text\r\nTEXT\r\nend\r\n",
            ),
        ] {
            assert_correction(&SingleLineDoEndBlock, before, after);
        }
    }
    #[test]
    fn single_line_means_delimiters_not_the_receiver_or_arguments() {
        for (source, line, col, end_line) in [
            ("foo\n .bar do x end", 1, 1, 2),
            ("outer(a do\n b\nend.c do _1 end)", 1, 7, 3),
            ("a.b(a do\n b\nend.c do\n d\nend) do z end", 1, 1, 5),
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == SingleLineDoEndBlock.name())
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line)
            );
        }
    }
    #[test]
    fn exact_positions_and_lookalikes() {
        for source in [
            "foo do bar end",
            "->(arg) do foo arg end",
            "lambda do |arg| foo(arg) end",
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == SingleLineDoEndBlock.name())
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!(found[0].message, "Prefer multiline `do`...`end` block.");
        }
        for source in [
            "foo do\n bar\nend",
            "foo { bar }",
            "foo do\nrescue CustomError\nend",
            "foo do\nensure\nend",
            "# foo do bar end\n'foo do end'",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == SingleLineDoEndBlock.name()),
                "{source}"
            );
        }
    }
}
