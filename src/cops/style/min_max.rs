//! `Style/MinMax` — combined min/max calls.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MinMax;
impl Cop for MinMax {
    fn name(&self) -> &'static str {
        "Style/MinMax"
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
        let (elements, array) = if let Some(a) = node.as_array_node() {
            (a.elements().iter().collect::<Vec<_>>(), true)
        } else if let Some(r) = node.as_return_node() {
            (
                r.arguments()
                    .map(|a| a.arguments().iter().collect())
                    .unwrap_or_default(),
                false,
            )
        } else {
            return;
        };
        let [min, max] = elements.as_slice() else {
            return;
        };
        let (Some(min), Some(max)) = (min.as_call_node(), max.as_call_node()) else {
            return;
        };
        if min.name().as_slice() != b"min"
            || max.name().as_slice() != b"max"
            || min.is_safe_navigation()
            || max.is_safe_navigation()
            || min.block().is_some()
            || max.block().is_some()
            || !ast::call_arguments(&min).is_empty()
            || !ast::call_arguments(&max).is_empty()
        {
            return;
        }
        let (Some(receiver), Some(other)) = (min.receiver(), max.receiver()) else {
            return;
        };
        if ast::fingerprint_with_source_lines(&receiver, file)
            != ast::fingerprint_with_source_lines(&other, file)
        {
            return;
        }
        let range = if array {
            node.location().start_offset()..node.location().end_offset()
        } else {
            min.location().start_offset()..max.location().end_offset()
        };
        let (line, col) = file.line_col(range.start);
        let correction = cx
            .corrections_requested()
            .then(|| {
                if !integer_array(&receiver) {
                    return None;
                }
                safe_numeric_edit(
                    file,
                    cx,
                    Correction {
                        range: range.clone(),
                        replacement: format!(
                            "{}.minmax",
                            std::str::from_utf8(receiver.location().as_slice()).ok()?
                        ),
                    },
                )
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(range.end.saturating_sub(1)),
            message: format!(
                "Use `{}.minmax` instead of `{}`.",
                single_line(receiver.location().as_slice()),
                single_line(&file.bytes[range])
            ),
            correction,
        });
    }
}
/// Only immediate Integers: no comparisons, conversions, mutable ties, NaN or
/// heap-identity changes can be hidden in the key/receiver expressions.
pub(super) fn integer_array(node: &Node<'_>) -> bool {
    let grouped;
    let node = if let Some(p) = node.as_parentheses_node() {
        let Some(s) = p.body().and_then(|n| n.as_statements_node()) else {
            return false;
        };
        if s.body().len() != 1 {
            return false;
        }
        grouped = s.body().iter().next().unwrap();
        &grouped
    } else {
        node
    };
    node.as_array_node().is_some_and(|a| {
        a.elements().iter().all(|n| {
            n.as_integer_node().is_some_and(|n| {
                let value: Result<i32, _> = n.value().try_into();
                value.is_ok_and(|v| (-(1 << 30)..(1 << 30)).contains(&v))
            })
        })
    })
}
pub(super) fn safe_numeric_edit(
    file: &SourceFile,
    cx: &NodeContext<'_, '_>,
    edit: Correction,
) -> Option<Correction> {
    let root = cx.ancestors().next_back()?;
    let facts = file.min_max_safety(root);
    if facts.changed
        || facts
            .first_effect
            .is_some_and(|end| end <= edit.range.start)
        || super::correction::contains_comment(file, edit.range.clone())
        || super::correction::touches_directive(file, edit.range.clone())
        || file.heredoc_pending_at(root, edit.range.start)
    {
        return None;
    }
    super::literal_spelling::safe_edit(file, cx, edit)
}
/// Shared correction-only, single main-AST walk. Unsupported scopes/syntax and
/// visible declarations invalidate globally; completed unknown calls invalidate
/// subsequent edits. We do not infer external monkey patches or local types.
#[derive(Default)]
pub(crate) struct Safety {
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Safety {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        if let Some(call) = node.as_call_node() {
            let numeric = call.receiver().is_some_and(|n| integer_array(&n));
            let trusted = numeric
                && (super::redundant_min_max_by::identity(&call).is_some()
                    || super::redundant_sort_by::identity(&call).is_some()
                    || (matches!(
                        call.name().as_slice(),
                        b"min" | b"max" | b"minmax" | b"sort"
                    ) && call.arguments().is_none()
                        && call.block().is_none()));
            if !trusted {
                self.changed |= call.block().is_some();
                let end = call.location().end_offset();
                self.first_effect = Some(self.first_effect.map_or(end, |old| old.min(end)));
            }
            return;
        }
        self.changed |= !ast::is_basic_literal(&node)
            && !matches!(
                node,
                Node::ProgramNode { .. }
                    | Node::StatementsNode { .. }
                    | Node::ArgumentsNode { .. }
                    | Node::ArrayNode { .. }
                    | Node::IntegerNode { .. }
                    | Node::FloatNode { .. }
                    | Node::LocalVariableReadNode { .. }
                    | Node::LocalVariableWriteNode { .. }
                    | Node::SourceLineNode { .. }
                    | Node::ReturnNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::BlockNode { .. }
                    | Node::BlockParametersNode { .. }
                    | Node::ParametersNode { .. }
                    | Node::RequiredParameterNode { .. }
                    | Node::ImplicitRestNode { .. }
                    | Node::NumberedParametersNode { .. }
                    | Node::ItParametersNode { .. }
                    | Node::ItLocalVariableReadNode { .. }
            );
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/min_max_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == MinMax.name())
            .collect()
    }
    #[test]
    fn canonical_numeric_fixes_and_unknown_receiver_withholding() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for (s, after) in [
            ("[[3,1,2].min, [3,1,2].max]", "[3,1,2].minmax"),
            ("x = [3,1,2].min, [3,1,2].max", "x = [3,1,2].minmax"),
            ("return [3,1,2].min, [3,1,2].max", "return [3,1,2].minmax"),
            ("[([3,1]).min, ([3,1]).max]", "([3,1]).minmax"),
        ] {
            assert_correction(&MinMax, s, after);
        }
        for s in [
            "[foo.min, foo.max]",
            "[xs.min, xs.max]",
            "[[nil].min, [nil].max]",
            "[[1.0].min, [1.0].max]",
            "[[99999999999999999999999].min, [99999999999999999999999].max]",
            "[[[1]].min, [[1]].max]",
            "[[3, # retained\n1].min, [3,1].max]",
            "[[3,1].min, [3,\n1].max]; p __LINE__",
        ] {
            let o = correction_offenses(&MinMax, s);
            assert_eq!(o.len(), 1, "{s}");
            assert!(o[0].correction.is_none(), "{s}");
        }
    }
    #[test]
    fn visible_overrides_loads_observers_and_deferred_scopes_withhold_both_safe_cops() {
        use crate::cops::testing::correction_offenses;
        for prefix in [
            "class Integer; def <=>(x); 0; end; end\n",
            "class Array; private :min; end\n",
            "class Array; alias min max; end\n",
            "class Array; undef minmax; end\n",
            "Array.define_method(:min) { 7 }\n",
            "require 'patch'\n",
            "eval(code)\n",
            "using Extensions\n",
            "ObjectSpace.count_objects\n",
            "TracePoint.new {}\n",
            "DATA.pos\n",
            "File.read(__FILE__)\n",
        ] {
            for (cop, body) in [
                (&MinMax as &dyn Cop, "[[3,1].min,[3,1].max]\n"),
                (
                    &super::super::RedundantMinMaxBy as &dyn Cop,
                    "[3,1].min_by { it }\n",
                ),
            ] {
                let s = format!("{prefix}{body}");
                let o = correction_offenses(cop, &s);
                assert_eq!(o.len(), 1, "{s}");
                assert!(o[0].correction.is_none(), "{s}");
            }
        }
        for s in [
            "def f; [[3,1].min,[3,1].max]; end",
            "custom.each { [[3,1].min,[3,1].max] }",
            "[[3,1].min,[3,1].max]; class Array; def minmax; 7; end; end",
        ] {
            assert!(
                correction_offenses(&MinMax, s)[0].correction.is_none(),
                "{s}"
            );
        }
    }
    #[test]
    fn pinned_array_assignment_and_return_locations() {
        for (s, col, old) in [
            ("[foo.min, foo.max]", 1, "[foo.min, foo.max]"),
            ("bar = foo.min, foo.max", 7, "foo.min, foo.max"),
            ("return foo.min, foo.max", 8, "foo.min, foo.max"),
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (1, col));
            assert_eq!(
                o[0].message,
                format!("Use `foo.minmax` instead of `{old}`.")
            );
        }
    }
    #[test]
    fn source_line_equality_uses_evaluated_lines() {
        let s = "\n[[__LINE__].min, [__LINE__].max]";
        let o = offenses(s);
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col), (2, 1));
        assert_eq!(
            o[0].message,
            "Use `[__LINE__].minmax` instead of `[[__LINE__].min, [__LINE__].max]`."
        );
        assert!(
            crate::cops::testing::correction_offenses(&MinMax, s)[0]
                .correction
                .is_none()
        );
        assert!(offenses("[[__LINE__].min,\n [__LINE__].max]").is_empty());
    }
    #[test]
    fn pinned_negative_shapes_and_comparators() {
        for s in [
            "[foo.min, bar.max]",
            "[foo.min, foo.baz, foo.max]",
            "[min, max]",
            "bar = foo.min, bar.max",
            "bar = min, max",
            "return foo.min, foo.baz, foo.max",
            "return foo.min, bar.max",
            "return min, max",
            "[foo.max, foo.min]",
            "[foo.min(2), foo.max]",
            "[foo.min { |a,b| b <=> a }, foo.max]",
            "[foo&.min, foo&.max]",
            "[foo.min(&cmp), foo.max]",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
