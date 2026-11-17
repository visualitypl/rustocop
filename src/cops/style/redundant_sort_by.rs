//! `Style/RedundantSortBy` — identity key blocks, conservative safe fixes.
use super::symbol_proc::{local, parameter_names, single_body};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct RedundantSortBy;
impl Cop for RedundantSortBy {
    fn name(&self) -> &'static str {
        "Style/RedundantSortBy"
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
        let Some(display) = identity(&call) else {
            return;
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        let correction = cx.corrections_requested().then(|| Correction {
            range: selector.start_offset()..call.location().end_offset(),
            replacement: "sort".into(),
        });
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(call.location()),
            message: format!("Use `sort` instead of `sort_by {display}`."),
            correction,
        });
    }
}
pub(super) fn identity(call: &CallNode<'_>) -> Option<String> {
    if call.name().as_slice() != b"sort_by" || call.arguments().is_some() {
        return None;
    }
    let block = call.block()?.as_block_node()?;
    let [param] = parameter_names::<1>(block.parameters(), true)?;
    if !single_body(block.body()).is_some_and(|n| local(&n, param)) {
        return None;
    }
    let parameters = block.parameters()?;
    Some(if parameters.as_numbered_parameters_node().is_some() {
        "{ _1 }".into()
    } else if parameters.as_it_parameters_node().is_some() {
        "{ it }".into()
    } else {
        let name = String::from_utf8_lossy(param);
        format!("{{ |{name}| {name} }}")
    })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/redundant_sort_by_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::cops::testing::assert_correction;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantSortBy.name())
            .collect()
    }
    #[test]
    fn canonical_integer_fixes_all_pinned_shapes() {
        for block in [
            "{ |x| x }",
            "{ |y| y }",
            "{ _1 }",
            "{ it }",
            "do |x|\n x\nend",
        ] {
            for dot in [".", "&."] {
                let s = format!("[3,1,2]{dot}sort_by {block}");
                assert_correction(&RedundantSortBy, &s, &format!("[3,1,2]{dot}sort"));
            }
        }
    }
    #[test]
    fn explicit_implicit_and_multiline_positions() {
        for (block, display) in [
            ("{ |x| x }", "{ |x| x }"),
            ("{ |y| y }", "{ |y| y }"),
            ("{ _1 }", "{ _1 }"),
            ("{ it }", "{ it }"),
            ("do |x|\n x\nend", "{ |x| x }"),
        ] {
            for (dot, col) in [(".", 7), ("&.", 8)] {
                let s = format!("\narray{dot}sort_by {block}");
                let o = offenses(&s);
                assert_eq!(o.len(), 1, "{s}");
                assert_eq!((o[0].line, o[0].col), (2, col));
                assert_eq!(
                    o[0].message,
                    format!("Use `sort` instead of `sort_by {display}`.")
                );
            }
        }
    }
    #[test]
    fn transforms_destructuring_arity_and_control_flow_are_not_identity() {
        for s in [
            "xs.sort_by { |x| x.key }",
            "xs.sort_by { |x| -x }",
            "xs.sort_by(&:it)",
            "xs.sort_by(2) { |x| x }",
            "xs.sort_by { |x,y| x }",
            "xs.sort_by { |(x)| x }",
            "xs.sort_by { |x; y| x }",
            "xs.sort_by { |x=1| x }",
            "xs.sort_by { _2 }",
            "xs.sort_by { |x| x; x }",
            "xs.sort_by { |x| next x }",
            "xs.sort_by { |x| (x) }",
            "xs.sort_by { |x| other }",
            "xs.map { |x| x }",
            "xs.sort_by! { |x| x }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
