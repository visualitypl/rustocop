//! `Security/IoMethods` — flags `IO.read`, `IO.binread`, `IO.write`,
//! `IO.binwrite`, `IO.foreach` and `IO.readlines`, which run a shell command
//! when the path starts with `|`; the `File` methods never do.
//!
//! Matches RuboCop's semantics: a plain call (not `&.`) to one of these
//! methods on `IO` is reported, at the start of the call, unless its first
//! argument is a plain string starting with `|` after leading whitespace
//! (an intentional command). One deviation: RuboCop only matches the
//! receiver spelled `IO`, while rustocop also flags `::IO`, the same class.
//! Another: the parser gem splits a non-squiggly string literal spanning
//! several source lines (a multi-line `<<-X` heredoc, `"| cmd⏎bar"`) into a
//! `dstr`, so RuboCop flags `IO.read` with it even when it starts with `|`.
//! Prism gives one string; rustocop judges it by its content and accepts it.

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{call_arguments, is_global_const},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `IO` class methods that spawn a command for a `|path`.
const METHODS: [&[u8]; 6] = [
    b"read",
    b"binread",
    b"write",
    b"binwrite",
    b"foreach",
    b"readlines",
];

/// The `Security/IoMethods` cop.
pub struct IoMethods;

impl Cop for IoMethods {
    fn name(&self) -> &'static str {
        "Security/IoMethods"
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
        let method = call.name().as_slice();
        if !METHODS.contains(&method)
            || call.is_safe_navigation()
            || !call
                .receiver()
                .is_some_and(|receiver| is_global_const(&receiver, &["IO"]))
        {
            return;
        }
        let intentional_command = call_arguments(&call)
            .first()
            .and_then(|first| first.as_string_node())
            .is_some_and(|string| starts_with_pipe(string.unescaped()));
        if intentional_command {
            return;
        }
        let method = String::from_utf8_lossy(method);
        let (line, col) = file.line_col(call.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("`File.{method}` is safer than `IO.{method}`."),
            line,
            end_line: file.line_of(crate::cops::ast::source_end(node).saturating_sub(1)),
            col,
        });
    }
}

/// Ruby's `value.strip.start_with?('|')`: `strip` drops leading NUL and
/// ASCII whitespace.
fn starts_with_pipe(value: &[u8]) -> bool {
    value
        .iter()
        .find(|&&byte| !matches!(byte, b'\0' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' '))
        == Some(&b'|')
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/security/io_methods_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::pipeline::lint_str;

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Security/IoMethods")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn at(line: u32, col: u32, method: &str) -> (u32, u32, String) {
        (
            line,
            col,
            format!("`File.{method}` is safer than `IO.{method}`."),
        )
    }

    #[test]
    fn flags_io_methods_with_a_variable_argument() {
        for (source, method) in [
            ("IO.read(path)\n", "read"),
            ("IO.write(path, \"hi\")\n", "write"),
            ("IO.binread(path)\n", "binread"),
            ("IO.binwrite(path, \"hi\")\n", "binwrite"),
            ("IO.readlines(path)\n", "readlines"),
            ("IO.foreach(path) { |x| puts x }\n", "foreach"),
        ] {
            assert_eq!(found(source), [at(1, 1, method)], "{source:?}");
        }
    }

    #[test]
    fn flags_io_methods_with_a_string_argument() {
        for (source, method) in [
            ("IO.read(\"command\")\n", "read"),
            ("IO.write(\"command\", \"hi\")\n", "write"),
            ("IO.binwrite(\"command\", \"hi\")\n", "binwrite"),
            ("IO.binwrite(path, \"hi\")\n", "binwrite"),
            ("IO.readlines(\"command\")\n", "readlines"),
            ("IO.foreach(\"command\") { |x| puts x }\n", "foreach"),
        ] {
            assert_eq!(found(source), [at(1, 1, method)], "{source:?}");
        }
    }

    #[test]
    fn flags_io_methods_with_a_non_string_literal_argument() {
        for (source, method) in [
            ("IO.read(:some_path)\n", "read"),
            ("IO.read(123)\n", "read"),
            ("IO.write(123, \"hi\")\n", "write"),
        ] {
            assert_eq!(found(source), [at(1, 1, method)], "{source:?}");
        }
    }

    #[test]
    fn flags_arguments_not_known_to_start_with_a_pipe() {
        for source in [
            "IO.read\n",
            "IO.read(&blk)\n",
            "IO.read(\"#{x}| cmd\")\n",
            "IO.read(\"| #{cmd}\")\n",
            "IO.read(__FILE__)\n",
            "IO.read(*[\"| cmd\"])\n",
            "IO.read(path, mode: \"rb\")\n",
        ] {
            assert_eq!(found(source), [at(1, 1, "read")], "{source:?}");
        }
    }

    #[test]
    fn reports_at_the_start_of_the_call() {
        assert_eq!(found("x = IO.binread(path)\n"), [at(1, 5, "binread")]);
        assert_eq!(found("IO\n  .read(path)\n"), [at(1, 1, "read")]);
    }

    #[test]
    fn flags_top_level_io() {
        // RuboCop only matches the receiver spelled `IO`; `::IO` is the same
        // class, so rustocop flags it too.
        assert_eq!(found("::IO.read(path)\n"), [at(1, 1, "read")]);
        assert_eq!(
            found("lines = ::IO.readlines(path, chomp: true)\n"),
            [at(1, 9, "readlines")]
        );
    }

    #[test]
    fn accepts_file_receiver() {
        for source in [
            "File.read(path)\n",
            "File.binread(path)\n",
            "File.binwrite(path, \"hi\")\n",
            "File.readlines(path)\n",
            "File.foreach(path) { |x| puts x }\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_no_receiver() {
        for source in [
            "read(\"command\")\n",
            "write(\"command\", \"hi\")\n",
            "binwrite(\"command\", \"hi\")\n",
            "readlines(\"command\")\n",
            "foreach(\"command\") { |x| puts x }\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_intentional_commands() {
        for source in [
            "IO.read(\"| command\")\n",
            "IO.write(\"| command\", \"hi\")\n",
            "IO.binwrite(\"| command\", \"hi\")\n",
            "IO.readlines(\"| command\")\n",
            "IO.foreach(\"| command\") { |x| puts x }\n",
            "IO.read(\" | command\")\n",
            "IO.write(\" | command\", \"hi\")\n",
            "IO.binwrite(\" | command\", \"hi\")\n",
            "IO.readlines(\" | command\")\n",
            "IO.foreach(\" | command\") { |x| puts x }\n",
            "IO.read(\"\\t\\n| command\")\n",
            "IO.read(\"\\v\\0\\f\\r| command\")\n",
            "IO.read('|')\n",
            "IO.read(?|)\n",
            "IO.read(<<~CMD)\n  | command\nCMD\n",
            // Multi-line strings are judged by content (RuboCop flags these
            // two, as the parser gem splits them into a `dstr`).
            "IO.read(<<-CMD)\n| command\nbar\nCMD\n",
            "IO.read(\"| command\nbar\")\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "IO&.read(path)\n",
            "Foo::IO.read(path)\n",
            "IO.popen(cmd)\n",
            "IO.readline(path)\n",
            "io.read(path)\n",
            "StringIO.read(path)\n",
            "IO.new(fd).read\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
