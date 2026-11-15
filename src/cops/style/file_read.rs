//! `Style/FileRead` — direct whole-file reads.
use super::file_api::{no_arguments, read_replacement};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
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
        // Update modes require write access. Explicit text mode enables newline
        // conversion (even on Unix); plain File.read does not preserve it.
        let correction = (cx.corrections_requested() && !mode.contains('+') && mode != "rt")
            .then(|| {
                // File.open accepts integer descriptors; File.read does not. Limit
                // fixes to known path strings/__FILE__, preserving detection for all
                // upstream matches, and never infer arbitrary Ruby values or flow.
                if filename.as_string_node().is_none() && filename.as_source_file_node().is_none() {
                    return None;
                }
                let path = std::str::from_utf8(filename.location().as_slice()).ok()?;
                let receiver = open.receiver()?;
                let receiver = std::str::from_utf8(receiver.location().as_slice()).ok()?;
                // A chained read with its own block is not the canonical open block.
                if read.location().end_offset() != node.location().end_offset()
                    && read.as_call_node().is_some_and(|c| c.block().is_some())
                {
                    return None;
                }
                read_replacement(&read, cx, file, format!("{receiver}.{method}({path})"))
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
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    #[test]
    fn pinned_detection_and_canonical_literal_corrections() {
        for receiver in ["File", "::File"] {
            for mode in ["", "r", "rt", "rb", "r+", "r+t", "r+b"] {
                let mode_arg = if mode.is_empty() {
                    String::new()
                } else {
                    format!(", '{mode}'")
                };
                let method = if mode.ends_with('b') {
                    "binread"
                } else {
                    "read"
                };
                for suffix in [
                    ".read",
                    ", &:read)",
                    " { |f| f.read }",
                    " do |f|\n    f.read\n  end",
                ] {
                    let open = format!("{receiver}.open(filename{mode_arg})");
                    let call = if suffix == ", &:read)" {
                        format!("{}{suffix}", open.trim_end_matches(')'))
                    } else {
                        format!("{open}{suffix}")
                    };
                    let source = format!("# header\n  {call}\n");
                    let found: Vec<_> = lint_str(&source)
                        .into_iter()
                        .filter(|o| o.cop == "Style/FileRead")
                        .collect();
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!((found[0].line, found[0].col), (2, 3));
                    assert_eq!(found[0].message, format!("Use `File.{method}`."));
                    // Unknown filename values could be file descriptors, not paths.
                    assert_correction(&FileRead, &source, &source);
                    let literal = source.replace("filename", "'file'");
                    if mode.contains('+') || mode == "rt" {
                        assert_correction(&FileRead, &literal, &literal);
                    } else {
                        assert_correction(
                            &FileRead,
                            &literal,
                            &format!("# header\n  {receiver}.{method}('file')\n"),
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn withholds_uncertain_values_resource_modes_and_source_sensitive_edits() {
        for source in [
            "File.open(fd).read",
            "File.open(3).read",
            "File.open(*paths).read",
            "File.open(**options).read",
            "File.open(path()).read",
            "File.open(helper { break 'file' }).read",
            "File.open(\"#{next_path}\").read",
            "File.open('file', 'r+').read",
            "File.open('file', 'rt').read",
            "File.open('file', 'r+t', &:read)",
            "File.open('file', 'r+b') { |f| f.read }",
            "File.open( # retain\n 'file').read",
            "File.open('file') { |f| # retain\n f.read }",
            "File.open(<<~PATH).read\nfile\nPATH\n",
            "File.open(helper(<<~PATH)).read\nfile\nPATH\n",
            "File.open('file') do |f|\n f.read\nend\np __LINE__\n",
            "File.open('file') do |f|\n f.read\nend # rustocop:disable Lint/Debugger\n",
            "File.open('file').read { side_effect }",
        ] {
            let found = correction_offenses(&FileRead, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn api_string_eval_and_unknown_eval_arguments_withhold_edits() {
        for binding in [
            "File.class_eval('def self.read(path); 123; end')",
            "::File.singleton_class.module_eval(\"alias read basename\")",
            "File.instance_eval(code)",
            "IO.class_eval('def read; 123; end')",
            "Symbol.class_eval('def to_proc; proc { 123 }; end')",
            "File.instance_eval { singleton_class.class_eval(code) }",
        ] {
            let source = format!("{binding}\nFile.open('file').read\n");
            let found = correction_offenses(&FileRead, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
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
    fn cached_binding_safeguards_cover_defs_aliases_attributes_struct_data_and_eval() {
        for binding in [
            "File = Custom",
            "module M; File = Custom; end",
            "class File; end",
            "class IO; end",
            "class Symbol; end",
            "def File.read(*args); 123; end",
            "def read; 123; end",
            "alias read object_id",
            "undef read",
            "alias_method :read, :object_id",
            "define_method('read') { 123 }",
            "object.define_singleton_method(:binread) { 123 }",
            "attr_reader :other, :read",
            "attr_accessor :open",
            "attr :to_proc, true",
            "send(:alias_method, :read, :other)",
            "object.__send__('define_method', 'read') { 123 }",
            "Struct.new(:read)",
            "Data.define(:open)",
            "Struct.new('Named', 'read', keyword_init: true)",
            "Data.public_send(:define, :binread)",
            "File.define_singleton_method(:open) { 123 }",
            "File.singleton_class.alias_method(:read, :other)",
            "File.private_class_method(:binread)",
            "File.instance_eval { def read(path); 123; end }",
            "File.singleton_class.class_eval { alias_method :read, :other }",
            "File.instance_eval { Dir.class_exec(private_class_method(:read)) {} }",
            "File.singleton_class.module_exec(:read) { |name| define_method(name) { 123 } }",
            "define_method(name) { 123 }",
            "attr_reader name",
            "Struct.new(*members)",
        ] {
            let source = format!("{binding}\n  File.open('file').read\n");
            let found = correction_offenses(&FileRead, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert!(found[0].correction.is_none(), "{source}");
        }
        for binding in [
            "def other; 123; end",
            "alias other read",
            "attr_writer :read",
            "Struct.new(:other)",
            "Data.define(:other)",
            "File.define_singleton_method(:other) { 123 }",
            "alias_method :other, :read",
            "File.singleton_class.class_eval { alias_method :other, :read }",
            "# define_method(:read) { 123 }",
        ] {
            let source = format!("{binding}\nFile.open('file').read\n");
            assert_correction(
                &FileRead,
                &source,
                &format!("{binding}\nFile.read('file')\n"),
            );
        }
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
