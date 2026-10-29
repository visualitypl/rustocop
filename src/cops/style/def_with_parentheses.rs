//! `Style/DefWithParentheses` — omit optional empty method parameters.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct DefWithParentheses;
impl Cop for DefWithParentheses {
    fn name(&self) -> &'static str {
        "Style/DefWithParentheses"
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
        let Some(definition) = node.as_def_node() else {
            return;
        };
        if definition.parameters().is_some() {
            return;
        }
        let (Some(left), Some(right)) = (definition.lparen_loc(), definition.rparen_loc()) else {
            return;
        };
        let after = file.bytes.get(right.end_offset());
        let single_line =
            file.line_of(node.location().start_offset()) == file.last_line_of(node.location());
        if after != Some(&b';')
            && ((single_line && definition.equal_loc().is_none()) || after == Some(&b'='))
        {
            return;
        }
        let range = left.start_offset()..right.end_offset();
        let (line, col) = file.line_col(range.start);
        // Only horizontal whitespace may disappear. Multiline/comment ranges
        // stay reported but unedited: no literal, directive or __LINE__ shifts.
        let correction = cx
            .corrections_requested()
            .then(|| {
                let separator = file.bytes[right.end_offset()..]
                    .iter()
                    .find(|b| !matches!(b, b' ' | b'\t' | b'\r'));
                let safe_separator = matches!(separator, Some(b';' | b'\n' | b'#'))
                    || (definition.equal_loc().is_some() && separator == Some(&b'='));
                (safe_separator
                    && file.bytes[left.end_offset()..right.start_offset()]
                        .iter()
                        .all(|b| matches!(b, b' ' | b'\t')))
                .then(|| Correction {
                    range: range.clone(),
                    replacement: String::new(),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Omit the parentheses in defs when the method doesn't accept any arguments."
                .into(),
            line,
            col,
            end_line: file.line_of(range.end - 1),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/def_with_parentheses_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/DefWithParentheses")
            .collect()
    }
    #[test]
    fn pinned_optional_parentheses_and_canonical_corrections() {
        for (source, col) in [
            ("def func()\nend\n", 9),
            ("def Test.func()\n  something\nend\n", 14),
            ("def foo() = do_something\n", 8),
            ("def foo() =do_something\n", 8),
            ("def foo(); end\n", 8),
            ("def foo(); bar; end\n", 8),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(
                found[0].message,
                "Omit the parentheses in defs when the method doesn't accept any arguments."
            );
            assert!(found[0].correction.is_none());
            assert_correction(&DefWithParentheses, source, &source.replace("()", ""));
        }
    }
    #[test]
    fn visibility_operators_singleton_endless_and_heredoc_bodies() {
        for source in [
            "private def foo()\n  42\nend\n",
            "def (receiver).foo() = __LINE__\n",
            "def self.!()\n  false\nend\n",
            "def self.foo(  ) # retain\n  __LINE__\nend\n",
            "def foo()\n  <<~TEXT\n    café #{__LINE__}\nTEXT\nend\np __LINE__\n",
            "def self.foo() = <<~TEXT\n  café #{__LINE__}\nTEXT\np __LINE__\n",
        ] {
            let after = source.replace("()", "").replace("(  )", "");
            assert_correction(&DefWithParentheses, source, &after);
        }
    }
    #[test]
    fn required_parentheses_parameters_and_lookalikes() {
        for source in [
            "def foo()= do_something\n",
            "def foo()=do_something\n",
            "def func(a)\nend\n",
            "def func\nend\n",
            "def to_s() join '/' end",
            "def foo()end",
            "def foo() end",
            "def foo(a = 1, *rest, key: 2, **kw, &block); end",
            "def foo(...) = bar(...)",
            "def foo(**nil); end",
            "foo()\n# def foo()\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn same_line_body_in_multiline_definition_does_not_become_parameters() {
        for source in [
            "def foo() bar\nend\n",
            "def foo()end\np __LINE__\n",
            "def foo() do_something(<<~TEXT)\n  body\nTEXT\nend\n",
        ] {
            let found = correction_offenses(&DefWithParentheses, source);
            if source.starts_with("def foo()end") {
                assert!(found.is_empty());
            } else {
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn multiline_comment_parentheses_keep_lines_and_comments() {
        for source in [
            "def foo(\n)\n  __LINE__\nend\np __LINE__\n",
            "def foo( # keep\n)\nend\n",
            "def foo(\r\n)\r\nend\r\n",
        ] {
            let found = correction_offenses(&DefWithParentheses, source);
            assert_eq!(found.len(), 1, "{source}");
            // Detection remains complete, but deleting multiline parentheses can
            // move a body/signature/directive, so this individual edit is withheld.
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}
