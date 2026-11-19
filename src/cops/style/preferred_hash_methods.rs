//! `Style/PreferredHashMethods` — fixed short names; unsafe, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct PreferredHashMethods;
impl Cop for PreferredHashMethods {
    fn name(&self) -> &'static str {
        "Style/PreferredHashMethods"
    }
    fn unsafe_fixes(&self) -> bool {
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
        let (current, prefer) = match call.name().as_slice() {
            b"has_key?" => ("has_key?", "key?"),
            b"has_value?" => ("has_value?", "value?"),
            _ => return,
        };
        if ast::call_arguments(&call).len() != 1 {
            return;
        }
        let Some(loc) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `Hash#{prefer}` instead of `Hash#{current}`."),
            line,
            col,
            end_line: file.line_of(loc.end_offset() - 1),
            correction: cx
                .corrections_requested()
                .then(|| crate::offense::Correction {
                    range: loc.start_offset()..loc.end_offset(),
                    replacement: prefer.into(),
                }),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop preferred_hash_methods_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::pipeline::lint_str;
    #[test]
    fn keyword_positional_splat_and_block_pass_arguments_are_counted_not_rewritten() {
        for arg in [
            ":key",
            "dynamic",
            "'string'",
            "key: value",
            "{key: value}",
            "*keys",
            "**keys",
            "&block",
            "&nil",
        ] {
            for receiver in ["custom.", "custom&.", ""] {
                let s = format!("{receiver}has_key?({arg})");
                let o: Vec<_> = lint_str(&s)
                    .into_iter()
                    .filter(|o| o.cop == "Style/PreferredHashMethods")
                    .collect();
                assert_eq!(o.len(), 1, "{s}");
                assert!(o[0].correction.is_none());
            }
        }
        for s in [
            "custom.has_key?(:a) { effect }",
            "has_value?([1]) do\n effect\nend",
        ] {
            let o: Vec<_> = lint_str(s)
                .into_iter()
                .filter(|o| o.cop == "Style/PreferredHashMethods")
                .collect();
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(o[0].end_line, 1);
        }
    }
    #[test]
    fn exact_short_selector_and_argument_policy() {
        for (s, col, current, prefer) in [
            ("o.has_key?(o)", 3, "has_key?", "key?"),
            ("o&.has_value?(o)", 4, "has_value?", "value?"),
            ("has_key?(o)", 1, "has_key?", "key?"),
        ] {
            let o: Vec<_> = lint_str(s)
                .into_iter()
                .filter(|o| o.cop == "Style/PreferredHashMethods")
                .collect();
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, col, 1));
            assert_eq!(
                o[0].message,
                format!("Use `Hash#{prefer}` instead of `Hash#{current}`.")
            );
            assert!(o[0].correction.is_none());
        }
        for s in [
            "o.has_key?",
            "o.has_value?()",
            "o.has_key?(a,b)",
            "o.has_key?(a,&b)",
            "o.key?(a)",
            "o.value?(a)",
        ] {
            assert!(
                !lint_str(s)
                    .iter()
                    .any(|o| o.cop == "Style/PreferredHashMethods"),
                "{s}"
            );
        }
    }
}
