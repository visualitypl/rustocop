//! `Style/MapJoin` — redundant string conversion before join (unsafe).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MapJoin;
impl Cop for MapJoin {
    fn name(&self) -> &'static str {
        "Style/MapJoin"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(join) = node.as_call_node() else {
            return;
        };
        if join.name().as_slice() != b"join" {
            return;
        }
        let Some(map) = join.receiver().and_then(|n| n.as_call_node()) else {
            return;
        };
        if !matches!(map.name().as_slice(), b"map" | b"collect")
            || map.arguments().is_some_and(|a| !a.arguments().is_empty())
        {
            return;
        }
        let Some(block) = map.block() else { return };
        let matches = if let Some(pass) = block.as_block_argument_node() {
            pass.expression()
                .and_then(|n| n.as_symbol_node())
                .is_some_and(|n| n.unescaped() == b"to_s")
        } else if let Some(block) = block.as_block_node() {
            converts_parameter(&block)
        } else {
            false
        };
        if !matches {
            return;
        }
        let selector = map.message_loc().unwrap();
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Remove redundant `{}(&:to_s)` before `join`.",
                String::from_utf8_lossy(map.name().as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(selector),
            correction: None,
        });
    }
}
fn converts_parameter(block: &ruby_prism::BlockNode<'_>) -> bool {
    let Some(parameters) = block.parameters() else {
        return false;
    };
    let name = if let Some(p) = parameters.as_numbered_parameters_node() {
        if p.maximum() != 1 {
            return false;
        }
        b"_1".as_slice()
    } else if parameters.as_it_parameters_node().is_some() {
        b"it".as_slice()
    } else {
        let Some(p) = parameters.as_block_parameters_node() else {
            return false;
        };
        if !p.locals().is_empty() {
            return false;
        }
        let Some(p) = p.parameters() else {
            return false;
        };
        if p.requireds().len() != 1
            || !p.optionals().is_empty()
            || p.rest()
                .is_some_and(|r| r.as_implicit_rest_node().is_none())
            || !p.posts().is_empty()
            || !p.keywords().is_empty()
            || p.keyword_rest().is_some()
            || p.block().is_some()
        {
            return false;
        }
        let Some(p) = p
            .requireds()
            .iter()
            .next()
            .and_then(|n| n.as_required_parameter_node())
        else {
            return false;
        };
        p.name().as_slice()
    };
    let Some(body) = block.body().and_then(|n| n.as_statements_node()) else {
        return false;
    };
    if body.body().len() != 1 {
        return false;
    }
    let Some(call) = body.body().iter().next().and_then(|n| n.as_call_node()) else {
        return false;
    };
    !call.is_safe_navigation()
        && call.name().as_slice() == b"to_s"
        && ast::call_arguments(&call).is_empty()
        && call.block().is_none()
        && call.receiver().is_some_and(|n| {
            n.as_local_variable_read_node()
                .is_some_and(|n| n.name().as_slice() == name)
                || (name == b"it" && n.as_it_local_variable_read_node().is_some())
        })
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop map_join_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/MapJoin")
            .collect()
    }
    #[test]
    fn symbols_explicit_numbered_and_it_blocks() {
        for method in ["map", "collect"] {
            for body in [
                "(&:to_s)",
                " { |x| x.to_s }",
                " { _1.to_s }",
                " { it.to_s }",
            ] {
                for (receiver, col) in [("array.", 7), ("array&.", 8), ("", 1)] {
                    for suffix in [".join", "&.join(', ')"] {
                        let s = format!("{receiver}{method}{body}{suffix}\n");
                        let found = offenses(&s);
                        assert_eq!(found.len(), 1, "{s}");
                        assert_eq!((found[0].line, found[0].col), (1, col), "{s}");
                        assert_eq!(
                            found[0].message,
                            format!("Remove redundant `{method}(&:to_s)` before `join`.")
                        );
                        assert!(found[0].correction.is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn trailing_comma_does_not_introduce_a_second_parameter() {
        assert_eq!(offenses("foo.map { |x,| x.to_s }.join {}").len(), 1);
        assert!(offenses("foo.map { |x,*rest| x.to_s }.join").is_empty());
    }
    #[test]
    fn rejects_arity_control_flow_and_conversion_lookalikes() {
        for s in [
            "array.map(&:to_i).join",
            "array.map(&:to_s).compact",
            "array.join",
            "array.map { |x| x.to_i }.join",
            "array.map { |x| x.to_s(16) }.join",
            "array.map { |x,y| x.to_s }.join",
            "array.map { |(x,y)| x.to_s }.join",
            "array.map { |x; y| x.to_s }.join",
            "array.map { |x| y.to_s }.join",
            "array.map { |x| x&.to_s }.join",
            "array.map { _2.to_s }.join",
            "array.map { |x| next x.to_s }.join",
            "array.map { |x| return x.to_s }.join",
            "array.map { |x| p x; x.to_s }.join",
            "array.map(1) { |x| x.to_s }.join",
            "array.map(&converter).join",
            "array.map(&:to_s, other).join",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn multiline_selector_and_lazy_unknown_receivers_still_detect_without_fixes() {
        for (s, line, col) in [
            ("array\n  .map(&:to_s)\n  .join(', ')", 2, 4),
            ("source.lazy.map { _1.to_s }.join", 1, 13),
            ("side_effect().map(&:to_s).join(separator())", 1, 15),
            ("self.map(&:to_s).join", 1, 6),
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert!(found[0].correction.is_none());
        }
    }
}
