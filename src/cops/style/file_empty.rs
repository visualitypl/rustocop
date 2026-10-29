//! `Style/FileEmpty` — unsafe file emptiness idioms, detection-only (Ruby >= 2.4).
use super::file_api::{argument, integer, no_arguments, send};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct FileEmpty;
impl Cop for FileEmpty {
    fn name(&self) -> &'static str {
        "Style/FileEmpty"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((constant, arg)) = offensive(node) else {
            return;
        };
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `{}.empty?({})` instead.",
                String::from_utf8_lossy(constant.location().as_slice()),
                String::from_utf8_lossy(arg.location().as_slice())
            ),
            line,
            col,
            end_line: file.line_of(
                ast::source_end(node)
                    .saturating_sub(1)
                    .max(node.location().start_offset()),
            ),
            correction: None,
        });
    }
}

fn offensive<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>)> {
    // RuboCop visits the send below an enclosing literal block. Keep that
    // detection (and its send-only range); this unsafe cop never fixes it.
    let call = node.as_call_node().filter(|n| !n.is_safe_navigation())?;
    let name = call.name().as_slice();
    if name == b"zero?"
        && let Some(constant) = call
            .receiver()
            .filter(|n| ast::is_global_const(n, &["File", "FileTest"]))
    {
        return Some((constant, argument(&call)?));
    }
    let receiver = call.receiver()?;
    let method: &[&[u8]] = match name {
        b"==" | b">=" | b"!=" => {
            let value = argument(&call)?;
            if matches!(name, b"==" | b">=") && integer(&value, 0) {
                &[b"size"]
            } else if matches!(name, b"==" | b"!=")
                && value
                    .as_string_node()
                    .is_some_and(|n| n.unescaped().is_empty())
            {
                &[b"read", b"binread"]
            } else {
                return None;
            }
        }
        b"zero?" if no_arguments(&call) => &[b"size"],
        b"empty?" if no_arguments(&call) => &[b"read", b"binread"],
        _ => return None,
    };
    let mut inner = send(&receiver)?;
    if matches!(name, b"==" | b">=" | b"!=")
        && inner.name().as_slice() == b"!"
        && no_arguments(&inner)
    {
        inner = send(&inner.receiver()?)?;
    }
    if !method.contains(&inner.name().as_slice()) {
        return None;
    }
    let constant = inner.receiver()?;
    if !ast::is_global_const(&constant, &["File", "FileTest"]) {
        return None;
    }
    Some((constant, argument(&inner)?))
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/file_empty_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};

    #[test]
    fn pinned_diagnostics_are_always_detection_only() {
        for constant in ["File", "::File", "FileTest", "::FileTest"] {
            for tail in [
                "zero?(path)",
                "size(path) == 0",
                "size(path) >= 0",
                "size(path).zero?",
                "read(path).empty?",
                "binread(path).empty?",
                "read(path) == ''",
                "binread(path) == ''",
                "read(path) != ''",
                "binread(path) != ''",
            ] {
                let source = format!("# header\n  {constant}.{tail}\n");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/FileEmpty")
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    format!("Use `{constant}.empty?(path)` instead.")
                );
                assert!(
                    correction_offenses(&FileEmpty, &source)[0]
                        .correction
                        .is_none()
                );
            }
        }
        for (source, col) in [
            ("!File.zero?(path)", 2),
            ("!File.size(path) == 0", 1),
            ("!File.size(path) >= 0", 1),
            ("!File.read(path) == ''", 1),
            ("!File.binread(path) != ''", 1),
            ("File.\n  zero?(path)", 1),
        ] {
            let found = correction_offenses(&FileEmpty, source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col));
            assert_eq!(found[0].message, "Use `File.empty?(path)` instead.");
            assert!(found[0].correction.is_none());
        }
        assert!(!FileEmpty.safe_fixes());
    }

    #[test]
    fn arbitrary_single_ast_arguments_and_negative_zero_match_upstream() {
        for source in [
            "File.size(&b) == 0",
            "File.size(path) == -0",
            "File.read(*paths).empty?",
            "File.zero?(encoding: enc)",
        ] {
            let found = correction_offenses(&FileEmpty, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn ignores_nonmatching_receivers_arguments_values_and_blocks() {
        for source in [
            "File.empty?(path)",
            "File.exist?(path)",
            "Other::File.zero?(path)",
            "file.zero?(path)",
            "Other::FileTest.zero?(path)",
            "File.size(path) > 0",
            "File.size(path) != 0",
            "File.size(path) == 1",
            "File.read(path) == 'x'",
            "File.read(path) == \"#{value}\"",
            "File.size(path, extra) == 0",
            "File.read(path, 0).empty?",
            "File.binread(path, 0).empty?",
            "File.zero?",
            "File.zero?(path, &b)",
            "File.size(path, &b).zero?",
            "File.read(path).empty?(&b)",
            "File.read(path) { work }.empty?",
            "File&.zero?(path)",
            "File.size(path)&.zero?",
            "File&.read(path).empty?",
            "File.size(path).zero?(arg)",
            "File.read(path).size == 0",
        ] {
            assert!(
                correction_offenses(&FileEmpty, source).is_empty(),
                "{source}"
            );
        }
    }
}
