//! `Style/ExpandPathArguments` — canonical current-file expand_path arguments.
use super::file_api::{argument, no_arguments, replacement, send};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ExpandPathArguments;
impl Cop for ExpandPathArguments {
    fn name(&self) -> &'static str {
        "Style/ExpandPathArguments"
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
        let Some(call) = node
            .as_call_node()
            .filter(|c| !c.is_safe_navigation() && c.name().as_slice() == b"expand_path")
        else {
            return;
        };
        let args = ast::call_arguments(&call);
        if call
            .receiver()
            .is_some_and(|r| ast::is_global_const(&r, &["File"]))
        {
            let [path, base] = args.as_slice() else {
                return;
            };
            if base.as_source_file_node().is_none() {
                return;
            }
            let Some(string) = path.as_string_node() else {
                return;
            };
            let current = String::from_utf8_lossy(string.unescaped());
            let mut parts: Vec<_> = current
                .split('/')
                .filter(|p| !p.is_empty() && *p != ".")
                .collect();
            let depth = parts.len();
            let canonical = std::str::from_utf8(string.unescaped()).is_ok()
                && std::str::from_utf8(path.location().as_slice()).is_ok()
                && (string.unescaped().is_ascii() || file.is_utf8())
                && !current.starts_with('/')
                && (parts.is_empty() || parts.first() == Some(&".."))
                && !current.bytes().any(|b| b.is_ascii_control());
            if let Some(index) = parts.iter().position(|p| *p == "..") {
                parts.remove(index);
            }
            let parent = parts.join("/");
            // Removing the parent component must not expose Ruby's leading-~
            // home expansion: ../~user is an ordinary relative path.
            let parent = if parent.starts_with('~') {
                format!("./{parent}")
            } else {
                parent
            };
            let new_args = if depth == 0 {
                "__FILE__".into()
            } else if depth == 1 {
                "__dir__".into()
            } else {
                format!("{}, __dir__", quote(&parent))
            };
            let start = call.message_loc().unwrap().start_offset();
            let prefix = &file.bytes[node.location().start_offset()..start];
            let text = std::str::from_utf8(prefix)
                .ok()
                .map(|p| format!("{p}expand_path({new_args})"));
            let correction = (cx.corrections_requested()
                && canonical
                && (depth == 0 || !file.noncanonical_load_path()))
            .then(|| {
                replacement(
                    node,
                    cx,
                    file,
                    text?,
                    if depth == 0 { "FilePath" } else { "File" },
                )
            })
            .flatten();
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "Use `expand_path({new_args})` instead of `expand_path({}, __FILE__)`.",
                    quote(&current)
                ),
                line,
                col,
                end_line: file.line_of(call.message_loc().unwrap().end_offset() - 1),
                correction,
            });
            return;
        }
        if !no_arguments(&call) {
            return;
        }
        let Some(parent) = call
            .receiver()
            .and_then(|n| send(&n))
            .filter(|c| c.name().as_slice() == b"parent" && no_arguments(c))
        else {
            return;
        };
        let Some(constructor) = parent.receiver().and_then(|n| send(&n)) else {
            return;
        };
        let factory =
            constructor.name().as_slice() == b"Pathname" && constructor.receiver().is_none();
        let new = constructor.name().as_slice() == b"new"
            && constructor
                .receiver()
                .is_some_and(|n| ast::is_global_const(&n, &["Pathname"]));
        if !factory && !new {
            return;
        }
        let Some(base) = argument(&constructor).filter(|n| n.as_source_file_node().is_some())
        else {
            return;
        };
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        let name = if factory { "Pathname" } else { "Pathname.new" };
        let correction = (cx.corrections_requested() && !file.noncanonical_load_path())
            .then(|| {
                let before =
                    std::str::from_utf8(&file.bytes[start..base.location().start_offset()]).ok()?;
                let middle = std::str::from_utf8(
                    &file.bytes[base.location().end_offset()..constructor.location().end_offset()],
                )
                .ok()?;
                let after =
                    std::str::from_utf8(&file.bytes[parent.location().end_offset()..end]).ok()?;
                replacement(
                    node,
                    cx,
                    file,
                    format!("{before}__dir__{middle}{after}"),
                    "Pathname",
                )
            })
            .flatten();
        let message = format!(
            "Use `{name}(__dir__).expand_path` instead of `{name}(__FILE__).parent.expand_path`."
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction,
        });
    }
}
fn quote(path: &str) -> String {
    format!("'{}'", path.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/expand_path_arguments_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    #[test]
    fn uncertain_paths_and_source_sensitive_changes_remain_reported() {
        for source in [
            "File.expand_path('lib', __FILE__)",
            "File.expand_path('/absolute', __FILE__)",
            "File.expand_path('~user', __FILE__)",
            "__dir__ = :custom; File.expand_path('..', __FILE__)",
            "def __dir__; :custom; end; Pathname(__FILE__).parent.expand_path",
            "File.expand_path( # comment\n '..', __FILE__)",
            "File.expand_path('..', __FILE__) { effect }",
            "File.expand_path(\n '..', __FILE__)\np __LINE__",
            "File.expand_path(<<~PATH, __FILE__)\n..\nPATH\n",
            "Pathname.new(__FILE__).parent. # retain\n expand_path",
        ] {
            let found = correction_offenses(&ExpandPathArguments, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn pathname_and_directory_bindings_withhold_only_uncertain_edits() {
        for binding in [
            "Pathname = Custom",
            "class Pathname; end",
            "def Pathname.new(x); x; end",
            "Pathname.define_singleton_method(:new) { |x| x }",
            "Pathname.class_eval { def parent; self; end }",
            "Pathname.singleton_class.class_eval { alias new allocate }",
            "Pathname.class_eval(dynamic)",
            "Object.const_set(:Pathname, Custom)",
            "def Pathname(x); x; end",
            "alias Pathname object_id",
            "define_method(:Pathname) { |x| x }",
            "attr_reader :Pathname",
        ] {
            for expression in [
                "Pathname(__FILE__).parent.expand_path",
                "Pathname.new(__FILE__).parent.expand_path",
            ] {
                let source = format!("{binding}\n{expression}\n");
                let found = correction_offenses(&ExpandPathArguments, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
            let source = format!("{binding}\nFile.expand_path('..', __FILE__)\n");
            // Unknown evaluated Ruby can return/acquire VM factories even
            // when it does not affect this cop's own File binding.
            assert_eq!(
                correction_offenses(&ExpandPathArguments, &source)[0]
                    .correction
                    .is_some(),
                binding != "Pathname.class_eval(dynamic)",
                "{source}"
            );
        }
    }
    #[test]
    fn decoded_literal_variants_and_source_headers() {
        for literal in ["%q(../..)", "\"..\\x2f..\"", "'./../..'", "\"./../..\""] {
            assert_correction(
                &ExpandPathArguments,
                &format!("File.expand_path({literal}, __FILE__)"),
                "File.expand_path('..', __dir__)",
            );
        }
        assert_correction(
            &ExpandPathArguments,
            "\u{feff}# encoding: UTF-8\r\nFile.expand_path('../..', __FILE__)\r\np __LINE__\r\n__END__\r\nraw\r\n",
            "\u{feff}# encoding: UTF-8\r\nFile.expand_path('..', __dir__)\r\np __LINE__\r\n__END__\r\nraw\r\n",
        );
    }
    #[test]
    fn exposed_tildes_remain_relative_paths() {
        for (path, expected) in [
            ("../~", "./~"),
            ("../~/lib", "./~/lib"),
            ("../~root", "./~root"),
            ("./..//./~nobody/lib", "./~nobody/lib"),
        ] {
            assert_correction(
                &ExpandPathArguments,
                &format!("File.expand_path('{path}', __FILE__)"),
                &format!("File.expand_path('{expected}', __dir__)"),
            );
        }
    }
    #[test]
    fn pinned_file_examples() {
        for receiver in ["File", "::File"] {
            for (path, argument) in [
                ("..", "__dir__"),
                ("../..", "'..', __dir__"),
                ("../../..", "'../..', __dir__"),
                (".", "__FILE__"),
                ("../../lib", "'../lib', __dir__"),
                ("./../..", "'..', __dir__"),
            ] {
                let source = format!("# header\n  {receiver}.expand_path('{path}', __FILE__)\n");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == "Style/ExpandPathArguments")
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (2, if receiver == "File" { 8 } else { 10 }, 2)
                );
                assert_eq!(
                    found[0].message,
                    format!(
                        "Use `expand_path({argument})` instead of `expand_path('{path}', __FILE__)`."
                    )
                );
                assert_correction(
                    &ExpandPathArguments,
                    &source,
                    &format!("# header\n  {receiver}.expand_path({argument})\n"),
                );
            }
        }
    }
    #[test]
    fn pinned_pathname_examples_and_lookalikes() {
        for (constructor, message_constructor) in [
            ("Pathname", "Pathname"),
            ("Pathname.new", "Pathname.new"),
            ("::Pathname.new", "Pathname.new"),
        ] {
            let source = format!("{constructor}(__FILE__).parent.expand_path\n");
            let found = correction_offenses(&ExpandPathArguments, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "Use `{message_constructor}(__dir__).expand_path` instead of `{message_constructor}(__FILE__).parent.expand_path`."
                )
            );
            assert_correction(
                &ExpandPathArguments,
                &source,
                &format!("{constructor}(__dir__).expand_path\n"),
            );
        }
        for source in [
            "File.expand_path(__dir__)",
            "File.expand_path('..', __dir__)",
            "File.expand_path(__FILE__)",
            "File.expand_path(path, __FILE__)",
            "File.expand_path(\"#{path}.png\", __FILE__)",
            "Pathname(__dir__).expand_path",
            "Other.expand_path('..', __FILE__)",
            "Other::File.expand_path('..', __FILE__)",
            "File&.expand_path('..', __FILE__)",
            "Pathname.new(__FILE__).parent(2).expand_path",
            "Pathname.new(__FILE__).parent.expand_path(base)",
            "loader.Pathname(__FILE__).parent.expand_path",
            "Other::Pathname.new(__FILE__).parent.expand_path",
            "File.expand_path('..', __FILE__, kw: true)",
        ] {
            assert!(
                !lint_str(source)
                    .iter()
                    .any(|o| o.cop == "Style/ExpandPathArguments"),
                "{source}"
            );
        }
    }
}
