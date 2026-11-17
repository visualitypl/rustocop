//! `Style/ArrayJoin` — prefer explicit literal Array#join over Array#*.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ArrayJoin;
impl Cop for ArrayJoin {
    fn name(&self) -> &'static str {
        "Style/ArrayJoin"
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
        if !candidate(&call) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Favor `Array#join` over `Array#*`.".into(),
            line,
            col,
            end_line: file.last_line_of(selector),
            correction: cx
                .corrections_requested()
                .then(|| correction(&call))
                .flatten(),
        });
    }
}
fn candidate(call: &ruby_prism::CallNode<'_>) -> bool {
    !call.is_safe_navigation()
        && call.name().as_slice() == b"*"
        && call.receiver().is_some_and(|r| r.as_array_node().is_some())
        && matches!(ast::call_arguments(call).as_slice(), [a] if a.as_string_node().is_some())
}
/// RuboCop's corrector: `array.join(separator)`.
fn correction(call: &ruby_prism::CallNode<'_>) -> Option<Correction> {
    let receiver = call.receiver()?;
    let args = ast::call_arguments(call);
    let loc = call.location();
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement: format!(
            "{}.join({})",
            std::str::from_utf8(receiver.location().as_slice()).ok()?,
            std::str::from_utf8(args.first()?.location().as_slice()).ok()?
        ),
    })
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop array_join_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ArrayJoin.name())
            .collect()
    }
    #[test]
    fn pinned_operator_positions_and_messages() {
        for (s, col) in [
            ("%w(one two three) * \", \"", 19),
            ("%w(one two three)*\", \"", 18),
            ("foo = %w(one two three)*\", \"", 24),
        ] {
            let o = offenses(&format!("# header\n{s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col, 2));
            assert_eq!(o[0].message, "Favor `Array#join` over `Array#*`.");
        }
    }
    #[test]
    fn all_three_pinned_corrections_apply_and_stabilize() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "%w(one two three) * \", \"",
                "%w(one two three).join(\", \")",
            ),
            ("%w(one two three)*\", \"", "%w(one two three).join(\", \")"),
            (
                "foo = %w(one two three)*\", \"",
                "foo = %w(one two three).join(\", \")",
            ),
        ] {
            assert_correction(&ArrayJoin, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn canonical_empty_nested_percent_and_significant_token_bytes() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("[] * ','", "[].join(',')"),
            (
                "[nil, [1, false], :x] * ','",
                "[nil, [1, false], :x].join(',')",
            ),
            ("%w[a\\ b c\\]d] * %q|,|", "%w[a\\ b c\\]d].join(%q|,|)"),
            ("%i[a b] * ','", "%i[a b].join(',')"),
            ("%W[a b] * ','", "%W[a b].join(',')"),
            ("['x'].*(%q , )", "['x'].join(%q , )"),
            ("['x'] * ?\\ ", "['x'].join(?\\ )"),
            (
                "x = ['a'] * ','; y = [] * ','",
                "x = ['a'].join(','); y = [].join(',')",
            ),
            ("['x']::*(%q|,|)", "['x'].join(%q|,|)"),
            ("['\\xFF'] * \"\\xFF\"", "['\\xFF'].join(\"\\xFF\")"),
        ] {
            assert_correction(&ArrayJoin, before, after);
            assert!(offenses(after).is_empty());
        }
    }
    #[test]
    fn pinned_and_structural_no_offense_lookalikes() {
        for s in [
            "%w(one two three) * 4",
            "%w(one two three) * test",
            "x * ','",
            "'x' * ','",
            "[a] * \"#{effect}\"",
            "[a]&.*(',')",
            "[a].*(',', &block)",
            "[a].*(&block)",
            "[a].*(',', ',')",
            "([a]) * ','",
            "[a] * (',')",
            "[a].join(',')",
            "# [a] * ','\n'*'",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
