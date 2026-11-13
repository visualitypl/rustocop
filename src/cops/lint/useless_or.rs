//! `Lint/UselessOr` — unreachable fallbacks after fixed truthy selectors.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct UselessOr;
impl Cop for UselessOr {
    fn name(&self) -> &'static str {
        "Lint/UselessOr"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_or_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(or) = node.as_or_node() else { return };
        let (owner, truthy) = if truthy(&or.left()) {
            (node, or.left())
        } else if truthy(&or.right()) {
            let mut ancestors = cx.ancestors();
            let mut parent = ancestors.next();
            // Statements are transparent; a single explicit grouping is one begin.
            if parent.is_some_and(|p| p.as_statements_node().is_some()) {
                parent = ancestors.next();
            }
            if parent.is_some_and(|p| p.as_parentheses_node().is_some()) {
                parent = ancestors.next();
                if parent.is_some_and(|p| p.as_statements_node().is_some()) {
                    parent = ancestors.next();
                }
            }
            let Some(parent) = parent.filter(|p| p.as_or_node().is_some()) else {
                return;
            };
            (parent, or.right())
        } else {
            return;
        };
        let owner = owner.as_or_node().unwrap();
        let start = owner.operator_loc().start_offset();
        let rhs = owner.right();
        let end = rhs.location().end_offset();
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        let text = |n: &Node<'_>| {
            let loc = n.location();
            let bytes = loc.as_slice();
            source_text(bytes, file)
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: None,
            message: format!(
                "`{}` will never evaluate because `{}` always returns a truthy value.",
                text(&rhs),
                text(&truthy)
            ),
        });
    }
}
pub(super) fn source_text(bytes: &[u8], file: &SourceFile) -> String {
    file.match_pattern_source(bytes)
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().to_owned();
            for b in chunk.invalid() {
                text.push_str(&format!("\\x{b:02X}"));
            }
            text
        })
        .collect::<String>()
        .replace("\r\n", "\n")
}
fn truthy(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        !call.is_safe_navigation()
            && call.arguments().is_none_or(|a| a.arguments().is_empty())
            && call.block().is_none()
            && [
                b"to_a".as_slice(),
                b"to_c",
                b"to_d",
                b"to_i",
                b"to_f",
                b"to_h",
                b"to_r",
                b"to_s",
                b"to_sym",
                b"intern",
                b"inspect",
                b"hash",
                b"object_id",
                b"__id__",
            ]
            .contains(&call.name().as_slice())
    })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_or_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessOr")
            .collect()
    }
    #[test]
    fn complete_rhs_ranges_and_single_begin_parent_projection() {
        let found = offenses("x.to_s || fallback do\n work\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 8, 3));
        assert_eq!(
            found[0].message,
            "`fallback do\n work\nend` will never evaluate because `x.to_s` always returns a truthy value."
        );
        assert!(offenses("begin; a || x.to_s; end || fallback").is_empty());
        assert!(offenses("((a || x.to_s)) || fallback").is_empty());
        assert_eq!(offenses("(a; b || x.to_s) || fallback").len(), 1);
    }
    #[test]
    fn pinned_selectors_and_chains() {
        for method in [
            "to_a",
            "to_c",
            "to_d",
            "to_i",
            "to_f",
            "to_h",
            "to_r",
            "to_s",
            "to_sym",
            "intern",
            "inspect",
            "hash",
            "object_id",
            "__id__",
        ] {
            for (source, col) in [
                (format!("x.{method} || fallback"), method.len() + 4),
                (format!("x.{method} or fallback"), method.len() + 4),
                (
                    format!("x.{method} || fallback || other_fallback"),
                    method.len() + 4,
                ),
                (format!("foo || x.{method} || fallback"), method.len() + 11),
                (
                    format!("(foo || x.{method}) || fallback"),
                    method.len() + 13,
                ),
            ] {
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, col as u32), "{source}");
                assert_eq!(
                    found[0].message,
                    format!(
                        "`fallback` will never evaluate because `x.{method}` always returns a truthy value."
                    )
                );
                assert!(found[0].correction.is_none());
            }
            for source in [
                format!("(foo || x.{method}) && operand"),
                format!("foo || x.{method}"),
                format!("x&.{method} || fallback"),
                format!("x.{method}"),
                format!("x.{method}(1) || fallback"),
                format!("x.{method} {{ nil }} || fallback"),
                format!("(x.{method}) || fallback"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        assert!(offenses("x.foo || fallback").is_empty());
    }
}
