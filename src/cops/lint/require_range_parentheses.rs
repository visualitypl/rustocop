//! `Lint/RequireRangeParentheses` — ambiguous multiline range endpoints.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct RequireRangeParentheses;
impl Cop for RequireRangeParentheses {
    fn name(&self) -> &'static str {
        "Lint/RequireRangeParentheses"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let (left, right, operator) = if let Some(range) = node.as_range_node() {
            (range.left(), range.right(), range.operator_loc())
        } else if let Some(range) = node.as_flip_flop_node() {
            (range.left(), range.right(), range.operator_loc())
        } else {
            return;
        };
        if super::flip_flop::conditional_range(node, cx) {
            return;
        }
        let (Some(left), Some(right)) = (left, right) else {
            return;
        };
        if file.line_of(operator.start_offset()) == file.line_of(right.location().start_offset())
            || begin_parent(cx)
        {
            return;
        }
        let start = left.location().start_offset();
        let text = file.match_pattern_source(&file.bytes[start..left.location().end_offset()]);
        // Preserve invalid binary-source bytes explicitly, never U+FFFD.
        let prefix = text
            .utf8_chunks()
            .map(|chunk| {
                let mut text = chunk.valid().to_owned();
                for byte in chunk.invalid() {
                    text.push_str(&format!("\\x{byte:02X}"));
                }
                text
            })
            .collect::<String>()
            .replace("\r\n", "\n");
        let op = std::str::from_utf8(operator.as_slice()).unwrap_or("..");
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            correction: None,
            line,
            col,
            end_line: file.line_of(node.location().end_offset().saturating_sub(1)),
            message: format!(
                "Wrap the range literal `{prefix}{op}` in parentheses to avoid confusion with an endless range."
            ),
        });
    }
}
// Parser's `begin` is grouping parentheses or a multi-statement sequence,
// never explicit begin/end (`kwbegin`). Interpolation bodies also wrap even a
// single expression in `begin`. Prism retains Statements for all these bodies.
fn begin_parent(cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors();
    if let Some(parent) = ancestors.next() {
        if let Some(statements) = parent.as_statements_node() {
            let outer = ancestors.next();
            return outer.is_some_and(|n| {
                n.as_parentheses_node().is_some() || n.as_embedded_statements_node().is_some()
            }) || (statements.body().len() > 1
                && !outer.is_some_and(|n| {
                    n.as_begin_node().is_some_and(|b| {
                        b.begin_keyword_loc().is_some()
                            && b.rescue_clause().is_none()
                            && b.ensure_clause().is_none()
                    })
                }));
        }
        return parent.as_parentheses_node().is_some();
    }
    false
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/require_range_parentheses_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/RequireRangeParentheses")
            .collect()
    }
    #[test]
    fn pinned_multiline_ranges_and_when_clause() {
        for (s, line, col, text) in [
            ("42..\ndo_something\n", 1, 1, "42.."),
            ("42...\ndo_something\n", 1, 1, "42..."),
            (
                "case condition\nwhen 42..\n do_something\nend\n",
                2,
                6,
                "42..",
            ),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(
                found[0].message,
                format!(
                    "Wrap the range literal `{text}` in parentheses to avoid confusion with an endless range."
                )
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn interpolation_begin_parents_but_not_nested_assignments() {
        for template in [
            "puts \"#{BODY}\"\n",
            "puts :\"#{BODY}\"\n",
            "puts /#{BODY}/\n",
            "puts `#{BODY}`\n",
            "puts <<TEXT\n#{BODY}\nTEXT\n",
        ] {
            for operator in ["..", "..."] {
                let range = format!("42{operator}\nvalue");
                let source = template.replace("BODY", &range);
                assert!(offenses(&source).is_empty(), "{source}");
                let source = template.replace("BODY", &format!("x = {range}"));
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                let offset = source.find("42").unwrap();
                let prefix = &source[..offset];
                assert_eq!(
                    (found[0].line, found[0].col),
                    (
                        prefix.bytes().filter(|&b| b == b'\n').count() as u32 + 1,
                        prefix.rsplit('\n').next().unwrap().len() as u32 + 1
                    ),
                    "{source}"
                );
            }
        }
    }
    #[test]
    fn pinned_lookalikes_and_parser_parent_boundaries() {
        for s in [
            "(42..\ndo_something)\n",
            "42..\n",
            "..42\n",
            "42..nil\n",
            "nil..42\n",
            "42..do_something\n",
            "foo(\n bar)..baz(quux)\n",
            "if 42..\nvalue; end\n",
            "(nil; 42..\nvalue)\n",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        assert_eq!(offenses("nil\n42..\nvalue\n").len(), 0); // multi-statement parser begin parent
        assert_eq!(offenses("x = 42..\nvalue\n").len(), 1);
    }
}
