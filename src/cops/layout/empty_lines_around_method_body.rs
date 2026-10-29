//! `Layout/EmptyLinesAroundMethodBody` — no padding inside method bodies.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct EmptyLinesAroundMethodBody;
impl Cop for EmptyLinesAroundMethodBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundMethodBody"
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
        let Some(method) = node.as_def_node() else {
            return;
        };
        if let Some(equal) = method.equal_loc() {
            let first = file.line_of(equal.start_offset());
            if method
                .body()
                .is_some_and(|body| file.line_of(body.location().start_offset()) > first + 1)
            {
                super::body_padding::report(
                    self,
                    "method",
                    (first + 1, "beginning"),
                    cx,
                    file,
                    out,
                );
            }
            return;
        }
        let header = method
            .rparen_loc()
            .or_else(|| method.parameters().map(|p| p.location()));
        let first = header.map_or_else(
            || file.line_of(node.location().start_offset()),
            |loc| file.last_line_of(loc),
        );
        super::body_padding::check(
            self,
            "method",
            (first, file.last_line_of(node.location())),
            cx,
            file,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's layout/empty_lines_around_method_body_spec.rb at
    //! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == EmptyLinesAroundMethodBody.name())
            .collect()
    }
    #[test]
    fn pinned_multiline_parameters_and_empty_parentheses() {
        for header in [
            "def some_method(\n  arg\n)",
            "def some_method(\n\n)",
            "def some_method arg,\n  other",
        ] {
            let before = format!("{header}\n\n  do_something\nend\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col),
                (header.lines().count() as u32 + 1, 1)
            );
            assert_correction(
                &EmptyLinesAroundMethodBody,
                &before,
                &format!("{header}\n  do_something\nend\n"),
            );
        }
    }

    #[test]
    fn pinned_method_no_offense_lookalikes() {
        for source in [
            "",
            "def foo; end\n\nother\n",
            "def foo; work; end\n\nother\n",
            "def foo(bar,\n  baz); end\n",
            "def foo\n  \n  work\nend\n",
            "def foo = quux\n",
            "def foo(bar,\n  baz) = quux\n",
            "def foo(bar, baz) =\n  quux\n",
            "def foo(bar, baz) =\n  # comment\n  quux\n",
            "def foo\n  # comment\n\n  work\n\n  # tail\nend\n",
            "def foo\n  work\nrescue\n\n  recover\nensure\n\n  cleanup\nend\n",
            "def foo\n  begin\n\n    work\n\n  end\nend\n",
            "def foo\n  work do\n\n    stuff\n\n  end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_endless_method_padding() {
        for header in ["def foo(bar, baz) =", "def self.foo(bar,\n  baz) ="] {
            for comment in ["", "  # comment\n"] {
                let before = format!("{header}\n\n{comment}  quux\n");
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (header.lines().count() as u32 + 1, 1)
                );
                assert_eq!(
                    found[0].message,
                    "Extra empty line detected at method body beginning."
                );
                assert_correction(
                    &EmptyLinesAroundMethodBody,
                    &before,
                    &format!("{header}\n{comment}  quux\n"),
                );
            }
        }
    }

    #[test]
    fn endless_crlf_corrections_and_safety_withholding() {
        let before = "def self.foo =\r\n\r\n  # é\r\n  quux\r\n";
        let found = crate::cops::testing::correction_offenses(&EmptyLinesAroundMethodBody, before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 1));
        assert_eq!(
            &before.as_bytes()[found[0].correction.as_ref().unwrap().range.clone()],
            b"\r\n"
        );
        assert_correction(
            &EmptyLinesAroundMethodBody,
            before,
            "def self.foo =\r\n  # é\r\n  quux\r\n",
        );
        for source in [
            "def foo =\n\n  __LINE__\n",
            "def foo =\n\n  quux\np __LINE__\n",
            "def foo =\n\n  # rubocop:disable Lint/Debugger\n  quux\n",
        ] {
            assert_eq!(offenses(source).len(), 1);
            assert_correction(&EmptyLinesAroundMethodBody, source, source);
        }
    }

    #[test]
    fn pinned_method_padding_positions_and_correction() {
        for name in ["some_method", "Test.some_method", "self.some_method"] {
            let before = format!("def {name}\n\n  do_something\n\nend\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 2);
            for (o, line, edge) in [(&found[0], 2, "beginning"), (&found[1], 4, "end")] {
                assert_eq!((o.line, o.col, o.end_line), (line, 1, line));
                assert_eq!(
                    o.message,
                    format!("Extra empty line detected at method body {edge}.")
                );
                assert!(o.correction.is_none());
            }
            assert_correction(
                &EmptyLinesAroundMethodBody,
                &before,
                &format!("def {name}\n  do_something\nend\n"),
            );
        }
    }
}
