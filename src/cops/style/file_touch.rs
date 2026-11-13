//! `Style/FileTouch` — append-mode opens with empty blocks (detection only).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct FileTouch;
impl Cop for FileTouch {
    fn name(&self) -> &'static str {
        "Style/FileTouch"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"open"
            || call.is_safe_navigation()
            || !call
                .receiver()
                .is_some_and(|r| ast::is_global_const(&r, &["File"]))
        {
            return;
        }
        let args = ast::call_arguments(&call);
        let [path, mode] = args.as_slice() else {
            return;
        };
        if !mode.as_string_node().is_some_and(|s| {
            [b"a".as_slice(), b"a+", b"ab", b"a+b", b"at", b"a+t"].contains(&s.unescaped())
        }) {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if block.body().is_some() {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `FileUtils.touch({})` instead of `File.open` in append mode with empty block.",
                String::from_utf8_lossy(path.location().as_slice())
            ),
            line,
            col,
            end_line: file.line_of(node.location().end_offset().saturating_sub(1)),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/file_touch_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    #[test]
    fn append_modes_empty_blocks_and_paths_are_detection_only() {
        for receiver in ["File", "::File"] {
            for mode in ["a", "a+", "ab", "a+b", "at", "a+t", "a\\x62"] {
                for path in [
                    "filename",
                    "'file'",
                    "%q(file)",
                    "\"fi\\x6ce\"",
                    "__FILE__",
                    "\"#{path}\"",
                ] {
                    for block in ["{}", "do\n  end", "{ |f| }", "{ # keep\n }"] {
                        let source =
                            format!("# header\n  {receiver}.open({path}, \"{mode}\") {block}\n");
                        let found: Vec<_> = lint_str(&source)
                            .into_iter()
                            .filter(|o| o.cop == "Style/FileTouch")
                            .collect();
                        assert_eq!(found.len(), 1, "{source}");
                        assert_eq!((found[0].line, found[0].col), (2, 3));
                        assert_eq!(
                            found[0].message,
                            format!(
                                "Use `FileUtils.touch({path})` instead of `File.open` in append mode with empty block."
                            )
                        );
                        assert!(
                            correction_offenses(&FileTouch, &source)[0]
                                .correction
                                .is_none()
                        );
                    }
                }
            }
        }
        assert!(!FileTouch.safe_fixes());
    }
    #[test]
    fn ignores_nonempty_blocks_modes_receivers_and_argument_lookalikes() {
        for source in [
            "File.open(filename, 'a')",
            "File.open(filename, 'w') {}",
            "File.open(filename) {}",
            "File.open(filename, 'a') { nil }",
            "File.open(filename, 'a') { f.write('x') }",
            "Other::File.open(filename, 'a') {}",
            "file.open(filename, 'a') {}",
            "File&.open(filename, 'a') {}",
            "File.open(filename, 'a', 0600) {}",
            "File.open(filename, mode) {}",
            "File.open(filename, 'ab+') {}",
            "File.open(filename, mode: 'a') {}",
            "File.open(filename, 'a', &block)",
        ] {
            assert!(
                correction_offenses(&FileTouch, source).is_empty(),
                "{source}"
            );
        }
    }
}
