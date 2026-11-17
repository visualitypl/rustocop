//! `Style/Strip` — zero-argument opposite trim chains.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct Strip;
impl Cop for Strip {
    fn name(&self) -> &'static str {
        "Style/Strip"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let method = call.name().as_slice();
        if !matches!(method, b"lstrip" | b"rstrip")
            || !crate::cops::ast::call_arguments(&call).is_empty()
        {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let Some(first) = receiver.as_call_node() else {
            return;
        };
        if !matches!(
            (first.name().as_slice(), method),
            (b"lstrip", b"rstrip") | (b"rstrip", b"lstrip")
        ) || !crate::cops::ast::call_arguments(&first).is_empty()
            || first.block().is_some()
        {
            return;
        }
        let Some(selector) = first.message_loc() else {
            return;
        };
        let start = selector.start_offset();
        let end = crate::cops::ast::source_end(node);
        let range = start..end;
        let correction = cx
            .corrections_requested()
            .then(|| crate::offense::Correction {
                range: range.clone(),
                replacement: "strip".into(),
            });
        let (line, col) = file.line_col(start);
        let current = crate::offense::single_line(&file.bytes[range]);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `strip` instead of `{current}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop strip_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Unknown receivers retain offenses.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == Strip.name())
            .collect()
    }
    #[test]
    fn canonical_primitive_strings_each_order_and_navigation() {
        for first in ["lstrip", "rstrip"] {
            let last = if first == "lstrip" {
                "rstrip"
            } else {
                "lstrip"
            };
            for dot in [".", "&."] {
                for second in [".", "&."] {
                    for args in ["", "()"] {
                        assert_correction(
                            &Strip,
                            &format!("\" \\0é\\t \"{dot}{first}{args}{second}{last}{args}"),
                            &format!("\" \\0é\\t \"{dot}strip"),
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn arity_block_ownership_and_parenthesis_lookalikes() {
        for s in [
            "str.strip",
            "str.lstrip",
            "str.rstrip.rstrip",
            "str.lstrip.lstrip",
            "str.lstrip!.rstrip",
            "str.lstrip.rstrip!",
            "str.lstrip(1).rstrip",
            "str.lstrip.rstrip(1)",
            "str.lstrip(&b).rstrip",
            "str.lstrip.rstrip(&b)",
            "str.lstrip { value }.rstrip",
            "(str.lstrip).rstrip",
            "str.lstrip.to_s.rstrip",
            "def lstrip; end",
            "# str.lstrip.rstrip\n'lstrip.rstrip'",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        let o = offenses("str.lstrip.rstrip { ignored }");
        assert_eq!(o[0].message, "Use `strip` instead of `lstrip.rstrip`.");
    }
    #[test]
    fn chained_pairs_report_independently_without_dropping_sends() {
        let o = offenses("str.lstrip.rstrip.lstrip");
        assert_eq!(o.len(), 2);
        assert_eq!((o[0].col, o[1].col), (5, 12));
        assert_correction(&Strip, "' x '.lstrip.rstrip.lstrip", "' x '.strip.lstrip");
    }
}
