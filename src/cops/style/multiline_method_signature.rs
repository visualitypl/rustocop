//! `Style/MultilineMethodSignature` — opt-in signature collapsing.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MultilineMethodSignature {
    max_line_length: Option<usize>,
}
impl Default for MultilineMethodSignature {
    fn default() -> Self {
        Self {
            max_line_length: Some(120),
        }
    }
}
impl Cop for MultilineMethodSignature {
    fn name(&self) -> &'static str {
        "Style/MultilineMethodSignature"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_line_length_limit(&self, max: Option<usize>) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            max_line_length: max,
        }))
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
        let (Some(left), Some(right), Some(parameters)) =
            (def.lparen_loc(), def.rparen_loc(), def.parameters())
        else {
            return;
        };
        let start = def.location().start_offset();
        let (line, col) = file.line_col(start);
        if line == file.line_of(right.start_offset()) {
            return;
        }
        let args: Vec<_> = parameters
            .requireds()
            .iter()
            .chain(parameters.optionals().iter())
            .chain(parameters.rest())
            .chain(parameters.posts().iter())
            .chain(parameters.keywords().iter())
            .chain(parameters.keyword_rest())
            .chain(parameters.block().map(|b| b.as_node()))
            .collect();
        if args.is_empty()
            || args.iter().any(|n| {
                file.line_of(n.location().start_offset()) != file.last_line_of(n.location())
            })
        {
            return;
        }
        // Match the pinned collapsed-width calculation, without rewriting any
        // token bytes (in particular, whitespace inside default literals).
        let mut width = 0;
        let mut space = false;
        for offset in file.character_offsets(start..right.end_offset()) {
            if super::is_ruby_whitespace(file.bytes[offset]) {
                if !space {
                    width += 1;
                }
                space = true;
            } else {
                width += 1;
                space = false;
            }
        }
        if self
            .max_line_length
            .is_some_and(|max| col as usize - 1 + width > max)
        {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                let range = def.def_keyword_loc().end_offset()..right.end_offset();
                if !super::correction::method_newline_safe(
                    node,
                    cx,
                    file,
                    start..right.end_offset(),
                ) {
                    return None;
                }
                let comments = file.comments();
                let first = comments.partition_point(|c| c.end <= start);
                if comments
                    .get(first)
                    .is_some_and(|c| c.start < right.end_offset())
                {
                    return None;
                }
                // Only whitespace may separate `def` from its name/receiver, and
                // parameters from their comma delimiters. Never regex-join tokens.
                let name_start = def.receiver().map_or(def.name_loc().start_offset(), |r| {
                    r.location().start_offset()
                });
                if !file.bytes[range.start..name_start]
                    .iter()
                    .all(u8::is_ascii_whitespace)
                {
                    return None;
                }
                let header =
                    std::str::from_utf8(&file.bytes[name_start..left.end_offset()]).ok()?;
                if header.contains(['\n', '\r']) {
                    return None;
                }
                let mut cursor = left.end_offset();
                let mut joined = Vec::new();
                for arg in &args {
                    let location = arg.location();
                    if !file.bytes[cursor..location.start_offset()]
                        .iter()
                        .all(|b| b.is_ascii_whitespace() || *b == b',')
                    {
                        return None;
                    }
                    joined.push(std::str::from_utf8(location.as_slice()).ok()?);
                    cursor = location.end_offset();
                }
                if !file.bytes[cursor..right.start_offset()]
                    .iter()
                    .all(|b| b.is_ascii_whitespace() || *b == b',')
                {
                    return None;
                }
                Some(Correction {
                    range,
                    replacement: format!(" {header}{})", joined.join(", ")),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Avoid multi-line method signatures.".into(),
            line,
            col,
            end_line: file.last_line_of(def.location()),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/multiline_method_signature_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, offense::Offense, pipeline::lint_str_with};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str_with(&[&MultilineMethodSignature::default()], source)
    }
    #[test]
    fn pinned_signatures_and_corrections() {
        for (before, after, line, col) in [
            ("def foo(bar\n    )\nend\n", "def foo(bar)\nend\n", 1, 1),
            (
                "class Foo\n  def foo(\n    arg\n)\n  end\nend\n",
                "class Foo\n  def foo(arg)\n  end\nend\n",
                2,
                3,
            ),
            ("def\nfoo(bar\n    )\nend\n", "def foo(bar)\nend\n", 1, 1),
            (
                "def\n\n\nfoo(bar\n    )\nend\n",
                "def foo(bar)\nend\n",
                1,
                1,
            ),
            (
                "def\nfoo(bar,\n    baz)\nend\n",
                "def foo(bar, baz)\nend\n",
                1,
                1,
            ),
            (
                "def self.foo(bar\n    )\nend\n",
                "def self.foo(bar)\nend\n",
                1,
                1,
            ),
            (
                "def foo(bar,\n        baz)\nend\n",
                "def foo(bar, baz)\nend\n",
                1,
                1,
            ),
            (
                "def foo(bar,\n        baz); end\n",
                "def foo(bar, baz); end\n",
                1,
                1,
            ),
            (
                "def foo(bar,\n        baz\n       ); end\n",
                "def foo(bar, baz); end\n",
                1,
                1,
            ),
            (
                "def self.foo(bar,\n        baz)\nend\n",
                "def self.foo(bar, baz)\nend\n",
                1,
                1,
            ),
            (
                "def self.foo(bar,\n        baz); end\n",
                "def self.foo(bar, baz); end\n",
                1,
                1,
            ),
            (
                "def self.foo(bar,\n        baz\n    ); end\n",
                "def self.foo(bar, baz); end\n",
                1,
                1,
            ),
            (
                "def foo(bar,\n        baz)\n  qux.qux\nend\n",
                "def foo(bar, baz)\n  qux.qux\nend\n",
                1,
                1,
            ),
            (
                "class Foo\n  private def bar(baz,\n                  qux)\n  end\nend\n",
                "class Foo\n  private def bar(baz, qux)\n  end\nend\n",
                2,
                11,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(found[0].message, "Avoid multi-line method signatures.");
            assert_correction(&MultilineMethodSignature::default(), before, after);
        }
    }
    #[test]
    fn literal_tokens_destructuring_forwarding_setters_and_endless_are_copied_verbatim() {
        for (before, after) in [
            (
                "def f(a = \" x   y \",\n b = %q(a,  b),\n key: / a  b /)\n [a,b,key]\nend\n",
                "def f(a = \" x   y \", b = %q(a,  b), key: / a  b /)\n [a,b,key]\nend\n",
            ),
            (
                "def f((a, b),\n c, *rest, d, key:, opt: 1, **kw, &block)\nend\n",
                "def f((a, b), c, *rest, d, key:, opt: 1, **kw, &block)\nend\n",
            ),
            ("def f(a,\n **nil)\nend\n", "def f(a, **nil)\nend\n"),
            ("def f(...\n)\nend\n", "def f(...)\nend\n"),
            (
                "def self.[]=(k,\n v)\n @x = v\nend\n",
                "def self.[]=(k, v)\n @x = v\nend\n",
            ),
            (
                "private def +(other,\n key: 1)\n other\nend\n",
                "private def +(other, key: 1)\n other\nend\n",
            ),
            ("def f(a,\n b) = [a,b]\n", "def f(a, b) = [a,b]\n"),
        ] {
            assert_correction(&MultilineMethodSignature::default(), before, after);
        }
    }
    #[test]
    fn configured_width_includes_visibility_and_disabling_length_removes_limit() {
        let source = "class Foo\n  private def bar(baz,\n                  qux)\n  end\nend\n";
        for (max, count) in [(Some(5), 0), (Some(20), 0), (Some(30), 1), (None, 1)] {
            let cop = MultilineMethodSignature::default()
                .with_line_length_limit(max)
                .unwrap();
            assert_eq!(lint_str_with(&[cop.as_ref()], source).len(), count);
        }
        let source = format!("def f({},\n b)\nend\n", "a".repeat(120));
        assert!(offenses(&source).is_empty());
        let cop = MultilineMethodSignature::default()
            .with_line_length_limit(None)
            .unwrap();
        assert_eq!(lint_str_with(&[cop.as_ref()], &source).len(), 1);
    }
    #[test]
    fn comments_directives_heredocs_line_values_and_observers_withhold_only_edits() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "def f(a, # explanation\n b)\nend\n",
            "def f(a,\n b) # rustocop:disable Layout\nend\n",
            "def f(a = __LINE__,\n b = 2)\nend\n",
            "def f(a,\n b)\nend\np __LINE__\n",
            "def f(a,\n b)\n <<~S\npayload\nS\nend\n",
            "def f(\n a = <<~S)\npayload\nS\nend\n",
            "def f(...\n)\n target(...)\nend\n",
            "def (\n Object.new\n).f(a,\n b)\nend\n",
        ] {
            assert_eq!(
                ruby_prism::parse(source.as_bytes()).errors().count(),
                0,
                "{source}"
            );
            let found = correction_offenses(&MultilineMethodSignature::default(), source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for observer in [
            "p method(:f).source_location",
            "p errors.map(&:full_message)",
            "p caller",
            "obj.public_send(selector)",
            "p DATA.pos",
            "p Object.const_get(:DATA).lineno",
        ] {
            let source = format!("def f(a,\n b)\nend\n{observer}\n__END__\nraw\n");
            let found = correction_offenses(&MultilineMethodSignature::default(), &source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{observer}");
        }
    }
    #[test]
    fn pinned_lookalikes_and_multiline_defaults() {
        for source in [
            "def foo(bar, baz)\nend",
            "def foo\nend",
            "def self.foo(bar, baz)\nend",
            "def self.foo\nend",
            "def\nmethod_name arg; end",
            "def foo(bar, baz = {\nqux: 1\n})\nend",
            "def foo(bar,\n baz = (qux = false\n nil))\nend",
            "def foo bar,\n baz\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
