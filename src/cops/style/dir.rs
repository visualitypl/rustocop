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
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    #[test]
    fn pinned_diagnostics_and_corrections() {
        for outer in ["File", "::File"] {
            for inner in ["File", "::File"] {
                for (a, b) in [("expand_path", "dirname"), ("dirname", "realpath")] {
                    let source = format!("# header\n  {outer}.{a}({inner}.{b}(__FILE__))\n");
                    let found: Vec<_> = lint_str(&source)
                        .into_iter()
                        .filter(|o| o.cop == "Style/Dir")
                        .collect();
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert_eq!(
                        found[0].message,
                        "Use `__dir__` to get an absolute path to the current file's directory."
                    );
                    assert_correction(&Dir, &source, "# header\n  __dir__\n");
                }
            }
        }
    }

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
    fn groups_replacement_when_used_as_a_receiver() {
        for suffix in [" [0]", "[0]", " [0] = value", " +\"suffix\"", ".bytes"] {
            assert_correction(
                &Dir,
                &format!("File.dirname(File.realpath(__FILE__)){suffix}\n"),
                &format!("(__dir__){suffix}\n"),
            );
        }
    }

    #[test]
    fn explicit_method_aliases_withhold_corrections() {
        for name in [
            "__dir__ object_id",
            ":__dir__ :object_id",
            ":\"__dir__\" :object_id",
            ":\"#{name}\" :object_id",
        ] {
            let source = format!(
                "class C; alias {name}; def run; File.dirname(File.realpath(__FILE__)); end; end"
            );
            let found = correction_offenses(&Dir, &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
        assert_correction(
            &Dir,
            "class C; alias other object_id; def run; File.dirname(File.realpath(__FILE__)); end; end",
            "class C; alias other object_id; def run; __dir__; end; end",
        );
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
    fn unrelated_method_binding_names_keep_canonical_edits() {
        for binding in [
            "alias_method :other, :__dir__",
            "self.alias_method '__dir__=', :object_id",
            "C.define_method(:other) { 123 }",
            "define_method('__dir__=') { |value| value }",
            "object.define_singleton_method(:other) { 123 }",
            "define_singleton_method(:__dir__extra) { 123 }",
            "not_define_method(:__dir__) { 123 }",
            "'define_method(:__dir__) { 123 }'",
            "# alias_method :__dir__, :object_id",
        ] {
            let source = format!("{binding}\nFile.dirname(File.realpath(__FILE__))\n");
            assert_correction(&Dir, &source, &format!("{binding}\n__dir__\n"));
        }
    }

    #[test]
    fn equivalent_literal_binding_and_lookup_calls_withhold_only_dir_edits() {
        for binding in [
            "attr_reader :other, :__dir__",
            "self.attr_accessor 'other', '__dir__'",
            "attr :other, :__dir__",
            "attr :__dir__, true",
            "C.send(:alias_method, :__dir__, :object_id)",
            "self.__send__('define_method', '__dir__') { 123 }",
            "object.public_send(:define_singleton_method, :__dir__) { 123 }",
            "C.send(\"define_\\x6dethod\", :\"__di\\x72__\") { 123 }",
            "undef_method :__dir__",
            "undef :other, :__dir__",
            // A nonliteral name on a recognized binder is conservatively
            // possible evidence; no attempt is made to evaluate its value.
            "alias_method name, :object_id",
            "define_method(name) { 123 }",
            "define_singleton_method(\"#{name}\") { 123 }",
            "attr_reader name",
        ] {
            let source = format!(
                "class C; {binding}; def run; File.dirname(File.realpath(__FILE__)); end; end\nDir.children('.').empty?\n"
            );
            let found = correction_offenses(&Dir, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
            let dir_empty = correction_offenses(&super::super::DirEmpty, &source);
            assert_eq!(dir_empty.len(), 1, "{source}");
            assert!(dir_empty[0].correction.is_some(), "{source}");
        }
    }

    #[test]
    fn equivalent_binding_unrelated_names_do_not_block_canonical_fixes() {
        for binding in [
            "attr_reader :other",
            "attr_accessor '__dir__other'",
            "attr :other, true",
            "attr :other, false",
            // Writers never introduce a method named __dir__.
            "attr_writer :__dir__",
            "send(:alias_method, :other, :__dir__)",
            "__send__('define_method', :other) { 123 }",
            "public_send(:define_singleton_method, :other) { 123 }",
            "send(:not_a_binder, :__dir__)",
            "undef_method :other",
            "undef :other",
        ] {
            let source = format!("{binding}\nFile.dirname(File.realpath(__FILE__))\n");
            assert_correction(&Dir, &source, &format!("{binding}\n__dir__\n"));
        }
    }

    #[test]
    fn struct_and_data_readers_withhold_edits_without_hiding_diagnostics() {
        for constructor in ["Struct.new", "::Struct.new", "Data.define", "::Data.define"] {
            for members in [
                ":__dir__",
                ":other, :__dir__",
                ":\"__di\\x72__\"",
                "name",
                "*names",
            ] {
                for expression in [
                    "File.dirname(File.realpath(__FILE__))",
                    "File.expand_path(File.dirname(__FILE__))",
                ] {
                    let source = format!(
                        "C = {constructor}({members}) {{ def run; {expression}; end }}\nDir.children('.').empty?\n"
                    );
                    let found = correction_offenses(&Dir, &source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(found[0].line, 1, "{source}");
                    assert_eq!(
                        found[0].col as usize,
                        source.find(expression).unwrap() + 1,
                        "{source}"
                    );
                    assert_eq!(
                        found[0].message,
                        "Use `__dir__` to get an absolute path to the current file's directory."
                    );
                    assert!(found[0].correction.is_none(), "{source}");
                    assert!(
                        correction_offenses(&super::super::DirEmpty, &source)[0]
                            .correction
                            .is_some(),
                        "{source}"
                    );
                }
            }
        }
        for constructor in [
            "Struct.new('Named', '__dir__', keyword_init: true)",
            "Struct.new(nil, :__dir__)",
            "Data.define(\"__di\\x72__\")",
            "Struct.send(:new, :__dir__)",
            "Data.__send__('define', :__dir__)",
        ] {
            let source = format!("C = {constructor}\nFile.dirname(File.realpath(__FILE__))\n");
            assert!(
                correction_offenses(&Dir, &source)[0].correction.is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn unrelated_struct_and_data_members_keep_canonical_edits() {
        for constructor in [
            "Struct.new(:other)",
            "::Struct.new(:other, keyword_init: true)",
            "Struct.new('Named', 'other')",
            "Struct.new(nil, :other)",
            "Struct.new(:other, keyword_init: __dir__)",
            "Data.define(:other)",
            "::Data.define('other')",
            "Struct.send(:new, :other)",
            "Data.public_send(:define, :other)",
            "Struct.other(:__dir__)",
            "Data.new(:__dir__)",
        ] {
            let source = format!("C = {constructor}\nFile.dirname(File.realpath(__FILE__))\n");
            assert_correction(&Dir, &source, &format!("C = {constructor}\n__dir__\n"));
        }
        // Other may be an IO-class alias. An unknown constructor can expose
        // the acquired __FILE__ bytes; unlike Struct/Data it has no proof.
        let found = correction_offenses(
            &Dir,
            "C = Other.new(:__dir__)\nFile.dirname(File.realpath(__FILE__))\n",
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn preserves_headers_line_endings_data_and_trailing_comments() {
        assert_correction(
            &Dir,
            "\u{feff}# encoding: UTF-8\r\nx = File.dirname(File.realpath(__FILE__)) # keep\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nx = __dir__ # keep\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
        assert_correction(
            &Dir,
            "File.\n expand_path(File.dirname(__FILE__))\n",
            "__dir__\n",
        );
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
