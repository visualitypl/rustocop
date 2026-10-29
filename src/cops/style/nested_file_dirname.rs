//! `Style/NestedFileDirname` — use Ruby 3.1's dirname level argument.
use super::file_api::{argument, replacement, send};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct NestedFileDirname;
impl Cop for NestedFileDirname {
    fn name(&self) -> &'static str {
        "Style/NestedFileDirname"
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
        let Some(call) = node.as_call_node().filter(dirname) else {
            return;
        };
        // Prism's ArgumentsNode is transparent in RuboCop's send ancestry.
        if cx
            .ancestors()
            .find(|n| n.as_arguments_node().is_none())
            .and_then(|n| n.as_call_node())
            .is_some_and(|p| dirname(&p))
        {
            return;
        }
        let Some(mut path) = ast::call_arguments(&call).into_iter().next() else {
            return;
        };
        let mut level = 1;
        let mut canonical = argument(&call).is_some() && call.block().is_none();
        while let Some(inner) = send(&path).filter(dirname) {
            let Some(next) = ast::call_arguments(&inner).into_iter().next() else {
                return;
            };
            canonical &= argument(&inner).is_some();
            level += 1;
            path = next;
        }
        if level < 2 {
            return;
        }
        canonical &= !matches!(
            path,
            Node::SplatNode { .. }
                | Node::KeywordHashNode { .. }
                | Node::BlockArgumentNode { .. }
                | Node::ForwardingArgumentsNode { .. }
        );
        canonical &= std::str::from_utf8(path.location().as_slice()).is_ok();
        let path_text = String::from_utf8_lossy(path.location().as_slice());
        let text = format!("dirname({path_text}, {level})");
        let needs_group = path.as_call_node().is_some_and(|c| {
            c.block().is_some()
                || (c.opening_loc().is_none() && !ast::call_arguments(&c).is_empty())
        });
        let correction_text = if needs_group {
            format!("dirname(({path_text}), {level})")
        } else {
            text.clone()
        };
        let start = call.message_loc().unwrap().start_offset();
        let (line, col) = file.line_col(start);
        // Retain the outer receiver's exact spelling, including ::File.
        let prefix = &file.bytes[node.location().start_offset()..start];
        let full = std::str::from_utf8(prefix)
            .ok()
            .map(|p| format!("{p}{correction_text}"));
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{text}` instead."),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction: (cx.corrections_requested() && canonical)
                .then(|| replacement(node, cx, file, full?, "FilePath"))
                .flatten(),
        });
    }
}
fn dirname(call: &CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call.name().as_slice() == b"dirname"
        && call
            .receiver()
            .is_some_and(|n| ast::is_global_const(&n, &["File"]))
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/nested_file_dirname_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    #[test]
    fn preserves_levels_arguments_blocks_bindings_and_source() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "File.dirname(File.dirname(path, 2))",
            "File.dirname(File.dirname(path), level())",
            "File.dirname(File.dirname(*paths))",
            "File.dirname(File.dirname(path), &block)",
            "File.dirname(File.dirname(path)) { effect }",
            "File.dirname( # retain\n File.dirname(path))",
            "File.dirname(\n File.dirname(path))\np __LINE__",
            "File.dirname(File.dirname(<<~PATH))\n  path\nPATH\n",
            "module M; File = Custom; File.dirname(File.dirname(path)); end",
            "File.define_singleton_method(:dirname) { |x| x }; File.dirname(File.dirname(path))",
        ] {
            let found = correction_offenses(&NestedFileDirname, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &NestedFileDirname,
            "::File.dirname(File.dirname(path))",
            "::File.dirname(path, 2)",
        );
        assert_correction(
            &NestedFileDirname,
            "File.dirname(File.dirname(make_path arg))",
            "File.dirname((make_path arg), 2)",
        );
        assert_correction(
            &NestedFileDirname,
            "\u{feff}# encoding: UTF-8\r\nFile.dirname(File.dirname(\"a/b/c\"))\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nFile.dirname(\"a/b/c\", 2)\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
    }

    #[test]
    fn pinned_diagnostics_and_corrections() {
        for (expression, level) in [
            ("File.dirname(path)", 2),
            ("File.dirname(File.dirname(path))", 3),
        ] {
            let source = format!("# header\n  File.dirname({expression})\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Style/NestedFileDirname")
                .collect();
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 8, 2));
            assert_eq!(
                found[0].message,
                format!("Use `dirname(path, {level})` instead.")
            );
            assert_correction(
                &NestedFileDirname,
                &source,
                &format!("# header\n  File.dirname(path, {level})\n"),
            );
        }
        for source in [
            "File.dirname(path)",
            "File.dirname(path, 2)",
            "Other.dirname(Other.dirname(path))",
            "Other::File.dirname(File.dirname(path))",
            "File&.dirname(File.dirname(path))",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Style/NestedFileDirname"),
                "{source}"
            );
        }
    }
}
