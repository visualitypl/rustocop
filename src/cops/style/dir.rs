//! `Style/Dir` — canonical current-file directory idioms (Ruby >= 2.0).
use super::file_api::{argument, replacement, send};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Dir;
impl Cop for Dir {
    fn name(&self) -> &'static str {
        "Style/Dir"
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
        let Some(call) = node.as_call_node().filter(|n| !n.is_safe_navigation()) else {
            return;
        };
        let inner_method: &[u8] = match call.name().as_slice() {
            b"expand_path" => b"dirname",
            b"dirname" => b"realpath",
            _ => return,
        };
        if !call
            .receiver()
            .is_some_and(|n| ast::is_global_const(&n, &["File"]))
        {
            return;
        }
        let Some(inner) = argument(&call).and_then(|n| send(&n)) else {
            return;
        };
        if inner.name().as_slice() != inner_method
            || !inner
                .receiver()
                .is_some_and(|n| ast::is_global_const(&n, &["File"]))
            || !argument(&inner).is_some_and(|n| n.as_source_file_node().is_some())
        {
            return;
        }
        let start = node.location().start_offset();
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `__dir__` to get an absolute path to the current file's directory."
                .into(),
            line,
            col,
            end_line: file.line_of(ast::source_end(node).saturating_sub(1).max(start)),
            correction: (cx.corrections_requested()
                && (call.name().as_slice() != b"expand_path" || !file.noncanonical_load_path()))
            .then(|| {
                // A bare method name can absorb a spaced index (or unary
                // operator) as an argument instead of remaining its receiver.
                let receiver = cx
                    .parent()
                    .and_then(|p| p.as_call_node())
                    .and_then(|p| p.receiver())
                    .is_some_and(|r| r.location().start_offset() == start);
                let text = if receiver { "(__dir__)" } else { "__dir__" };
                replacement(node, cx, file, text.into(), "File")
            })
            .flatten(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/dir_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    //!
    //! Every target names `__FILE__`, a listed source observer,
    //! so every edit is withheld and the finding stays
    //! (`assert_listed_hold`); the binding guards below still decide the
    //! cop's own emission.
    use super::*;
    use crate::cops::testing::correction_offenses;

    #[test]
    fn withholds_local_binding_constant_redefinition_and_source_sensitive_edits() {
        for source in [
            "__dir__ = :shadow; File.expand_path(File.dirname(__FILE__))",
            "def __dir__; :custom; end; File.expand_path(File.dirname(__FILE__))",
            "def f(__dir__); File.dirname(File.realpath(__FILE__)); end",
            "proc { |__dir__| File.expand_path(File.dirname(__FILE__)) }",
            "module M; File = Custom; File.expand_path(File.dirname(__FILE__)); end",
            "class File; end; File.dirname(File.realpath(__FILE__))",
            "File.expand_path(File.dirname(__FILE__)) { side_effect }",
            "File.expand_path( # comment\n File.dirname(__FILE__))",
            "File.\n expand_path(File.dirname(__FILE__))\np __LINE__",
            "File. # rubocop:disable Lint/Debugger\n expand_path(File.dirname(__FILE__))",
        ] {
            let found = correction_offenses(&Dir, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn explicit_method_binding_calls_withhold_edits_without_hiding_diagnostics() {
        for receiver in [
            "",
            "self.",
            "C.",
            "::C.",
            "object.",
            "object&.",
            "Object.new.",
            "[].",
            "'literal'.",
        ] {
            for name in [":__dir__", "'__dir__'", ":\"__dir__\"", "\"__di\\x72__\""] {
                for binding in [
                    format!("{receiver}alias_method({name}, :object_id)"),
                    format!("{receiver}define_method({name}) {{ 123 }}"),
                    format!("{receiver}define_method({name}, implementation)"),
                    format!("{receiver}define_singleton_method({name}) {{ 123 }}"),
                    format!("{receiver}define_singleton_method({name}, &implementation)"),
                ] {
                    for expression in [
                        "File.dirname(File.realpath(__FILE__))",
                        "File.expand_path(File.dirname(__FILE__))",
                    ] {
                        let source =
                            format!("class C\n {binding}\n def run\n  {expression}\n end\nend\n");
                        let found = correction_offenses(&Dir, &source);
                        assert_eq!(found.len(), 1, "{source}");
                        assert_eq!((found[0].line, found[0].col), (4, 3), "{source}");
                        assert_eq!(
                            found[0].message,
                            "Use `__dir__` to get an absolute path to the current file's directory."
                        );
                        assert!(found[0].correction.is_none(), "{source}");
                    }
                }
            }
        }
    }

    #[test]
    fn ignores_receiver_argument_and_keyword_lookalikes() {
        for source in [
            "Other::File.expand_path(File.dirname(__FILE__))",
            "File.expand_path(Other::File.dirname(__FILE__))",
            "file.expand_path(File.dirname(__FILE__))",
            "File.expand_path(File.dirname('__FILE__'))",
            "File.expand_path(File.dirname(path))",
            "File.expand_path(File.dirname(__FILE__), base)",
            "File.dirname(File.realpath(__FILE__, base))",
            "File.expand_path(File.dirname(__FILE__, 2))",
            "File.expand_path(File.dirname(__FILE__), &b)",
            "File.expand_path(File.dirname(__FILE__, &b))",
            "File.expand_path(File.dirname(__FILE__) { side_effect })",
            "File.expand_path(File&.dirname(__FILE__))",
            "File&.expand_path(File.dirname(__FILE__))",
            "File.dirname(__FILE__)",
            "__dir__",
            "File.expand_path(File.dirname(file: __FILE__))",
        ] {
            assert!(correction_offenses(&Dir, source).is_empty(), "{source}");
        }
    }
}
