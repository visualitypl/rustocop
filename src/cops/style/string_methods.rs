//! `Style/StringMethods` — opt-in, fixed `intern` -> `to_sym`.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct StringMethods;
impl Cop for StringMethods {
    fn name(&self) -> &'static str {
        "Style/StringMethods"
    }
    fn default_enabled(&self) -> bool {
        false
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
        if call.name().as_slice() != b"intern" {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let correction = cx
            .corrections_requested()
            .then(|| crate::offense::Correction {
                range: selector.start_offset()..selector.end_offset(),
                replacement: "to_sym".into(),
            });
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Prefer `to_sym` over `intern`.".into(),
            line,
            col,
            end_line: file.line_of(selector.end_offset().saturating_sub(1)),
            correction,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop string_methods_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). Fixed intern preference only.
    use super::*;
    use crate::cops::testing::{assert_correction, correction_offenses};
    #[test]
    fn pinned_positions_and_messages() {
        for (s, col) in [("'something'.intern", 13), ("something&.intern", 12)] {
            let o = correction_offenses(&StringMethods, &format!("# header\n{s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col, 2));
            assert_eq!(o[0].message, "Prefer `to_sym` over `intern`.");
        }
    }
    #[test]
    fn canonical_string_symbol_and_safe_navigation_corrections() {
        for (s, after) in [
            ("'something'.intern", "'something'.to_sym"),
            ("'name'&.intern()", "'name'&.to_sym()"),
            (":name.intern", ":name.to_sym"),
            ("%s(name).intern", "%s(name).to_sym"),
            ("\"a\\0b\".intern", "\"a\\0b\".to_sym"),
            ("?x.intern", "?x.to_sym"),
        ] {
            assert_correction(&StringMethods, s, after);
        }
    }
    #[test]
    fn no_offense_lookalikes_and_opt_in_default() {
        for s in [
            "'x'.to_sym",
            "object.interned",
            "intern = 1; intern",
            "def intern; end",
            "# 'x'.intern\n\"intern\"",
        ] {
            assert!(correction_offenses(&StringMethods, s).is_empty(), "{s}");
        }
        assert!(!StringMethods.default_enabled());
        assert!(
            crate::pipeline::lint_str("'x'.intern")
                .iter()
                .all(|o| o.cop != StringMethods.name())
        );
    }
}
