//! `Lint/SafeNavigationWithEmpty` — safe `empty?` at the root of an if predicate.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SafeNavigationWithEmpty;
impl Cop for SafeNavigationWithEmpty {
    fn name(&self) -> &'static str {
        "Lint/SafeNavigationWithEmpty"
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
        let predicate = if let Some(n) = node.as_if_node() {
            n.predicate()
        } else if let Some(n) = node.as_unless_node() {
            n.predicate()
        } else {
            return;
        };
        // Prism represents pattern guards with If/Unless; parser's if_guard /
        // unless_guard are NOT on_if callbacks.
        if cx.parent().is_some_and(|n| n.as_in_node().is_some()) {
            return;
        }
        let Some(call) = predicate.as_call_node() else {
            return;
        };
        if !call.is_safe_navigation()
            || call.name().as_slice() != b"empty?"
            || !ast::call_arguments(&call).is_empty()
            || call.block().is_some()
        {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        if receiver.as_call_node().is_some_and(|c| {
            c.is_safe_navigation() && !c.block().is_some_and(|b| b.as_block_node().is_some())
        }) {
            return;
        }
        let start = predicate.location().start_offset();
        let end = ast::source_end(&predicate);
        let ast_end = end;
        let (line, col) = file.reference_line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: "Avoid calling `empty?` with the safe navigation operator in conditionals."
                .into(),
            correction: cx
                .corrections_requested()
                .then(|| {
                    let receiver = std::str::from_utf8(receiver.location().as_slice()).ok()?;
                    let loc = predicate.location();
                    Some(crate::offense::Correction {
                        range: loc.start_offset()..ast_end,
                        replacement: format!("{receiver} && {receiver}.empty?"),
                    })
                })
                .flatten(),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/safe_navigation_with_empty_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/SafeNavigationWithEmpty")
            .collect()
    }
    #[test]
    fn fixed_root_matcher_and_no_general_condition_search() {
        for s in [
            "foo = build_collection\nreturn if foo&.empty?",
            "return if @foo&.empty?",
            "return if FOO&.empty?",
            "if foo&.empty?; end",
            "foo&.empty? ? a : b",
            "if a; elsif foo&.empty?; end",
            "if (foo&.bar)&.empty?; end",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        for s in [
            "if foo&.empty?(1); end",
            "if foo&.empty?(&b); end",
            "if foo&.empty? { it }; end",
            "if (foo&.empty?); end",
            "if !foo&.empty?; end",
            "if foo&.empty? && bar; end",
            "while foo&.empty?; end",
            "case v; in x if foo&.empty?; end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn canonical_correction_is_guarded_and_not_a_noop() {
        for keyword in ["if", "unless"] {
            let before = format!("if false\n  {keyword} probe&.empty?\n  end\nend\n");
            let found =
                crate::cops::testing::correction_offenses(&SafeNavigationWithEmpty, &before);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_some(), "{found:?}");
            let after = crate::corrections::apply(before.as_bytes(), &found).unwrap();
            assert_eq!(after.count, 1);
            assert_eq!(
                after.bytes,
                before
                    .replace("probe&.empty?", "probe && probe.empty?")
                    .as_bytes()
            );
            assert!(offenses(std::str::from_utf8(&after.bytes).unwrap()).is_empty());
        }
    }
}
