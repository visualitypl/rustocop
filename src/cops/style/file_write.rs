//! `Style/FileWrite` — whole-file truncating writes.
use super::file_api::write_replacement;
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
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
        // Opening/truncation precedes content evaluation in the original. Only
        // literal strings and non-block local reads can safely move before it.
        // Update/text modes and descriptor-valued paths are not equivalent APIs.
        // Chained opens can retain buffered output until GC; the convenience
        // helper closes/flushes immediately. Only block forms preserve closure.
        let correction = (cx.corrections_requested() && !mode.contains('+') && mode != "wt")
            .then(|| {
                if path.as_string_node().is_none() && path.as_source_file_node().is_none() {
                    return None;
                }
                let movable = content.as_string_node().is_some()
                    || content
                        .as_local_variable_read_node()
                        .is_some_and(|r| parameter.as_deref() != Some(r.name().as_slice()));
                if parameter.is_none() || !movable {
                    return None;
                }
                let receiver = open.receiver()?;
                let receiver = std::str::from_utf8(receiver.location().as_slice()).ok()?;
                let path = std::str::from_utf8(path.location().as_slice()).ok()?;
                let content = std::str::from_utf8(content.location().as_slice()).ok()?;
                write_replacement(
                    &whole,
                    cx,
                    file,
                    format!("{receiver}.{method}({path}, {content})"),
                )
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
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_modes_receivers_and_block_forms() {
        for receiver in ["File", "::File"] {
            for mode in ["w", "wt", "wb", "w+", "w+t", "w+b"] {
                let method = if mode.ends_with('b') {
                    "binwrite"
                } else {
                    "write"
                };
                for suffix in [
                    ".write(content)",
                    " { |f| f.write(content) }",
                    " do |f|\n    f.write(content)\n  end",
                    " { |f,| f.write(content) }",
                ] {
                    let source = format!(
                        "content = 'hello'\n  {receiver}.open(filename, '{mode}'){suffix}\n"
                    );
                    let found: Vec<_> = lint_str(&source)
                        .into_iter()
                        .filter(|o| o.cop == "Style/FileWrite")
                        .collect();
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (2, 3));
                    assert_eq!(found[0].message, format!("Use `File.{method}`."));
                    assert_correction(&FileWrite, &source, &source); // unknown descriptors
                    let source = source.replace("filename", "'file'");
                    let expected =
                        if mode.contains('+') || mode == "wt" || suffix == ".write(content)" {
                            source.clone()
                        } else {
                            format!("content = 'hello'\n  {receiver}.{method}('file', content)\n")
                        };
                    assert_correction(&FileWrite, &source, &expected);
                }
            }
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
    #[test]
    fn canonical_literals_locals_and_headers() {
        for path in ["'file'", "%q(file)", "\"fi\\x6ce\"", "__FILE__"] {
            for content in ["'hello'", "\"a\\r\\nb\\0\\xff\"", "content"] {
                let before = format!(
                    "content = 'hello'\n::File.open({path}, 'wb') {{ |f| f.write({content}) }}\n"
                );
                if path == "__FILE__" {
                    // The listed `__FILE__` holds the edit.
                    crate::cops::testing::assert_listed_hold(&FileWrite, &before);
                    continue;
                }
                assert_correction(
                    &FileWrite,
                    &before,
                    &format!("content = 'hello'\n::File.binwrite({path}, {content})\n"),
                );
            }
        }
        assert_correction(
            &FileWrite,
            "\u{feff}# encoding: UTF-8\r\nx = File.open('file', \"w\\x62\") { |f| f.write('data') } # keep\r\np __LINE__\r\n__END__\r\nraw\0\r\n",
            "\u{feff}# encoding: UTF-8\r\nx = File.binwrite('file', 'data') # keep\r\np __LINE__\r\n__END__\r\nraw\0\r\n",
        );
    }
    #[test]
    fn withholds_evaluation_binding_resource_and_source_sensitive_edits() {
        for source in [
            "File.open(fd, 'w').write(content)",
            "File.open('file', 'w+').write(content)",
            "File.open('file', 'wt').write(content)",
            "File.open('file', 'w').write(build_content)",
            "File.open('file', 'w').write(\"#{File.read('file')}\")",
            "File.open('file', 'w') { |f| f.write(f) }",
            "File.open('file', 'w') { |f| f.write(f.path) }",
            "File.open('file', 'w') { |f| f.write(helper { break 'x' }) }",
            "File.open('file', 'w').write('x')",
            "File.open('file', 'w').write('x') { side_effect }",
            "File.open('file', 'w', &:write).write('x')",
            "File.open( # keep\n 'file', 'w').write('x')",
            "File.open('file', 'w') { |f| # keep\n f.write('x') }",
            "File.open('file', 'w') do |f|\n f.write('x')\nend\np __LINE__\n",
        ] {
            let found = correction_offenses(&FileWrite, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn pinned_heredoc_variants_keep_findings_but_withhold_literal_movement() {
        for content in [
            "<<~EOS",
            "<<~EOS.gsub(/^/, '')",
            "'prefix' + <<~EOS",
            "process(<<~EOS)",
        ] {
            let source = format!(
                "File.open(filename, 'w') do |f|\n  f.write({content})\n    content\n  EOS\nend\n"
            );
            let found = correction_offenses(&FileWrite, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
        for source in [
            "File.open('file', 'w') { |f| f.write(<<~EOS) }\n  content\nEOS\n",
            "File.open('file', 'w') do |f|\n f.write(<<~HEAD + <<~TAIL)\n head\nHEAD\n tail\nTAIL\nend\n",
            "File.open(<<~PATH.strip, 'w') do |f|\n path/to/file\nPATH\n f.write(content)\nend\n",
        ] {
            let found = correction_offenses(&FileWrite, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }
}
