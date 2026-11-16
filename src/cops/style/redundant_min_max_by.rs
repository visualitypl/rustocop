//! `Style/RedundantMinMaxBy` — identity key blocks.
use super::symbol_proc::{local, parameter_names, single_body};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct RedundantMinMaxBy;
impl Cop for RedundantMinMaxBy {
    fn name(&self) -> &'static str {
        "Style/RedundantMinMaxBy"
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
        let Some((replacement, display)) = identity(&call) else {
            return;
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (line, col) = file.line_col(selector.start_offset());
        // RuboCop replaces from the selector to the end of the block.
        let correction = cx.corrections_requested().then(|| Correction {
            range: selector.start_offset()..call.location().end_offset(),
            replacement: replacement.into(),
        });
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(call.location()),
            message: format!(
                "Use `{replacement}` instead of `{} {display}`.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
            correction,
        });
    }
}
pub(super) fn identity(call: &CallNode<'_>) -> Option<(&'static str, String)> {
    if call.arguments().is_some() {
        return None;
    }
    let replacement = match call.name().as_slice() {
        b"max_by" => "max",
        b"min_by" => "min",
        b"minmax_by" => "minmax",
        _ => return None,
    };
    let block = call.block()?.as_block_node()?;
    let [param] = parameter_names::<1>(block.parameters(), true)?;
    if !single_body(block.body()).is_some_and(|n| local(&n, param)) {
        return None;
    }
    let parameters = block.parameters()?;
    let display = if parameters.as_numbered_parameters_node().is_some() {
        "{ _1 }".into()
    } else if parameters.as_it_parameters_node().is_some() {
        "{ it }".into()
    } else {
        let name = String::from_utf8_lossy(param);
        format!("{{ |{name}| {name} }}")
    };
    Some((replacement, display))
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/redundant_min_max_by_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == RedundantMinMaxBy.name())
            .collect()
    }
    #[test]
    fn all_pinned_methods_and_block_shapes() {
        for (method, replacement) in [
            ("max_by", "max"),
            ("min_by", "min"),
            ("minmax_by", "minmax"),
        ] {
            for (block, display) in [
                ("{ |x| x }", "{ |x| x }"),
                ("{ |y| y }", "{ |y| y }"),
                ("do |x|\n x\nend", "{ |x| x }"),
                ("{ _1 }", "{ _1 }"),
                ("{ it }", "{ it }"),
            ] {
                for (dot, col) in [(".", 7), ("&.", 8)] {
                    let s = format!("array{dot}{method} {block}");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}");
                    assert_eq!((o[0].line, o[0].col), (1, col));
                    assert_eq!(
                        o[0].message,
                        format!("Use `{replacement}` instead of `{method} {display}`.")
                    );
                }
            }
        }
    }
    #[test]
    fn rejects_arguments_transforms_and_nonidentity_bindings() {
        for s in [
            "array.max_by { |x| x.foo }",
            "array.max_by { |x| -x }",
            "array.max_by(&:foo)",
            "array.min_by(2) { |x| x }",
            "array.min_by { |x,y| x }",
            "array.min_by { |(x)| x }",
            "array.min_by { |x; y| x }",
            "array.min_by { |x=1| x }",
            "array.min_by { _2 }",
            "array.min_by { |x| x; x }",
            "array.min_by { |x| next x }",
            "array.min_by { |x| (x) }",
            "array.min_by { |x| other }",
            "array.map { |x| x }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
