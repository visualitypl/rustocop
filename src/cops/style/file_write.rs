//! `Style/FileWrite` — whole-file truncating writes.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct FileWrite;
impl Cop for FileWrite {
    fn name(&self) -> &'static str {
        "Style/FileWrite"
    }
    fn safe_fixes(&self) -> bool {
        true
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
        let Some(open) = node.as_call_node() else {
            return;
        };
        let Some((path, mode, content, whole, parameter)) = evidence(&open, cx) else {
            return;
        };
        let method = if mode.ends_with('b') {
            "binwrite"
        } else {
            "write"
        };
        // RuboCop replaces from `open` to the end of the write, and restores
        // the bodies of heredocs that the replaced range held.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let end = whole.location().end_offset();
                let source = |n: &Node<'_>| {
                    std::str::from_utf8(n.location().as_slice())
                        .ok()
                        .map(str::to_owned)
                };
                let mut text = format!("{method}({}, {})", source(&path)?, source(&content)?);
                let mut heredocs = Vec::new();
                for argument in [&path, &content] {
                    heredoc_bodies(argument, &mut heredocs);
                }
                heredocs.retain(|body| body.end <= end);
                heredocs.sort_by_key(|body| body.start);
                for body in heredocs {
                    text.push('\n');
                    text.push_str(std::str::from_utf8(&file.bytes[body]).ok()?);
                }
                Some(Correction {
                    range: open.message_loc()?.start_offset()..end,
                    replacement: text,
                })
            })
            .flatten();
        let start = whole.location().start_offset();
        // Prism includes an attached block in a call's location. RuboCop
        // highlights that block only for the File.open block form, not for
        // a chained write send.
        let end = if parameter.is_some() {
            whole.location().end_offset()
        } else {
            ast::source_end(&whole)
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `File.{method}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction,
        });
    }
}

type Evidence<'pr> = (Node<'pr>, String, Node<'pr>, Node<'pr>, Option<Vec<u8>>);
fn evidence<'pr>(open: &CallNode<'pr>, cx: &NodeContext<'_, 'pr>) -> Option<Evidence<'pr>> {
    if open.name().as_slice() != b"open"
        || open.is_safe_navigation()
        || !open
            .receiver()
            .is_some_and(|r| ast::is_global_const(&r, &["File"]))
    {
        return None;
    }
    let mut args = open.arguments()?.arguments().iter();
    let path = args.next()?;
    let mode = args.next()?.as_string_node()?;
    if args.next().is_some()
        || ![b"w".as_slice(), b"wt", b"wb", b"w+", b"w+t", b"w+b"].contains(&mode.unescaped())
    {
        return None;
    }
    let mode = String::from_utf8_lossy(mode.unescaped()).into_owned();
    if let Some(block) = open.block() {
        if let Some(pass) = block.as_block_argument_node() {
            if pass.expression()?.as_symbol_node()?.unescaped() != b"write" {
                return None;
            }
            // A block-pass open can still be the receiver of an outer write.
        } else {
            let block = block.as_block_node()?;
            let params = block.parameters()?.as_block_parameters_node()?;
            if !params.locals().is_empty() {
                return None;
            }
            let params = params.parameters()?;
            if params.requireds().len() != 1
                || !params.optionals().is_empty()
                || params
                    .rest()
                    .is_some_and(|r| r.as_implicit_rest_node().is_none())
                || !params.posts().is_empty()
                || !params.keywords().is_empty()
                || params.keyword_rest().is_some()
                || params.block().is_some()
            {
                return None;
            }
            let param = params
                .requireds()
                .iter()
                .next()?
                .as_required_parameter_node()?;
            let body = block.body()?.as_statements_node()?;
            if body.body().len() != 1 {
                return None;
            }
            let write = body.body().iter().next()?.as_call_node()?;
            if write.block().is_some()
                || !write
                    .receiver()?
                    .as_local_variable_read_node()
                    .is_some_and(|r| r.name().as_slice() == param.name().as_slice())
            {
                return None;
            }
            let content = write_content(&write)?;
            return Some((
                path,
                mode,
                content,
                open.as_node(),
                Some(param.name().as_slice().to_vec()),
            ));
        }
    }
    let write = cx.parent()?.as_call_node()?;
    if !write.receiver().is_some_and(|r| {
        r.location().start_offset() == open.location().start_offset()
            && r.location().end_offset() == open.location().end_offset()
    }) {
        return None;
    }
    Some((path, mode, write_content(&write)?, write.as_node(), None))
}
/// The bodies of the heredocs in `node`, from the first body line to the
/// end of the terminator (without its newline), as RuboCop's `heredoc_range`.
pub(super) fn heredoc_bodies(node: &Node<'_>, out: &mut Vec<std::ops::Range<usize>>) {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Bodies<'a>(&'a mut Vec<std::ops::Range<usize>>);
    impl<'pr> NodeVisitor<'pr> for Bodies<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            let (opening, start, closing) = if let Some(n) = node.as_string_node() {
                (
                    n.opening_loc(),
                    Some(n.content_loc().start_offset()),
                    n.closing_loc(),
                )
            } else if let Some(n) = node.as_interpolated_string_node() {
                let start = n.parts().iter().next().map(|p| p.location().start_offset());
                (n.opening_loc(), start, n.closing_loc())
            } else if let Some(n) = node.as_x_string_node() {
                (
                    Some(n.opening_loc()),
                    Some(n.content_loc().start_offset()),
                    Some(n.closing_loc()),
                )
            } else if let Some(n) = node.as_interpolated_x_string_node() {
                let start = n.parts().iter().next().map(|p| p.location().start_offset());
                (Some(n.opening_loc()), start, Some(n.closing_loc()))
            } else {
                return;
            };
            let (Some(opening), Some(closing)) = (opening, closing) else {
                return;
            };
            if !opening.as_slice().starts_with(b"<<") {
                return;
            }
            let terminator = closing.as_slice();
            let trimmed = terminator
                .strip_suffix(b"\n")
                .map(|t| t.strip_suffix(b"\r").unwrap_or(t))
                .unwrap_or(terminator);
            let end = closing.start_offset() + trimmed.len();
            self.0.push(start.unwrap_or(closing.start_offset())..end);
        }
    }
    walk(&mut Bodies(out), node);
}
fn write_content<'pr>(write: &CallNode<'pr>) -> Option<Node<'pr>> {
    if write.name().as_slice() != b"write" || write.is_safe_navigation() {
        return None;
    }
    let mut args = ast::call_arguments(write);
    if args.len() != 1 || args[0].as_splat_node().is_some() {
        return None;
    }
    Some(args.remove(0))
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/file_write_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    #[test]
    fn corrects_as_rubocop() {
        for (before, after) in [
            (
                "File.open(filename, 'w').write(content)\n",
                "File.write(filename, content)\n",
            ),
            (
                "File.open(filename, 'wb') { |f| f.write(content) }\n",
                "File.binwrite(filename, content)\n",
            ),
        ] {
            crate::cops::testing::assert_correction(&super::FileWrite, before, after);
        }
    }
    #[test]
    fn highlights_distinguish_chained_write_from_open_block() {
        for prefix in ["rubocop", "rustocop"] {
            for write in [
                "File.open('x', 'w').write('x')",
                "File.open('x', 'w').write(\n  'x')",
            ] {
                let end_line = write.lines().count() as u32;
                for directive in ["", "side_effect", "end"] {
                    let comment = format!(" # {prefix}:disable Style/FileWrite");
                    let source = format!(
                        "{write} do\n  side_effect{}\nend{}\n",
                        if directive == "side_effect" {
                            &comment
                        } else {
                            ""
                        },
                        if directive == "end" { &comment } else { "" },
                    );
                    let found: Vec<_> = lint_str(&source)
                        .into_iter()
                        .filter(|o| o.cop == "Style/FileWrite")
                        .collect();
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (1, 1));
                    assert_eq!(found[0].end_line, end_line, "{source}");
                    assert_eq!(found[0].message, "Use `File.write`.");
                }
                let source =
                    format!("{write} do # {prefix}:disable Style/FileWrite\n  side_effect\nend\n");
                assert!(!lint_str(&source).iter().any(|o| o.cop == "Style/FileWrite"));
            }
            let source = "File.open('x', 'w') do |f|\n  f.write('x')\nend\n";
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/FileWrite")
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].end_line, 3);
            for line in ["f.write('x')", "end"] {
                let source =
                    source.replace(line, &format!("{line} # {prefix}:disable Style/FileWrite"));
                assert!(!lint_str(&source).iter().any(|o| o.cop == "Style/FileWrite"));
            }
        }
    }
    #[test]
    fn lookalikes_do_not_match() {
        for source in [
            "File.open(filename, 'w') { |f| something.write(content) }",
            "File.open(filename, 'w') { |f| f.write(*objects) }",
            "File.open(filename, 'w').write(*objects)",
            "File.open(filename, 'w').write(a, b)",
            "File.open(filename, 'w').write",
            "File.open(filename, 'a').write(content)",
            "File.open(filename, mode).write(content)",
            "File.open(filename, mode: 'w').write(content)",
            "File.open(filename, 'w', 0600).write(content)",
            "Other::File.open(filename, 'w').write(content)",
            "file.open(filename, 'w').write(content)",
            "File&.open(filename, 'w').write(content)",
            "File.open(filename, 'w')&.write(content)",
            "File.open(filename, 'w') { it.write(content) }",
            "File.open(filename, 'w') { |f, g| f.write(content) }",
            "File.open(filename, 'w') { |f; g| f.write(content) }",
            "File.open(filename, 'w') { |f| f.write(content); extra }",
            "File.open(filename, 'w', &writer)",
        ] {
            assert!(
                correction_offenses(&FileWrite, source).is_empty(),
                "{source}"
            );
        }
    }
}
