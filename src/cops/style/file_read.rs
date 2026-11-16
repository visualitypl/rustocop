//! `Style/FileRead` — direct whole-file reads.
use super::file_api::no_arguments;
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct FileRead;
impl Cop for FileRead {
    fn name(&self) -> &'static str {
        "Style/FileRead"
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
        let Some((filename, mode, read)) = evidence(&open, cx) else {
            return;
        };
        let method = if mode.ends_with('b') {
            "binread"
        } else {
            "read"
        };
        let start = read.location().start_offset();
        let (line, col) = file.line_col(start);
        // RuboCop replaces from `open` to the end of the read.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let path = std::str::from_utf8(filename.location().as_slice()).ok()?;
                let end = if read
                    .as_call_node()
                    .is_some_and(|c| c.name().as_slice() == b"open")
                {
                    read.location().end_offset()
                } else {
                    ast::source_end(&read)
                };
                Some(Correction {
                    range: open.message_loc()?.start_offset()..end,
                    replacement: format!("{method}({path})"),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `File.{method}`."),
            line,
            col,
            end_line: file.line_of(read.location().end_offset().saturating_sub(1).max(start)),
            correction,
        });
    }
}

fn evidence<'pr>(
    open: &CallNode<'pr>,
    cx: &NodeContext<'_, 'pr>,
) -> Option<(Node<'pr>, String, Node<'pr>)> {
    if open.name().as_slice() != b"open"
        || open.is_safe_navigation()
        || !open
            .receiver()
            .is_some_and(|r| ast::is_global_const(&r, &["File"]))
    {
        return None;
    }
    // Unlike call_arguments, literal blocks are not positional arguments.
    let mut args = open.arguments()?.arguments().iter();
    let filename = args.next()?;
    let mode = if let Some(mode) = args.next() {
        let mode = mode.as_string_node()?;
        if ![b"r".as_slice(), b"rt", b"rb", b"r+", b"r+t", b"r+b"].contains(&mode.unescaped()) {
            return None;
        }
        String::from_utf8_lossy(mode.unescaped()).into_owned()
    } else {
        "r".into()
    };
    if args.next().is_some() {
        return None;
    }
    if let Some(block) = open.block() {
        if let Some(pass) = block.as_block_argument_node() {
            if pass.expression()?.as_symbol_node()?.unescaped() != b"read" {
                return None;
            }
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
            let read = body.body().iter().next()?.as_call_node()?;
            if read.block().is_some()
                || !read_call(&read)
                || !read
                    .receiver()?
                    .as_local_variable_read_node()
                    .is_some_and(|r| r.name().as_slice() == param.name().as_slice())
            {
                return None;
            }
        }
        Some((filename, mode, open.as_node()))
    } else {
        let read = cx.parent()?.as_call_node()?;
        if !read_call(&read)
            || !read.receiver().is_some_and(|r| {
                r.location().start_offset() == open.location().start_offset()
                    && r.location().end_offset() == open.location().end_offset()
            })
        {
            return None;
        }
        Some((filename, mode, read.as_node()))
    }
}
fn read_call(call: &CallNode<'_>) -> bool {
    call.name().as_slice() == b"read" && !call.is_safe_navigation() && no_arguments(call)
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/file_read_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn corrects_as_rubocop() {
        assert_correction(
            &FileRead,
            "File.open(filename).read\n",
            "File.read(filename)\n",
        );
        assert_correction(
            &FileRead,
            "File.open(filename, 'rb') { |f| f.read }\n",
            "File.binread(filename)\n",
        );
    }
    #[test]
    fn trailing_comma_block_parameter_is_still_a_single_argument() {
        assert_correction(
            &FileRead,
            "File.open('file') { |f,| f.read }\n",
            "File.read('file')\n",
        );
    }
    #[test]
    fn rejects_nonmatching_receivers_arguments_modes_and_blocks() {
        for source in [
            "File.open(filename) { |f| something_else.read }",
            "File.open(filename) { |f| f.read(3) }",
            "File.open(filename).read(3)",
            "File.open(filename).read(nil)",
            "File.open(filename).readlines",
            "File.open(filename).read(&b)",
            "File.open(filename, encoding: 'UTF-8').read",
            "File.open(filename, 'r:UTF-8').read",
            "File.open(filename, 'w+').read",
            "File.open(filename, mode).read",
            "File.open(filename, 'r', 0600).read",
            "File.open(filename, &reader)",
            "Other::File.open(filename).read",
            "file.open(filename).read",
            "File&.open(filename).read",
            "File.open(filename)&.read",
            "File.open(filename) { |f, g| f.read }",
            "File.open(filename) { |f| f.read; extra }",
            "File.open(filename) { it.read }",
            "File.open(filename) { |f; other| f.read }",
            "File.open(filename) { |f| f.read {} }",
            "consume File.open('file') do |f| f.read end",
        ] {
            assert!(
                correction_offenses(&FileRead, source).is_empty(),
                "{source}"
            );
        }
    }
}
