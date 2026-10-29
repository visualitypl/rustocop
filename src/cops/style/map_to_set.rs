//! `Style/MapToSet` — unsafe map/collect to_set advice; detection only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MapToSet;
impl Cop for MapToSet {
    fn name(&self) -> &'static str {
        "Style/MapToSet"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(to_set) = node.as_call_node() else {
            return;
        };
        if to_set.name().as_slice() != b"to_set"
            || to_set
                .arguments()
                .is_some_and(|a| !a.arguments().is_empty())
            || to_set.block().is_some()
        {
            return;
        }
        let Some(map) = to_set.receiver().and_then(|n| n.as_call_node()) else {
            return;
        };
        if !matches!(map.name().as_slice(), b"map" | b"collect")
            || map.arguments().is_some_and(|a| !a.arguments().is_empty())
        {
            return;
        }
        let Some(block) = map.block() else { return };
        if block.as_block_node().is_none()
            && !block
                .as_block_argument_node()
                .and_then(|b| b.expression())
                .is_some_and(|e| e.as_symbol_node().is_some())
        {
            return;
        }
        let selector = map.message_loc().unwrap();
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Pass a block to `to_set` instead of calling `{}.to_set`.",
                String::from_utf8_lossy(map.name().as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(selector),
            correction: None,
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop map_to_set_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/MapToSet")
            .collect()
    }
    #[test]
    fn literal_and_symbol_blocks_all_arities_navigation_and_messages() {
        for method in ["map", "collect"] {
            for block in [
                " { |x| x*2 }",
                " { |x,y| [x,y] }",
                " { [_1,_2] }",
                " { it*2 }",
                "(&:pair)",
                " { |(k,v); captured| captured=k; v }",
            ] {
                for (receiver, col) in [("foo.", 5), ("foo&.", 6), ("", 1)] {
                    for dot in [".", "&."] {
                        let s = format!("{receiver}{method}{block}{dot}to_set.freeze");
                        let found = offenses(&s);
                        assert_eq!(found.len(), 1, "{s}");
                        assert_eq!(
                            (found[0].line, found[0].col, found[0].end_line),
                            (1, col, 1)
                        );
                        assert_eq!(
                            found[0].message,
                            format!(
                                "Pass a block to `to_set` instead of calling `{method}.to_set`."
                            )
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn lookalikes_and_outer_blocks_are_excluded() {
        for s in [
            "foo.map.to_set",
            "foo.map { it }",
            "foo.map(&converter).to_set",
            "foo.map(1) { it }.to_set",
            "foo.map(1,&:pair).to_set",
            "foo.map { it }.to_set(1)",
            "foo.map { it }.to_set(&:pair)",
            "foo.map { it }.to_set { it }",
            "foo.map { it }.to_set { _1 }",
            "foo.collect { it }.to_set { |x| x }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn multiline_control_flow_and_lazy_receivers_are_reported_not_rewritten() {
        for body in ["next x", "break x", "return x", "captured << x; x"] {
            let s = format!("side_effect.lazy\n  .collect {{ |x| {body} }}\n  &.to_set");
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (2, 4));
            assert!(found[0].correction.is_none());
        }
    }
}
