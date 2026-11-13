//! `Style/EmptyMethod` — compact empty method definitions.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{DefNode, Node};

pub struct EmptyMethod {
    max: Option<usize>,
}
impl Default for EmptyMethod {
    fn default() -> Self {
        Self { max: Some(120) }
    }
}
impl Cop for EmptyMethod {
    fn name(&self) -> &'static str {
        "Style/EmptyMethod"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_line_length_limit(&self, max: Option<usize>) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self { max }))
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(def) = node.as_def_node() else {
            return;
        };
        let range = def.location();
        if def.body().is_some()
            || def.equal_loc().is_some()
            || file.line_of(range.start_offset()) == file.last_line_of(def.location())
            || super::correction::contains_comment(file, range.start_offset()..range.end_offset())
        {
            return;
        }
        let args = arguments(&def);
        if args.iter().any(|arg| {
            file.line_of(arg.location().start_offset()) != file.last_line_of(arg.location())
        }) {
            return;
        }
        let (line, col) = file.line_col(range.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(range),
            message: "Put empty method definitions on a single line.".into(),
            correction: cx
                .corrections_requested()
                .then(|| self.correction(node, &def, &args, cx, file))
                .flatten(),
        });
    }
}
fn arguments<'pr>(def: &DefNode<'pr>) -> Vec<Node<'pr>> {
    let Some(p) = def.parameters() else {
        return Vec::new();
    };
    let mut args: Vec<_> = p
        .requireds()
        .iter()
        .chain(p.optionals().iter())
        .chain(p.rest())
        .chain(p.posts().iter())
        .chain(p.keywords().iter())
        .chain(p.keyword_rest())
        .chain(p.block().map(|b| b.as_node()))
        .collect();
    args.sort_by_key(|a| a.location().start_offset());
    args
}
impl EmptyMethod {
    fn correction(
        &self,
        node: &Node<'_>,
        def: &DefNode<'_>,
        args: &[Node<'_>],
        cx: &NodeContext<'_, '_>,
        file: &SourceFile,
    ) -> Option<Correction> {
        let loc = def.location();
        let range = loc.start_offset()..loc.end_offset();
        if !super::correction::method_newline_safe(node, cx, file, range.clone()) {
            return None;
        }
        let line = file.line_of(range.start);
        // Defaults/receivers must not move to different execution/backtrace lines.
        let header =
            std::str::from_utf8(&file.bytes[range.start..def.name_loc().end_offset()]).ok()?;
        if header.contains(['\n', '\r'])
            || args
                .iter()
                .any(|a| file.line_of(a.location().start_offset()) != line)
        {
            return None;
        }
        let mut replacement = header.to_string();
        if !args.is_empty() {
            let joined = args
                .iter()
                .map(|a| std::str::from_utf8(a.location().as_slice()).ok())
                .collect::<Option<Vec<_>>>()?
                .join(", ");
            if def.lparen_loc().is_some() {
                replacement.push_str(&format!("({joined})"));
            } else {
                replacement.push_str(&format!(" {joined}"));
            }
        }
        replacement.push_str("; end");
        if self
            .max
            .is_some_and(|max| replacement.chars().count() > max)
        {
            return None;
        }
        // Never relocate an end-line comment/magic header, or an inline statement.
        let tail_end = file
            .line_start(file.line_of(range.end) + 1)
            .min(file.bytes.len());
        if !file.bytes[range.end..tail_end]
            .iter()
            .all(u8::is_ascii_whitespace)
        {
            return None;
        }
        // Keep subsequent execution/declaration/backtrace lines in place. At EOF,
        // the pinned compact spelling needs no otherwise-observable blank padding.
        let root = cx.ancestors().next_back()?;
        if range.end < root.location().end_offset() {
            let newline = if file.bytes[file.line_start(line)..file.line_start(line + 1)]
                .ends_with(b"\r\n")
            {
                "\r\n"
            } else {
                "\n"
            };
            for _ in memchr::memchr_iter(b'\n', loc.as_slice()) {
                replacement.push_str(newline);
            }
        }
        Some(Correction { range, replacement })
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/empty_method_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn all_pinned_diagnostics_and_canonical_safe_corrections() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/empty_method_pinned.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let before = case["source"].as_str().unwrap();
            let max = if case["toml"]["Layout/LineLength"]["enabled"] == false {
                None
            } else {
                Some(
                    case["toml"]["Layout/LineLength"]["max"]
                        .as_u64()
                        .unwrap_or(120) as usize,
                )
            };
            let found = crate::cops::testing::correction_offenses(&EmptyMethod { max }, before);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{case}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(messages), case["messages"], "{case}");
            if let Some(after) = case["correction"].as_str() {
                assert_correction(&EmptyMethod { max }, before, after);
            } else {
                assert!(found.iter().all(|o| o.correction.is_none()), "{case}");
            }
        }
    }
    #[test]
    fn nonempty_commented_compact_and_multiline_argument_lookalikes() {
        for source in [
            "def f; end",
            "def self.f; end",
            "def f = nil",
            "def f\n nil\nend",
            "def f\n false\nend",
            "def f\n return\nend",
            "def f\n # keep\nend",
            "def f # keep\nend",
            "def self.f\n # keep\nend",
            "def f\n ensure\nend",
            "def f(a = {\n x: 1\n})\nend",
            "def f(a = \"x\ny\")\nend",
        ] {
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == "Style/EmptyMethod")
                .collect();
            assert!(found.is_empty(), "{source}");
        }
    }
    #[test]
    fn unsafe_line_comment_heredoc_and_source_observer_edits_are_withheld() {
        for source in [
            "def f\nend # keep\n",
            "def f\nend; raise 'x'\n",
            "def f\nend\np __LINE__\n",
            "def f(a = __LINE__)\nend\n",
            "def f\nend\np method(:f).source_location\n",
            "def f\nend\np DATA.pos\n__END__\nraw\n",
            "def f\nend\np File.read(__FILE__)\n",
            "def f(\n a = risky\n)\nend",
            "def f(a = <<~TEXT)\ntext\nTEXT\nend",
            "def outer; def inner\nend; end",
        ] {
            let found = crate::cops::testing::correction_offenses(&EmptyMethod::default(), source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn subsequent_execution_lines_and_literal_token_bytes_are_preserved() {
        assert_correction(
            &EmptyMethod::default(),
            "class C\n  def f(a = 'x  y', *rest, k: false, **kw, &b)\n  end\nend\n",
            "class C\n  def f(a = 'x  y', *rest, k: false, **kw, &b); end\n\nend\n",
        );
        assert_correction(
            &EmptyMethod::default(),
            "def f\nend\nraise 'x'\n",
            "def f; end\n\nraise 'x'\n",
        );
    }
    #[test]
    fn compact_empty_definitions_and_exact_diagnostic() {
        for (before, after) in [
            ("def foo\nend\n", "def foo; end\n"),
            ("def foo(bar, baz)\nend\n", "def foo(bar, baz); end\n"),
            ("def foo bar, baz\nend\n", "def foo bar, baz; end\n"),
            ("def foo\n\nend\n", "def foo; end\n"),
            ("def foo(arg\n); end\n", "def foo(arg); end\n"),
            ("def self.foo\nend\n", "def self.foo; end\n"),
            ("def foo(...)\nend\n", "def foo(...); end\n"),
            ("def foo(*, **, &)\nend\n", "def foo(*, **, &); end\n"),
            ("private def foo()\nend\n", "private def foo; end\n"),
        ] {
            let found: Vec<_> = lint_str(before)
                .into_iter()
                .filter(|o| o.cop == "Style/EmptyMethod")
                .collect();
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, if before.starts_with("private ") { 9 } else { 1 })
            );
            assert_eq!(
                found[0].message,
                "Put empty method definitions on a single line."
            );
            assert_correction(&EmptyMethod::default(), before, after);
        }
    }
}
