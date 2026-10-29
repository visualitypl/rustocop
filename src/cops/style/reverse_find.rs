//! `Style/ReverseFind` — Ruby 4.0 `rfind` hints (inactive at fixed Ruby 3.4).
//! Unsafe for unknown/lazy receivers; never emits corrections.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ReverseFind;
impl Cop for ReverseFind {
    fn name(&self) -> &'static str {
        "Style/ReverseFind"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !matches!(call.name().as_slice(), b"find" | b"detect")
            || !crate::cops::ast::call_arguments(&call).iter().all(|n| {
                n.as_block_argument_node()
                    .is_some_and(|b| b.expression().is_some_and(|n| n.as_symbol_node().is_some()))
            })
        {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let Some(reverse) = receiver.as_call_node() else {
            return;
        };
        if !matches!(reverse.name().as_slice(), b"reverse" | b"reverse_each")
            || !crate::cops::ast::call_arguments(&reverse).is_empty()
            || reverse.block().is_some()
        {
            return;
        }
        let (Some(start), Some(end)) = (reverse.message_loc(), call.message_loc()) else {
            return;
        };
        let (line, col) = file.line_col(start.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(end),
            message: "Use `rfind` instead.".into(),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/reverse_find_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        config::ResolvedConfig,
        cops::{Registry, policy::RubyVersion, select_active_for_version},
        pipeline::lint_str_with,
    };
    fn offenses(source: &str, version: RubyVersion) -> Vec<Offense> {
        let registry = Registry::from_cops(vec![Box::new(ReverseFind)]);
        let config = ResolvedConfig::default();
        let active = select_active_for_version(
            &registry,
            &config,
            &["Style/ReverseFind".into()],
            &[],
            version,
        );
        lint_str_with(&active, source)
    }
    #[test]
    fn ruby_four_real_matcher_all_pinned_forms_and_no_unsafe_fixes() {
        for (source, col) in [
            ("array.reverse.find { |item| item.even? }", 7),
            ("array&.reverse&.find { |item| item.even? }", 8),
            ("array.reverse.find { _1.even? }", 7),
            ("array.reverse.find { it.even? }", 7),
            ("array.reverse.detect { |item| item.even? }", 7),
            ("array.reverse_each.find { |item| item.even? }", 7),
            ("array.reverse_each.detect { |item| item.even? }", 7),
            ("array.reverse.find(&:even?)", 7),
            ("array.reverse.detect(&:even?)", 7),
        ] {
            let found = offenses(source, RubyVersion(4, 0));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (1, col, 1)
            );
            assert_eq!(found[0].message, "Use `rfind` instead.");
            assert!(
                crate::cops::testing::correction_offenses(&ReverseFind, source)[0]
                    .correction
                    .is_none()
            );
            assert!(offenses(source, RubyVersion(3, 4)).is_empty());
        }
        assert!(!ReverseFind.safe_fixes());
    }
    #[test]
    fn argument_arity_and_receiver_lookalikes() {
        for source in [
            "array.rfind { it.even? }",
            "array.reverse(1).find",
            "array.reverse { it }.find",
            "array.reverse.find(fallback)",
            "array.reverse.find(&predicate)",
            "array.reverse.find(1, &:even?)",
            "array.sort.find { it }",
            "array.reverse.find_all { it }",
        ] {
            assert!(offenses(source, RubyVersion(4, 0)).is_empty(), "{source}");
        }
    }
    #[test]
    fn enumerators_side_effects_and_control_flow_stay_detection_only() {
        for source in [
            "produce.lazy.reverse_each.detect",
            "reverse.find { break :found }",
            "produce.reverse.find { |a,b| a == b }",
            "produce.reverse_each.detect { return it }",
        ] {
            let found = offenses(source, RubyVersion(4, 0));
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
    }
}
