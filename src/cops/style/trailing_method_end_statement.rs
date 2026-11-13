//! `Style/TrailingMethodEndStatement` — multiline methods end on their own line.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct TrailingMethodEndStatement;
impl Cop for TrailingMethodEndStatement {
    fn name(&self) -> &'static str {
        "Style/TrailingMethodEndStatement"
    }
    fn safe_fixes(&self) -> bool {
        true
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
        if def.equal_loc().is_some() {
            return;
        }
        let (Some(body), Some(end)) = (def.body(), def.end_keyword_loc()) else {
            return;
        };
        let (line, col) = file.line_col(end.start_offset());
        if file.line_of(def.location().start_offset()) == line
            || file.line_of(body_end_offset(&body, file).saturating_sub(1)) != line
        {
            return;
        }
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Place the end statement of a multi-line method on its own line.".into(),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| {
                    let offset = end.start_offset();
                    if !super::correction::method_newline_safe(
                        node,
                        cx,
                        file,
                        offset..end.end_offset(),
                    ) {
                        return None;
                    }
                    let indent = file.line_col(def.def_keyword_loc().start_offset()).1 - 1;
                    Some(Correction {
                        range: offset..offset,
                        replacement: format!(
                            "{}{}",
                            super::trailing_body_on_class::newline(file, line),
                            " ".repeat(indent as usize)
                        ),
                    })
                })
                .flatten(),
        });
    }
}

// Implicit rescue/ensure wrappers include the enclosing method's end in Prism,
// unlike RuboCop's rescue/ensure expression. Explicit begin/end keeps its end.
pub(super) fn body_end_offset(body: &Node<'_>, file: &SourceFile) -> usize {
    let Some(begin) = body
        .as_begin_node()
        .filter(|b| b.begin_keyword_loc().is_none())
    else {
        return body.location().end_offset();
    };
    let end = if let Some(ensure) = begin.ensure_clause() {
        return ensure.statements().map_or_else(
            || ensure.ensure_keyword_loc().end_offset(),
            |s| s.location().end_offset(),
        );
    } else if let Some(otherwise) = begin.else_clause() {
        return otherwise.statements().map_or_else(
            || otherwise.else_keyword_loc().end_offset(),
            |s| s.location().end_offset(),
        );
    } else if let Some(mut rescue) = begin.rescue_clause() {
        while let Some(next) = rescue.subsequent() {
            rescue = next
        }
        if rescue.statements().is_some() {
            return rescue.location().end_offset();
        }
        rescue.location().end_offset()
    } else {
        return body.location().end_offset();
    };
    let Some(close) = begin.end_keyword_loc() else {
        return end;
    };
    // RuboCop includes separators after empty rescue (not else/ensure) bodies. Only
    // inspect this local trailing gap; comments are indexed by the main parse.
    let mut last = end;
    let comments = file.comments();
    let mut index = comments.partition_point(|c| c.end <= end);
    let mut cursor = end;
    while cursor < close.start_offset() {
        if let Some(comment) = comments.get(index).filter(|c| c.start == cursor) {
            cursor = comment.end;
            index += 1;
        } else {
            match file.bytes[cursor] {
                b';' => last = cursor + 1,
                b' ' | b'\t' | b'\r' | b'\n' => {}
                _ => return end,
            }
            cursor += 1;
        }
    }
    last
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/trailing_method_end_statement_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TrailingMethodEndStatement")
            .collect()
    }
    #[test]
    fn pinned_positions_messages_and_corrections() {
        for (before, after, line, col) in [
            (
                "def some_method\nfoo; end\n",
                "def some_method\nfoo; \nend\n",
                2,
                6,
            ),
            (
                "def a\n  b\n{ foo: bar }; end\n",
                "def a\n  b\n{ foo: bar }; \nend\n",
                3,
                15,
            ),
            (
                "def c\n  b = calculation\n  [b] end # because b\n",
                "def c\n  b = calculation\n  [b] \nend # because b\n",
                3,
                7,
            ),
            (
                "def d\n  block do\n    foo\n  end end\n",
                "def d\n  block do\n    foo\n  end \nend\n",
                4,
                7,
            ),
            (
                "class Foo\n  def some_method\n  foo; end\nend\n",
                "class Foo\n  def some_method\n  foo; \n  end\nend\n",
                3,
                8,
            ),
            (
                "def self.some_method\nfoo; end\n",
                "def self.some_method\nfoo; \nend\n",
                2,
                6,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(
                found[0].message,
                "Place the end statement of a multi-line method on its own line."
            );
            assert_correction(&TrailingMethodEndStatement, before, after);
        }
        assert_correction(
            &TrailingMethodEndStatement,
            "class Foo\n  def some_method\n    [] end\n  def another_method\n    {} end\nend\n",
            "class Foo\n  def some_method\n    [] \n  end\n  def another_method\n    {} \n  end\nend\n",
        );
    }
    #[test]
    fn setters_operators_parameters_visibility_literals_and_rescue_shapes() {
        for header in [
            "def f(x = 1, k: 2, &b)",
            "def self.f",
            "private def f",
            "def f=(x)",
            "def [](x)",
            "def self.[]=(x,y)",
            "def +(x)",
        ] {
            let before = format!("{header}\n  [1, 2] end\n");
            assert_eq!(offenses(&before).len(), 1);
            let indent = if header.starts_with("private") {
                "        "
            } else {
                ""
            };
            assert_correction(
                &TrailingMethodEndStatement,
                &before,
                &format!("{header}\n  [1, 2] \n{indent}end\n"),
            );
        }
        for (before, after) in [
            ("def f\n  \"a\nb\" end\n", "def f\n  \"a\nb\" \nend\n"),
            (
                "def f\n foo\nrescue\n bar end\n",
                "def f\n foo\nrescue\n bar \nend\n",
            ),
            (
                "def f\n foo\nensure\n bar end\n",
                "def f\n foo\nensure\n bar \nend\n",
            ),
            (
                "def f\n foo\nrescue\n; end\n",
                "def f\n foo\nrescue\n; \nend\n",
            ),
        ] {
            assert_correction(&TrailingMethodEndStatement, before, after);
        }
        for source in [
            "def f\n foo\nrescue\n bar\n; end",
            "def f\n foo\nensure\n; end",
            "def f\n foo\nrescue\n bar\nelse\n; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn uncertain_newlines_keep_each_offense_without_an_edit() {
        for source in [
            "def f\n <<~TXT; end\npayload\nTXT\n",
            "def f\n 1 end\np __LINE__\n",
            "def f\n 1 end # rubocop:disable Layout\n",
            "def f\n 1 end # rustocop:disable Layout\n",
            "def outer\n def inner\n 1 end\nend\n",
            "def f\n 1 end\np errors.map(&:full_message)\n",
            "def f\n 1 end\np Object.const_get(:DATA).pos\n__END__\nraw\n",
        ] {
            let found =
                crate::cops::testing::correction_offenses(&TrailingMethodEndStatement, source);
            assert!(!found.is_empty(), "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()), "{source}");
        }
    }
    #[test]
    fn accepts_empty_single_line_endless_and_separate_ends() {
        for source in [
            "def no_op; end",
            "def something; do_stuff; end",
            "def foo = bar",
            "def foo =\n  bar",
            "def foo\nend",
            "def foo\n  bar\nend",
            "def foo\n  bar\n  # end\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
