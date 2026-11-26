//! `Lint/IdentityComparison` — object_id comparisons instead of identity queries.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct IdentityComparison;
impl Cop for IdentityComparison {
    fn name(&self) -> &'static str {
        "Lint/IdentityComparison"
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
        let op = call.name().as_slice();
        if call.is_safe_navigation() || !matches!(op, b"==" | b"!=") {
            return;
        }
        let Some(lhs) = call.receiver() else { return };
        let arguments = ast::call_arguments(&call);
        let [rhs] = arguments.as_slice() else { return };
        if !object_id(&lhs) || !object_id(rhs) {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.reference_line_col(start);
        let bang = if op == b"!=" { "!" } else { "" };
        let op = std::str::from_utf8(op).unwrap();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: format!("Use `{bang}equal?` instead of `{op}` when comparing `object_id`."),
            correction: cx
                .corrections_requested()
                .then(|| {
                    let text = |n: &Node<'_>| -> Option<String> {
                        let receiver = n.as_call_node()?.receiver()?;
                        std::str::from_utf8(receiver.location().as_slice())
                            .ok()
                            .map(str::to_owned)
                    };
                    Some(crate::offense::Correction {
                        range: start..end,
                        replacement: format!("{bang}{}.equal?({})", text(&lhs)?, text(rhs)?),
                    })
                })
                .flatten(),
        });
    }
}

// RuboCop's send matcher requires an explicit receiver, no arguments and no
// attached block on either operand. Parenthesized begin/csend are not send.
fn object_id(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        !call.is_safe_navigation()
            && call.name().as_slice() == b"object_id"
            && call.receiver().is_some()
            && call.arguments().is_none()
            && call.block().is_none()
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/identity_comparison_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/IdentityComparison")
            .collect()
    }
    #[test]
    fn pinned_comparison_offenses() {
        for (op, bang) in [("==", ""), ("!=", "!")] {
            let source = format!("nil\n  foo.object_id {op} bar.object_id\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("Use `{bang}equal?` instead of `{op}` when comparing `object_id`.")
            );
        }
    }
    #[test]
    fn canonical_rewrite_only_in_a_closed_inactive_branch() {
        for (op, bang) in [("==", ""), ("!=", "!")] {
            crate::cops::testing::assert_correction(
                &super::IdentityComparison,
                &format!("if false\n  foo.object_id {op} bar.object_id\nend\n"),
                &format!("if false\n  {bang}foo.equal?(bar)\nend\n"),
            );
        }
    }
    #[test]
    fn original_byte_reference_matrix() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/identity_comparison.json"
        ))
        .unwrap();
        let mut mismatches = Vec::new();
        for row in rows {
            let bytes: Vec<u8> = crate::fixture::value(&row["bytes"]);
            let Ok(source) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let mut found: Vec<_> = offenses(source)
                .into_iter()
                .map(|o| (o.line, o.col, o.end_line, o.message))
                .collect();
            found.sort();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            if found != expected {
                mismatches.push(format!("{source:?}: {found:?} != {expected:?}"));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
    #[test]
    fn pinned_no_offense_examples() {
        for source in [
            "foo.object_id == bar.do_something",
            "foo.object_id != bar.do_something",
            "foo.do_something == bar.object_id",
            "foo.do_something != bar.object_id",
            "foo.equal?(bar)",
            "!foo.equal?(bar)",
            "object_id == bar.object_id",
            "object_id != bar.object_id",
            "foo.object_id == object_id",
            "foo.object_id != object_id",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
