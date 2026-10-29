//! `Style/MapCompactWithConditionalBlock` — unsafe select/reject suggestions.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct MapCompactWithConditionalBlock;
impl Cop for MapCompactWithConditionalBlock {
    fn name(&self) -> &'static str {
        "Style/MapCompactWithConditionalBlock"
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
        let (map, compact) =
            if call.name().as_slice() == b"compact" && ast::call_arguments(&call).is_empty() {
                let Some(map) = call.receiver().and_then(|n| n.as_call_node()) else {
                    return;
                };
                (map, true)
            } else if call.name().as_slice() == b"filter_map" {
                // The outer compact reports the same start with the longer range.
                if cx.parent().and_then(|n| n.as_call_node()).is_some_and(|p| {
                    p.name().as_slice() == b"compact" && ast::call_arguments(&p).is_empty()
                }) {
                    return;
                }
                (call, false)
            } else {
                return;
            };
        if !matches!(map.name().as_slice(), b"map" | b"filter_map")
            || !ast::call_arguments(&map).is_empty()
        {
            return;
        }
        let Some(block) = map.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        let Some(params) = block
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
        else {
            return;
        };
        if !params.locals().is_empty() {
            return;
        }
        let Some(p) = params.parameters() else { return };
        if p.requireds().len() != 1
            || !p.optionals().is_empty()
            || p.rest()
                .is_some_and(|r| r.as_implicit_rest_node().is_none())
            || !p.posts().is_empty()
            || !p.keywords().is_empty()
            || p.keyword_rest().is_some()
            || p.block().is_some()
        {
            return;
        }
        let Some(param) = p
            .requireds()
            .iter()
            .next()
            .and_then(|n| n.as_required_parameter_node())
        else {
            return;
        };
        let Some(body) = block.body().and_then(|n| n.as_statements_node()) else {
            return;
        };
        let Some(select) = conditional(
            &body.body().iter().collect::<Vec<_>>(),
            param.name().as_slice(),
        ) else {
            return;
        };
        let selector = map.message_loc().unwrap();
        let (line, col) = file.line_col(selector.start_offset());
        let end = if compact {
            ast::source_end(node)
        } else {
            map.location().end_offset()
        };
        let current = if compact {
            format!(
                "{} {{ ... }}.compact",
                String::from_utf8_lossy(map.name().as_slice())
            )
        } else {
            "filter_map { ... }".into()
        };
        let prefer = if select { "select" } else { "reject" };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Replace `{current}` with `{prefer}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
fn single<'pr>(body: Option<ruby_prism::StatementsNode<'pr>>) -> Option<Option<Node<'pr>>> {
    let Some(body) = body else { return Some(None) };
    (body.body().len() <= 1).then(|| body.body().iter().next())
}
fn branches<'pr>(node: &Node<'pr>) -> Option<(Option<Node<'pr>>, Option<Node<'pr>>, bool)> {
    if let Some(n) = node.as_if_node() {
        let other = match n.subsequent() {
            None => None,
            Some(n) => single(n.as_else_node()?.statements())?,
        };
        Some((single(n.statements())?, other, false))
    } else {
        let n = node.as_unless_node()?;
        let other = match n.else_clause() {
            Some(e) => single(e.statements())?,
            None => None,
        };
        Some((single(n.statements())?, other, true))
    }
}
fn local(node: Option<&Node<'_>>, name: &[u8]) -> bool {
    node.and_then(|n| n.as_local_variable_read_node())
        .is_some_and(|n| n.name().as_slice() == name)
}
fn next_local(node: Option<&Node<'_>>, name: &[u8]) -> bool {
    node.and_then(|n| n.as_next_node())
        .and_then(|n| n.arguments())
        .is_some_and(|a| {
            a.arguments().len() == 1 && local(a.arguments().iter().next().as_ref(), name)
        })
}
fn empty_or_next(node: Option<&Node<'_>>, explicit_nil: bool) -> bool {
    node.is_none_or(|n| n.as_next_node().is_some() || (explicit_nil && n.as_nil_node().is_some()))
}
fn conditional(body: &[Node<'_>], name: &[u8]) -> Option<bool> {
    match body {
        [condition] => {
            let (yes, no, inverse) = branches(condition)?;
            if (local(yes.as_ref(), name) && empty_or_next(no.as_ref(), false))
                || (next_local(yes.as_ref(), name) && empty_or_next(no.as_ref(), true))
            {
                Some(!inverse)
            } else if (local(no.as_ref(), name) && empty_or_next(yes.as_ref(), false))
                || (next_local(no.as_ref(), name) && empty_or_next(yes.as_ref(), true))
            {
                Some(inverse)
            } else {
                None
            }
        }
        [condition, last] => {
            let (yes, no, inverse) = branches(condition)?;
            if no.is_some() {
                return None;
            }
            let next = yes.as_ref()?.as_next_node()?;
            if local(Some(last), name) {
                // Match the pinned argument-count message policy, including next nil.
                let has_value = next.arguments().is_some_and(|a| !a.arguments().is_empty());
                Some(has_value != inverse)
            } else if last.as_nil_node().is_some() && next_local(yes.as_ref(), name) {
                Some(!inverse)
            } else {
                None
            }
        }
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop map_compact_with_conditional_block_spec.rb (MIT
    //! License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == MapCompactWithConditionalBlock.name())
            .collect()
    }
    #[test]
    fn all_pinned_conditional_shapes_and_messages() {
        for (body, prefer) in [
            ("if item.bar?; item; else; next; end", "select"),
            ("if item.bar? && bar.baz; item; else; next; end", "select"),
            ("if item.bar?; next item; else; nil; end", "select"),
            ("if item.bar?; next; else; item; end", "reject"),
            ("if item.bar?; nil; else; next item; end", "reject"),
            ("item.bar? ? item : next", "select"),
            ("item.bar? ? next : item", "reject"),
            ("item if item.bar?", "select"),
            ("item unless item.bar?", "reject"),
            ("next if item.bar?; item", "reject"),
            ("next unless item.bar?; item", "select"),
            ("next item if item.bar?", "select"),
            ("next item unless item.bar?", "reject"),
            ("next item if item.bar?; nil", "select"),
            ("next item unless item.bar?; nil", "reject"),
            // Pinned lexical quirk: an explicit next nil is treated as carrying
            // a value when choosing the message, even though nil is discarded.
            ("next nil if item.bar?; item", "select"),
            ("next nil unless item.bar?; item", "reject"),
            ("if item.bar?; item; end", "select"),
            ("unless item.bar?; item; end", "reject"),
        ] {
            for (method, suffix, current) in [
                ("map", ".compact", "map { ... }.compact"),
                ("filter_map", "", "filter_map { ... }"),
                ("filter_map", ".compact", "filter_map { ... }.compact"),
            ] {
                for dot in [".", "&."] {
                    let s = format!("foo{dot}{method} do |item|\n  {body}\nend{suffix}");
                    let o = offenses(&s);
                    assert_eq!(o.len(), 1, "{s}: {o:?}");
                    assert_eq!((o[0].line, o[0].col), (1, if dot == "." { 5 } else { 6 }));
                    assert_eq!(
                        o[0].message,
                        format!("Replace `{current}` with `{prefer}`.")
                    );
                    assert!(o[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn pinned_no_offenses_and_arity_value_lookalikes() {
        for s in [
            "foo.map { |item| item if item.bar? }",
            "foo.map { |item| item if item.bar? }.do_something",
            "foo.map { |item| item if item.bar? }.compact(arg)",
            "foo.map { |item| item if item.bar? }.compact(&b)",
            "foo.map { |item| item.bar? ? 1 : 2 }.compact",
            "foo.map { |item| if item.bar?; item; elsif baz; item; else; next; end }.compact",
            "foo.map { |item| item if item.bar?; extra }.compact",
            "foo.map { |item| next if item.bar?; next if item.baz?; item }.compact",
            "foo.map { |item, other| item if item.bar? }.compact",
            "foo.map { _1 if _1.bar? }.compact",
            "foo.filter_map { it if it.bar? }",
            "foo.collect { |item| item if item.bar? }.compact",
            "foo.map(1) { |item| item if item.bar? }.compact",
            "foo.filter_map { |item| other if item.bar? }",
            "foo.map { |item| item.bar? ? item : nil }.compact",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn single_line_exact_position_and_end_line() {
        let s = "foo.map { |item| item.bar? ? item : next }.compact";
        let o = offenses(s);
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 5, 1));
        assert_eq!(o[0].message, "Replace `map { ... }.compact` with `select`.");
    }
}
