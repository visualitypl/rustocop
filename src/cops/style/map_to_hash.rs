//! `Style/MapToHash` — map followed by to_h (unsafe; Ruby >= 2.6).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MapToHash;
impl Cop for MapToHash {
    fn name(&self) -> &'static str {
        "Style/MapToHash"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(to_h) = node.as_call_node() else {
            return;
        };
        if to_h.name().as_slice() != b"to_h"
            || to_h.arguments().is_some_and(|a| !a.arguments().is_empty())
            || to_h.block().is_some()
        {
            return;
        }
        let Some(map) = to_h.receiver().and_then(|n| n.as_call_node()) else {
            return;
        };
        if !matches!(map.name().as_slice(), b"map" | b"collect")
            || map.arguments().is_some_and(|a| !a.arguments().is_empty())
        {
            return;
        }
        let Some(block) = map.block() else { return };
        if block.as_block_node().is_none()
            && !(ast::call_arguments(&map).len() == 1
                && block
                    .as_block_argument_node()
                    .and_then(|n| n.expression())
                    .is_some_and(|n| n.as_symbol_node().is_some()))
        {
            return;
        }
        let selector = map.message_loc().unwrap();
        let (line, col) = file.line_col(selector.start_offset());
        let dot = if to_h.is_safe_navigation() { "&." } else { "." };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Pass a block to `to_h` instead of calling `{}{dot}to_h`.",
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
    //! Ported from RuboCop map_to_hash_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/MapToHash")
            .collect()
    }
    #[test]
    fn block_arities_captures_and_safe_navigation() {
        for method in ["map", "collect"] {
            for block in [
                " { |x| [x, x*2] }",
                " { |(k,v)| [k,v*2] }",
                " { |x,y| [x.to_s,y.to_i] }",
                " { [_1,_2] }",
                " { [it,it] }",
                "(&:pair)",
                " { |x; captured| captured = x; [x,captured] }",
            ] {
                for (receiver, col) in [("foo.", 5), ("foo&.", 6), ("", 1)] {
                    for dot in [".", "&."] {
                        let s = format!("{receiver}{method}{block}{dot}to_h.freeze");
                        let found = offenses(&s);
                        assert_eq!(found.len(), 1, "{s}");
                        assert_eq!((found[0].line, found[0].col), (1, col));
                        assert_eq!(
                            found[0].message,
                            format!(
                                "Pass a block to `to_h` instead of calling `{method}{dot}to_h`."
                            )
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn rejects_outer_arguments_literal_blocks_and_non_symbol_passes() {
        for s in [
            "foo.map { |x| x*2 }",
            "foo.map.to_h",
            "foo.map(&:pair).to_h(1)",
            "foo.map { |x| x }.to_h { |x| x }",
            "foo.map { _1 }.to_h { [it,it] }",
            "foo.map(&converter).to_h",
            "foo.map(1, &:pair).to_h",
            "foo.collect(&:pair).to_a",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn neither_map_nor_to_h_may_have_additional_arguments() {
        for s in [
            "foo.map(1) { |x| x }.to_h",
            "foo.map { |x| x }.to_h(&:pair)",
            "foo.map(&:pair).to_h(&converter)",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn general_literal_blocks_include_control_flow() {
        for block in [
            " { |x| next [x,x] }",
            " { |x| break [x,x] }",
            " { |x| return [x,x] }",
        ] {
            let s = format!("side_effect().lazy.map{block}.to_h");
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none());
        }
        let found = offenses("foo\n  .collect { |x| [x,x] }\n  &.to_h");
        assert_eq!((found[0].line, found[0].col), (2, 4));
    }
}
