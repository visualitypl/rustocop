//! `Style/TrailingBodyOnClass` — start multiline class bodies below the header.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct TrailingBodyOnClass;
impl Cop for TrailingBodyOnClass {
    fn name(&self) -> &'static str {
        "Style/TrailingBodyOnClass"
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
        let (body, header_end) = if let Some(class) = node.as_class_node() {
            (
                class.body(),
                class
                    .superclass()
                    .unwrap_or_else(|| class.constant_path())
                    .location()
                    .end_offset(),
            )
        } else if let Some(class) = node.as_singleton_class_node() {
            (class.body(), class.expression().location().end_offset())
        } else {
            return;
        };
        check_body(self, node, body, header_end, cx, file, out);
    }
}

/// Namespace and method cops share the pinned trailing-body matcher/corrector.
pub(super) fn check_body<'pr>(
    cop: &dyn Cop,
    node: &Node<'pr>,
    body: Option<Node<'pr>>,
    header_end: usize,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let Some(body) = body else { return };
    let single_statement = body
        .as_statements_node()
        .is_some_and(|s| s.body().len() == 1);
    let first = body
        .as_statements_node()
        .and_then(|s| s.body().iter().next())
        .unwrap_or(body);
    let offset = if let Some(begin) = first
        .as_begin_node()
        .filter(|b| b.begin_keyword_loc().is_none())
    {
        begin
            .statements()
            .map(|s| s.location().start_offset())
            .or_else(|| {
                begin
                    .rescue_clause()
                    .map(|r| r.keyword_loc().start_offset())
            })
            .or_else(|| {
                begin
                    .ensure_clause()
                    .map(|e| e.ensure_keyword_loc().start_offset())
            })
            .unwrap_or(first.location().start_offset())
    } else {
        first.location().start_offset()
    };
    // Detection uses the body start (including an opening parenthesis), not
    // first_part_of's highlight, which can start on a later physical line.
    let body_line = file.line_of(offset);
    if file.line_of(node.location().start_offset()) != body_line
        || file.last_line_of(node.location()) == body_line
    {
        return;
    }
    // RuboCop's :begin is either a multi-expression statement list or a sole
    // parenthesized group. Unwrap exactly one of those layers, not both.
    let inner = if single_statement {
        first
            .as_parentheses_node()
            .and_then(|p| p.body())
            .and_then(|b| b.as_statements_node())
            .and_then(|s| s.body().iter().next())
    } else {
        None
    };
    let highlight = inner.as_ref().unwrap_or(&first);
    let highlight_offset = inner
        .as_ref()
        .map_or(offset, |n| n.location().start_offset());
    let (line, col) = file.line_col(highlight_offset);
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message: format!(
            "Place the first line of {} body on its own line.",
            if node.as_def_node().is_some() {
                "a multi-line method definition's"
            } else if node.as_module_node().is_some() {
                "module"
            } else {
                "class"
            }
        ),
        line,
        col,
        end_line: file.line_of(
            super::trailing_method_end_statement::body_end_offset(highlight, file)
                .saturating_sub(1),
        ),
        correction: cx
            .corrections_requested()
            .then(|| rewrite(node, offset, header_end, cx, file))
            .flatten(),
    });
}

pub(super) fn newline(file: &SourceFile, line: u32) -> &'static str {
    if file.bytes[file.line_start(line)..file.line_start(line + 1)].ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn rewrite(
    node: &Node<'_>,
    offset: usize,
    header_end: usize,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let start = node.location().start_offset();
    if header_end > offset
        || !super::correction::method_newline_safe(node, cx, file, start..offset + 1)
    {
        return None;
    }
    let (line, col) = file.line_col(start);
    let prefix = &file.bytes[file.line_start(line)..start];
    let prefix = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    let is_method = node.as_def_node().is_some();
    let indent_prefix = if is_method {
        [b"private ".as_slice(), b"protected ", b"public "]
            .iter()
            .find_map(|v| prefix.strip_suffix(*v))
            .unwrap_or(prefix)
    } else {
        prefix
    };
    let standalone = indent_prefix.iter().all(|b| matches!(b, b' ' | b'\t'));
    // A semicolon-delimited preceding expression may stay in place for methods.
    // Other inline prefixes need a separate binding proof. Comments may only
    // move above a standalone declaration, including its visibility wrapper.
    if !standalone && !(is_method && prefix.trim_ascii_end().ends_with(b";")) {
        return None;
    }
    let declaration_start = start - (prefix.len() - indent_prefix.len());
    let gap = &file.bytes[header_end..offset];
    if !gap.iter().all(|b| matches!(b, b' ' | b'\t' | b';')) {
        return None;
    }
    let nl = newline(file, line);
    let mut edits = vec![Correction {
        range: offset..offset,
        replacement: format!("{nl}{}", " ".repeat(col as usize + 1)),
    }];
    // Remove only separator tokens, never semicolons inside header expressions.
    for (index, byte) in gap.iter().enumerate() {
        if *byte == b';' {
            edits.push(Correction {
                range: header_end + index..header_end + index + 1,
                replacement: String::new(),
            });
        }
    }
    let comments = file.comments();
    let index = comments.partition_point(|c| c.start < offset);
    if let Some(comment) = comments
        .get(index)
        .filter(|c| file.line_of(c.start) == line)
    {
        if !standalone {
            return None;
        }
        let comment =
            comment.start..comment.end - usize::from(file.bytes[comment.clone()].ends_with(b"\r"));
        let text = std::str::from_utf8(&file.bytes[comment.clone()]).ok()?;
        if !super::correction::comment_relocation_safe(text.as_bytes()) {
            return None;
        }
        edits.push(Correction {
            range: declaration_start..declaration_start,
            replacement: format!(
                "{text}{nl}{}",
                " ".repeat(file.line_col(declaration_start).1 as usize - 1)
            ),
        });
        edits.push(Correction {
            range: comment,
            replacement: String::new(),
        });
    }
    super::correction::combine(file, edits)
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/trailing_body_on_class_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TrailingBodyOnClass")
            .collect()
    }
    #[test]
    fn pinned_positions_messages_and_corrections() {
        for (before, after, line, col, end_line) in [
            (
                "class Foo; body\nend\n",
                "class Foo \n  body\nend\n",
                1,
                12,
                1,
            ),
            (
                "class Foo body\nend\n",
                "class Foo \n  body\nend\n",
                1,
                11,
                1,
            ),
            (
                "class Bar; def bar; end\nend\n",
                "class Bar \n  def bar; end\nend\n",
                1,
                12,
                1,
            ),
            (
                "class Bar def bar; end\nend\n",
                "class Bar \n  def bar; end\nend\n",
                1,
                11,
                1,
            ),
            (
                "class << self; body\nend\n",
                "class << self \n  body\nend\n",
                1,
                16,
                1,
            ),
            (
                "class << self; def bar; end\nend\n",
                "class << self \n  def bar; end\nend\n",
                1,
                16,
                1,
            ),
            (
                "class Foo; body\n  def bar\n    qux\n  end\nend\n",
                "class Foo \n  body\n  def bar\n    qux\n  end\nend\n",
                1,
                12,
                1,
            ),
            (
                "class BarQux; foo # comment\nend\n",
                "# comment\nclass BarQux \n  foo \nend\n",
                1,
                15,
                1,
            ),
            (
                "\n  class Foo; body \n  end\n",
                "\n  class Foo \n    body \n  end\n",
                2,
                14,
                2,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end_line)
            );
            assert_eq!(
                found[0].message,
                "Place the first line of class body on its own line."
            );
            assert_correction(&TrailingBodyOnClass, before, after);
        }
    }
    #[test]
    fn implicit_rescue_and_ensure_ranges_exclude_the_namespace_keyword() {
        for (source, line, col, last) in [
            ("class C; foo\nrescue\nbar\nelse\nbaz\nend", 1, 10, 5),
            ("class C; foo\nensure\nbar\nend", 1, 10, 3),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1);
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, last)
            );
        }
        assert!(offenses("class C\n foo\nrescue\n bar\nend").is_empty());
    }
    #[test]
    fn qualified_superclass_receiver_headers_and_comments_are_token_bounded() {
        for header in [
            "class A::B",
            "class ::C",
            "class C < Base",
            "class C < (side_effect; Base)",
            "class << (side_effect; self)",
        ] {
            let before = format!("{header}; body\nend\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, header.len() as u32 + 3));
            assert_correction(
                &TrailingBodyOnClass,
                &before,
                &format!("{header} \n  body\nend\n"),
            );
        }
        assert_correction(
            &TrailingBodyOnClass,
            "class C; \"a\nb\"\nend\n",
            "class C \n  \"a\nb\"\nend\n",
        );
        assert_correction(
            &TrailingBodyOnClass,
            "class C; foo\nrescue\nbar\nend\n",
            "class C \n  foo\nrescue\nbar\nend\n",
        );
    }
    #[test]
    fn shebang_comments_are_not_relocated() {
        let module = crate::cops::style::TrailingBodyOnModule;
        let method = crate::cops::style::TrailingBodyOnMethodDefinition;
        for (cop, header) in [
            (&TrailingBodyOnClass as &dyn Cop, "class C"),
            (&TrailingBodyOnClass as &dyn Cop, "class << self"),
            (&module as &dyn Cop, "module M"),
            (&method as &dyn Cop, "def f"),
        ] {
            for prefix in ["", "\u{feff}", "  ", "# docs\n"] {
                let source = format!("{prefix}{header}; 1 #!/usr/bin/ruby -n\nend\n");
                let found = crate::cops::testing::correction_offenses(cop, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
    }
    #[test]
    fn parenthesized_namespace_ranges_and_safe_corrections() {
        let module = crate::cops::style::TrailingBodyOnModule;
        for (cop, header) in [
            (&TrailingBodyOnClass as &dyn Cop, "class C"),
            (&TrailingBodyOnClass as &dyn Cop, "class << self"),
            (&module as &dyn Cop, "module M"),
        ] {
            for (body, line, col, last) in [
                ("(1)", 1, header.len() as u32 + 3, 1),
                ("(1\n)", 1, header.len() as u32 + 3, 1),
                ("(\n1\n)", 2, 1, 2),
                ("((1))", 1, header.len() as u32 + 3, 1),
                ("(1; 2)", 1, header.len() as u32 + 3, 1),
                ("(begin\n1\nend)", 1, header.len() as u32 + 3, 3),
                // A namespace with multiple expressions already has an implicit
                // begin: first_part_of selects the whole first parenthesized group.
                ("(1\n); 2", 1, header.len() as u32 + 2, 2),
            ] {
                let source = format!("{header};{body}\nend\n");
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == cop.name())
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (line, col, last),
                    "{source}"
                );
                // Move the whole group, not the inner highlight. Removing the
                // separator and inserting inside it can create `class C(`.
                let corrected = format!("{header}\n  {body}\nend\n");
                assert_correction(cop, &source, &corrected);
                assert!(lint_str(&corrected).iter().all(|o| o.cop != cop.name()));
            }
            for source in [
                format!("{header}\n(1)\nend\n"),
                format!("{header};(1);end\n"),
            ] {
                assert!(lint_str(&source).iter().all(|o| o.cop != cop.name()));
            }
        }
    }
    #[test]
    fn accepts_regular_empty_single_line_and_inheritance() {
        for source in [
            "class Foo\n  def no_op; end\nend",
            "class Foo < Bar\nend",
            "class Foo; body; end",
            "class << self; body; end",
            "class Foo\nend",
            "class Foo < (Class.new do\nend)\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
