//! `Security/Open` — flags `Kernel#open` and `URI.open` with an argument
//! that may start with `|`, which makes them run a shell command.
//!
//! Matches RuboCop's semantics: a plain call (not `&.`) to `open` without a
//! receiver, or on `URI` or `::URI`, is reported at the method name unless
//! its first argument is known not to start with `|`:
//!
//! - a plain string that is not empty and does not start with `|` (leading
//!   whitespace is not stripped), or `__FILE__`;
//! - an interpolated string (or adjacent literals) whose first part is such
//!   a string;
//! - a `+` concatenation whose receiver is such a plain string.
//!
//! One deviation: the parser gem splits a non-squiggly string literal that
//! spans several source lines (`"foo⏎bar"`, a multi-line `<<-X` heredoc)
//! into a `dstr`, so RuboCop flags `open("foo⏎bar" + y)`. Prism gives one
//! string; rustocop judges it by its content and accepts it.
//!
//! As in RuboCop's AST, a block argument (`&blk`) counts as an argument.
//! `Kernel.open(x)` and `open` without arguments are not reported.

use ruby_prism::{CallNode, Node};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{call_arguments, is_global_const},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Security/Open` cop.
pub struct Open;

impl Cop for Open {
    fn name(&self) -> &'static str {
        "Security/Open"
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
        if call.name().as_slice() != b"open" || call.is_safe_navigation() {
            return;
        }
        let receiver = match call.receiver() {
            None => None,
            Some(receiver) if is_global_const(&receiver, &["URI"]) => Some(receiver),
            Some(_) => return,
        };
        let Some(first) = call_arguments(&call).into_iter().next() else {
            return;
        };
        if is_safe(&first) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let method = match receiver {
            Some(receiver) => format!(
                "{}.open",
                String::from_utf8_lossy(receiver.location().as_slice())
            ),
            None => "Kernel#open".to_string(),
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: format!("The use of `{method}` is a serious security risk."),
            line,
            end_line: line,
            col,
        });
    }
}

/// Whether the argument is known not to start with `|` (RuboCop's `safe?`).
fn is_safe(node: &Node<'_>) -> bool {
    match node {
        Node::StringNode { .. } => {
            let string = node.as_string_node().unwrap();
            let content = string.unescaped();
            !content.is_empty() && !content.starts_with(b"|")
        }
        // A string holding the file's path in RuboCop's AST.
        Node::SourceFileNode { .. } => true,
        Node::InterpolatedStringNode { .. } => node
            .as_interpolated_string_node()
            .unwrap()
            .parts()
            .iter()
            .next()
            .is_some_and(|first| is_safe(&first)),
        Node::CallNode { .. } => is_concatenated_string(&node.as_call_node().unwrap()),
        _ => false,
    }
}

/// `"literal" + rest`, with a safe plain string (or `__FILE__`) on the left.
fn is_concatenated_string(call: &CallNode<'_>) -> bool {
    call.name().as_slice() == b"+"
        && !call.is_safe_navigation()
        && call.receiver().is_some_and(|receiver| {
            matches!(
                receiver,
                Node::StringNode { .. } | Node::SourceFileNode { .. }
            ) && is_safe(&receiver)
        })
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/security/open_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::pipeline::lint_str;

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Security/Open")
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn at(line: u32, col: u32, method: &str) -> (u32, u32, String) {
        (
            line,
            col,
            format!("The use of `{method}` is a serious security risk."),
        )
    }

    #[test]
    fn flags_kernel_open_with_dynamic_arguments() {
        for source in [
            "open(something)\n",
            "open(something, \"r\")\n",
            "open(\"#{foo}.txt\")\n",
            "open(\"| #{foo}\")\n",
            "open('| foo')\n",
        ] {
            assert_eq!(found(source), [at(1, 1, "Kernel#open")], "{source:?}");
        }
    }

    #[test]
    fn flags_open_with_a_block() {
        assert_eq!(
            found("open(\"#{foo}.txt\") do |f|\n  f.gets\nend\n"),
            [at(1, 1, "Kernel#open")]
        );
        assert_eq!(
            found("::URI.open(\"| #{foo}\") do |f|\n  f.gets\nend\n"),
            [at(1, 7, "::URI.open")]
        );
    }

    #[test]
    fn flags_uri_open_and_names_the_receiver_as_written() {
        assert_eq!(found("URI.open(\"| #{foo}\")\n"), [at(1, 5, "URI.open")]);
        assert_eq!(
            found("::URI.open(\"| #{foo}\")\n"),
            [at(1, 7, "::URI.open")]
        );
        assert_eq!(found("URI.open(url)\n"), [at(1, 5, "URI.open")]);
        assert_eq!(
            found("body = URI\n  .open(url)\n"),
            [at(2, 4, "URI.open")],
            "reported at the method name"
        );
    }

    #[test]
    fn flags_unsafe_string_shapes() {
        for source in [
            "open(\"\")\n",
            "open('')\n",
            "open(\"\" \"#{x}\")\n",
            "open(\"#{\"|\"}foo\")\n",
            "open(\"#@path\")\n",
            "open(?|)\n",
            "open(\"\\| ls\")\n",
            "open(<<~X)\n  #{x}\nX\n",
            "open(<<~X)\n  | ls\nX\n",
        ] {
            assert_eq!(found(source), [at(1, 1, "Kernel#open")], "{source:?}");
        }
    }

    #[test]
    fn flags_concatenations_not_led_by_a_safe_plain_string() {
        for source in [
            "open(\"|\" + y)\n",
            "open(\"#{x}\" + y)\n",
            "open((\"foo\") + y)\n",
            "open(\"foo\"&.+(y))\n",
            "open(\"foo\" + y + z)\n",
            "open('a' 'b' + x)\n",
        ] {
            assert_eq!(found(source), [at(1, 1, "Kernel#open")], "{source:?}");
        }
    }

    #[test]
    fn flags_other_dynamic_argument_kinds() {
        for source in [
            "open(*args)\n",
            "open(path: x)\n",
            "open(:sym)\n",
            "open(`ls`)\n",
            "open(&blk)\n",
        ] {
            assert_eq!(found(source), [at(1, 1, "Kernel#open")], "{source:?}");
        }
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "open = something\n",
            "File.open(something)\n",
            "open(\"foo.txt\")\n",
            "open\n",
            "open \"prefix_#{foo}\"\n",
            "open \"prefix\" + foo\n",
            "open \"foo#{2}.txt\"\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn accepts_safe_string_shapes() {
        for source in [
            "URI.open(\"http://example.com\")\n",
            "URI.parse(url).open\n",
            "open(\"foo\" \"#{x}\")\n",
            "open(\"prefix\" \"| #{x}\")\n",
            "open \"a\" \"b\"\n",
            "open(%q(foo))\n",
            "open(?a)\n",
            "open(\"foo\".+(y))\n",
            "open(<<~X)\n  foo\nX\n",
            "open(<<~X)\n  foo\n  #{x}\nX\n",
            "open(__FILE__)\n",
            "open(__FILE__ + \".bak\")\n",
            "URI.open(__FILE__ + x)\n",
            // Multi-line strings are judged by content (RuboCop flags these
            // two, as the parser gem splits them into a `dstr`).
            "open(<<-X + y)\nfoo\nbar\nX\n",
            "open(\"foo\nbar\" + y)\n",
            // RuboCop does not strip leading whitespace here.
            "open(\" | ls\")\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "self.open(x)\n",
            "Kernel.open(x)\n",
            "URI&.open(x)\n",
            "Foo::URI.open(x)\n",
            "IO.popen(x)\n",
            "opener(x)\n",
            "obj.open(x)\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
