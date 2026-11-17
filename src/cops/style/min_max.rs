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
                Some(Correction {
                    range: range.clone(),
                    replacement: format!(
                        "{}.minmax",
                        std::str::from_utf8(receiver.location().as_slice()).ok()?
                    ),
                })
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
