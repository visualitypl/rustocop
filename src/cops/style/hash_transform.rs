//! Shared pinned HashTransformMethod predicates. No corrections: dispatch,
//! defaults and iteration semantics are not proven by the hash-producing hints.
use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{BlockNode, CallNode, Node};
use std::{collections::HashMap, ops::Range};

type Key = (usize, usize, u64);
fn key(node: &Node<'_>) -> Key {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::mem::discriminant(node).hash(&mut h);
    (
        node.location().start_offset(),
        node.location().end_offset(),
        h.finish(),
    )
}

pub(super) fn check<'pr>(
    cop: &dyn Cop,
    keys: bool,
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let Some(call) = node.as_call_node() else {
        return;
    };
    let (inner, desc, each) = match call.name().as_slice() {
        b"each_with_object" if call.block().is_some_and(|b| b.as_block_node().is_some()) => {
            (&call, "each_with_object", true)
        }
        b"to_h" if ast::call_arguments(&call).is_empty() => {
            if let Some(inner) = call.receiver().and_then(|r| r.as_call_node()).filter(|c| {
                matches!(c.name().as_slice(), b"map" | b"collect")
                    && c.block().is_some_and(|b| b.as_block_node().is_some())
            }) {
                return candidate(
                    cop,
                    keys,
                    node,
                    &inner,
                    "map {...}.to_h",
                    false,
                    cx,
                    file,
                    out,
                );
            }
            (&call, "to_h {...}", false)
        }
        b"[]"
            if !call.is_safe_navigation()
                && !call
                    .block()
                    .is_some_and(|b| b.as_block_argument_node().is_some())
                && call
                    .receiver()
                    .is_some_and(|r| ast::short_const_name(&r).as_deref() == Some("Hash")) =>
        {
            let args = ast::call_arguments(&call);
            let [arg] = args.as_slice() else { return };
            let Some(inner) = arg
                .as_call_node()
                .filter(|c| matches!(c.name().as_slice(), b"map" | b"collect"))
            else {
                return;
            };
            return candidate(
                cop,
                keys,
                node,
                &inner,
                "Hash[_.map {...}]",
                false,
                cx,
                file,
                out,
            );
        }
        _ => return,
    };
    candidate(cop, keys, node, inner, desc, each, cx, file, out);
}

#[allow(clippy::too_many_arguments)]
fn candidate<'pr>(
    cop: &dyn Cop,
    keys: bool,
    report: &Node<'pr>,
    call: &CallNode<'pr>,
    desc: &str,
    each: bool,
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    if !call.receiver().is_some_and(|r| hash_receiver(&r)) {
        return;
    }
    let args = ast::call_arguments(call);
    if if each {
        args.len() != 1 || !empty_hash(&args[0])
    } else {
        !args.is_empty()
    } {
        return;
    }
    let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
        return;
    };
    let Some(params) = parameters(&block) else {
        return;
    };
    let Some(body) = block
        .body()
        .and_then(|b| b.as_statements_node())
        .filter(|b| b.body().len() == 1)
        .and_then(|b| b.body().iter().next())
    else {
        return;
    };
    let (k, v, memo, left, right) = if each {
        let [pair, memo] = params.as_slice() else {
            return;
        };
        let Some(pair) = pair
            .as_multi_target_node()
            .filter(|p| p.rest().is_none() && p.rights().is_empty() && p.lefts().len() == 2)
        else {
            return;
        };
        let pair: Vec<_> = pair.lefts().iter().collect();
        let Some(k) = param_name(&pair[0]) else {
            return;
        };
        let Some(v) = param_name(&pair[1]) else {
            return;
        };
        let Some(memo) = param_name(memo) else { return };
        let Some(assign) = body.as_call_node().filter(|c| {
            c.name().as_slice() == b"[]="
                && c.receiver().is_some_and(|r| local(&r, memo))
                && c.block().is_none()
        }) else {
            return;
        };
        let values = ast::call_arguments(&assign);
        let Ok([left, right]): Result<[_; 2], _> = values.try_into() else {
            return;
        };
        (k, v, Some(memo), left, right)
    } else {
        let [k, v] = params.as_slice() else { return };
        let Some(k) = param_name(k) else { return };
        let Some(v) = param_name(v) else { return };
        let Some(array) = body.as_array_node().filter(|a| a.elements().len() == 2) else {
            return;
        };
        let values: Vec<_> = array.elements().iter().collect();
        let Ok([left, right]): Result<[_; 2], _> = values.try_into() else {
            return;
        };
        (k, v, None, left, right)
    };
    let (transformed, unchanged, expr, preserved) = if keys {
        (k, v, left, right)
    } else {
        (v, k, right, left)
    };
    if !local(&preserved, unchanged) || local(&expr, transformed) || expr.as_splat_node().is_some()
    {
        return;
    }
    let facts = file.hash_transform_facts(cx.ancestors().next_back().unwrap());
    // Upstream intentionally compares lexical names, even in shadowed scopes.
    // The memo descendant pattern also includes scalar symbols (e.g. :memo
    // and method names), while both-args and use-arg inspect lvar nodes only.
    if facts.contains(&facts.reads, &expr, unchanged, false)
        || !facts.contains(&facts.reads, &expr, transformed, false)
        || memo.is_some_and(|m| facts.contains(&facts.symbols, &expr, m, true))
    {
        return;
    }
    let (line, col) = file.line_col(report.location().start_offset());
    let method = if keys {
        "transform_keys"
    } else {
        "transform_values"
    };
    // RuboCop reports its send, excluding any outer literal to_h block.
    let end = if desc != "each_with_object" && desc != "to_h {...}" {
        ast::source_end(report)
    } else {
        report.location().end_offset()
    };
    out.push(Offense {
        cop: cop.name(),
        severity: cop.severity(),
        message: format!("Prefer `{method}` over `{desc}`."),
        line,
        col,
        end_line: file.line_of(end.saturating_sub(1)),
        correction: None,
    });
}
fn local(node: &Node<'_>, name: &[u8]) -> bool {
    node.as_local_variable_read_node()
        .is_some_and(|r| r.name().as_slice() == name)
}
fn param_name<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    node.as_required_parameter_node()
        .map(|p| p.name().as_slice())
}
fn parameters<'pr>(block: &BlockNode<'pr>) -> Option<Vec<Node<'pr>>> {
    let params = block.parameters()?.as_block_parameters_node()?;
    if !params.locals().is_empty() {
        return None;
    }
    let p = params.parameters()?;
    if !p.optionals().is_empty()
        || p.rest()
            .is_some_and(|r| r.as_implicit_rest_node().is_none())
        || !p.posts().is_empty()
        || !p.keywords().is_empty()
        || p.keyword_rest().is_some()
        || p.block().is_some()
    {
        return None;
    }
    Some(p.requireds().iter().collect())
}
fn empty_hash(node: &Node<'_>) -> bool {
    node.as_hash_node().is_some_and(|h| h.elements().is_empty())
        || node
            .as_keyword_hash_node()
            .is_some_and(|h| h.elements().is_empty())
}
fn hash_receiver(node: &Node<'_>) -> bool {
    if node.as_hash_node().is_some() || node.as_keyword_hash_node().is_some() {
        return true;
    }
    let Some(c) = node.as_call_node().filter(|c| !c.is_safe_navigation()) else {
        return false;
    };
    if let Some(block) = c.block().and_then(|b| b.as_block_node()) {
        if block.parameters().is_some_and(|p| {
            matches!(
                p,
                Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. }
            )
        }) {
            return false;
        }
        return (ast::call_arguments(&c).is_empty()
            && matches!(
                c.name().as_slice(),
                b"group_by"
                    | b"to_h"
                    | b"tally"
                    | b"transform_keys"
                    | b"transform_keys!"
                    | b"transform_values"
                    | b"transform_values!"
            ))
            || (c.name().as_slice() == b"each_with_object" && {
                let a = ast::call_arguments(&c);
                a.len() == 1 && empty_hash(&a[0])
            });
    }
    matches!(
        c.name().as_slice(),
        b"to_h" | b"to_hash" | b"merge" | b"merge!" | b"update" | b"invert" | b"except" | b"tally"
    )
}

/// One main-AST walk for both cops; subtree intervals use AST preorder rather
/// than byte containment (heredoc bodies can lie outside their parent's span).
/// Sorted lexical-name postings make each candidate query logarithmic, without
/// cloning an ancestor's sets or rescanning nested transformation bodies.
#[derive(Default)]
pub(crate) struct Facts {
    ranges: HashMap<Key, Range<usize>>,
    reads: HashMap<Vec<u8>, Vec<usize>>,
    symbols: HashMap<Vec<u8>, Vec<usize>>,
    pub(crate) visits: usize,
}
impl Facts {
    fn contains(
        &self,
        postings: &HashMap<Vec<u8>, Vec<usize>>,
        node: &Node<'_>,
        name: &[u8],
        inclusive: bool,
    ) -> bool {
        let Some(range) = self.ranges.get(&key(node)) else {
            return false;
        };
        let Some(positions) = postings.get(name) else {
            return false;
        };
        let start = range.start + usize::from(!inclusive);
        positions
            .get(positions.partition_point(|&p| p < start))
            .is_some_and(|&p| p < range.end)
    }
    pub(crate) fn build(root: &Node<'_>) -> Self {
        #[derive(Default)]
        struct Builder {
            facts: Facts,
            stack: Vec<(Key, usize, bool)>,
        }
        impl<'pr> NodeVisitor<'pr> for Builder {
            fn enter(&mut self, node: Node<'pr>) {
                let pos = self.facts.visits;
                self.facts.visits += 1;
                // MatchWrite targets are Prism-only named-capture bindings,
                // not scalar descendants of RuboCop's match_with_lvasgn node.
                let capture_target = node.as_local_variable_target_node().is_some()
                    && self.stack.last().is_some_and(|(_, _, is_match)| *is_match);
                self.stack
                    .push((key(&node), pos, node.as_match_write_node().is_some()));
                if let Some(r) = node.as_local_variable_read_node() {
                    self.facts
                        .reads
                        .entry(r.name().as_slice().to_vec())
                        .or_default()
                        .push(pos);
                }
                if !capture_target && let Some(name) = scalar_symbol(&node) {
                    self.facts
                        .symbols
                        .entry(name.to_vec())
                        .or_default()
                        .push(pos);
                }
                // Upstream's regopt children are individual scalar symbols.
                // Prism stores them after the one-byte closing delimiter.
                let regexp_close = node
                    .as_regular_expression_node()
                    .map(|r| r.closing_loc())
                    .or_else(|| {
                        node.as_interpolated_regular_expression_node()
                            .map(|r| r.closing_loc())
                    });
                if let Some(close) = regexp_close {
                    for option in &close.as_slice()[1..] {
                        self.facts
                            .symbols
                            .entry(vec![*option])
                            .or_default()
                            .push(pos);
                    }
                }
                if let Some(symbol) = node.as_symbol_node() {
                    self.facts
                        .symbols
                        .entry(symbol.unescaped().to_vec())
                        .or_default()
                        .push(pos);
                }
            }
            fn leave(&mut self) {
                let (key, start, _) = self.stack.pop().unwrap();
                self.facts.ranges.insert(key, start..self.facts.visits);
            }
        }
        let mut b = Builder::default();
        walk(&mut b, root);
        b.facts
    }
}
fn scalar_symbol<'pr>(node: &Node<'pr>) -> Option<&'pr [u8]> {
    match node {
        Node::CallNode { .. } => Some(node.as_call_node().unwrap().name().as_slice()),
        Node::CallAndWriteNode { .. } => Some(
            node.as_call_and_write_node()
                .unwrap()
                .read_name()
                .as_slice(),
        ),
        Node::CallOrWriteNode { .. } => {
            Some(node.as_call_or_write_node().unwrap().read_name().as_slice())
        }
        Node::CallOperatorWriteNode { .. } => Some(
            node.as_call_operator_write_node()
                .unwrap()
                .read_name()
                .as_slice(),
        ),
        Node::LocalVariableReadNode { .. } => Some(
            node.as_local_variable_read_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::LocalVariableWriteNode { .. } => Some(
            node.as_local_variable_write_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::LocalVariableTargetNode { .. } => Some(
            node.as_local_variable_target_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::LocalVariableAndWriteNode { .. } => Some(
            node.as_local_variable_and_write_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::LocalVariableOrWriteNode { .. } => Some(
            node.as_local_variable_or_write_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::LocalVariableOperatorWriteNode { .. } => Some(
            node.as_local_variable_operator_write_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::RequiredParameterNode { .. } => {
            Some(node.as_required_parameter_node().unwrap().name().as_slice())
        }
        Node::OptionalParameterNode { .. } => {
            Some(node.as_optional_parameter_node().unwrap().name().as_slice())
        }
        Node::RequiredKeywordParameterNode { .. } => Some(
            node.as_required_keyword_parameter_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::OptionalKeywordParameterNode { .. } => Some(
            node.as_optional_keyword_parameter_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::RestParameterNode { .. } => node
            .as_rest_parameter_node()
            .unwrap()
            .name()
            .map(|n| n.as_slice()),
        Node::KeywordRestParameterNode { .. } => node
            .as_keyword_rest_parameter_node()
            .unwrap()
            .name()
            .map(|n| n.as_slice()),
        Node::BlockParameterNode { .. } => node
            .as_block_parameter_node()
            .unwrap()
            .name()
            .map(|n| n.as_slice()),
        Node::BlockLocalVariableNode { .. } => Some(
            node.as_block_local_variable_node()
                .unwrap()
                .name()
                .as_slice(),
        ),
        Node::DefNode { .. } => Some(node.as_def_node().unwrap().name().as_slice()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Pinned RuboCop style/hash_transform_{keys,values}_spec.rb and
    //! preferred_hash_methods_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Case {
        cop: String,
        source: String,
        positions: Vec<(u32, u32)>,
    }
    fn transforms(s: &str) -> Vec<crate::offense::Offense> {
        assert!(
            crate::parser::parse(s.as_bytes()).errors().next().is_none(),
            "{s}"
        );
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop.starts_with("Style/HashTransform"))
            .collect()
    }
    #[test]
    fn minimum_ruby_gates_apply_even_when_force_selected_or_config_enabled() {
        use crate::{
            config::ResolvedConfig,
            cops::{Registry, policy::RubyVersion, select_active_for_version},
            pipeline::lint_str_with,
        };
        let registry = Registry::new();
        for (cop, minimum, before, body) in [
            (
                "Style/HashTransformKeys",
                RubyVersion(2, 5),
                RubyVersion(2, 4),
                "h[k.to_s]=v",
            ),
            (
                "Style/HashTransformValues",
                RubyVersion(2, 4),
                RubyVersion(2, 3),
                "h[k]=v.to_s",
            ),
        ] {
            for only in [vec![], vec![cop.into()]] {
                let mut config = ResolvedConfig::default();
                config.cop_settings.entry(cop.into()).or_default().enabled = Some(true);
                assert!(
                    !select_active_for_version(&registry, &config, &only, &[], before)
                        .iter()
                        .any(|c| c.name() == cop)
                );
                for version in [minimum, RubyVersion(3, 4)] {
                    let active = select_active_for_version(&registry, &config, &only, &[], version);
                    assert!(active.iter().any(|c| c.name() == cop));
                    let source = format!("{{}}.each_with_object({{}}) {{ |(k,v),h| {body} }}");
                    assert_eq!(
                        lint_str_with(&active, &source)
                            .iter()
                            .filter(|o| o.cop == cop)
                            .count(),
                        1
                    );
                }
            }
        }
    }
    #[test]
    fn all_hash_receiver_hints_and_non_hash_lookalikes() {
        for receiver in [
            "{}",
            "{a: 1}",
            "x.to_h",
            "x.to_hash",
            "x.merge(y)",
            "x.merge!(y)",
            "x.update(y)",
            "x.invert",
            "x.except(:a)",
            "x.tally",
            "x.group_by { |e| e.type }",
            "x.to_h { |e| [e,e] }",
            "x.tally { |e| e }",
            "x.transform_keys { |k| k }",
            "x.transform_values! { |v| v }",
            "x.each_with_object({}) { |e,h| h[e]=e }",
        ] {
            for body in ["[k.to_s,v]", "[k,v.to_s]"] {
                let s = format!("{receiver}.map {{ |k,v| {body} }}.to_h");
                assert_eq!(transforms(&s).len(), 1, "{s}");
            }
        }
        for receiver in [
            "x",
            "[]",
            "x&.to_h",
            "(x.to_h)",
            "x.merge(y) { |a,b| a }",
            "x.transform_keys(&:to_s)",
            "x.group_by(&:type)",
            "x.each_with_object({a: 1}) { |e,h| h[e]=e }",
            "x.to_enum.select { |e| e.ok? }",
        ] {
            let s = format!("{receiver}.map {{ |k,v| [k.to_s,v] }}.to_h");
            assert!(transforms(&s).is_empty(), "{s}");
        }
    }
    #[test]
    fn hash_producing_block_hints_require_zero_arguments() {
        for method in [
            "group_by",
            "to_h",
            "tally",
            "transform_keys",
            "transform_keys!",
            "transform_values",
            "transform_values!",
        ] {
            for args in ["1", "a: 1", "{a: 1}", "*args", "**kwargs"] {
                for body in ["[k.to_s,v]", "[k,v.to_s]"] {
                    let receiver = format!("x.{method}({args}) {{ |e| e }}");
                    for source in [
                        format!("{receiver}.map {{ |k,v| {body} }}.to_h"),
                        format!("Hash[{receiver}.collect {{ |k,v| {body} }}]"),
                        format!("{receiver}.to_h {{ |k,v| {body} }}"),
                    ] {
                        assert!(transforms(&source).is_empty(), "{source}");
                    }
                }
            }
        }
    }
    #[test]
    fn memo_regexp_options_are_scalar_symbols() {
        for arg in ["k", "v"] {
            for option in ["i", "m", "x", "o", "n", "e", "s", "u"] {
                for expr in [
                    format!("/#{{{arg}}}/{option}"),
                    format!("%r{{#{{{arg}}}}}{option}"),
                    format!("{arg}.to_s + /foo/{option}.to_s"),
                ] {
                    let (left, right) = if arg == "k" {
                        (expr, "v".into())
                    } else {
                        ("k".into(), expr)
                    };
                    let source = format!(
                        "{{}}.each_with_object({{}}) {{ |(k,v),{option}| {option}[{left}] = {right} }}"
                    );
                    assert!(transforms(&source).is_empty(), "{source}");
                }
            }
        }
    }
    #[test]
    fn named_capture_targets_are_not_memo_symbols_but_real_targets_and_reads_are() {
        for (cop, arg) in [
            ("Style/HashTransformKeys", "k"),
            ("Style/HashTransformValues", "v"),
        ] {
            let expr = format!("(/(?<h>.)/ =~ {arg}.to_s).to_s");
            let (left, right) = if arg == "k" {
                (expr, "v".into())
            } else {
                ("k".into(), expr)
            };
            let source = format!("{{}}.each_with_object({{}}) {{ |(k,v),h| h[{left}] = {right} }}");
            let found = transforms(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].cop, found[0].line, found[0].col), (cop, 1, 1));
            assert_eq!(
                found[0].message,
                format!(
                    "Prefer `{}` over `each_with_object`.",
                    if arg == "k" {
                        "transform_keys"
                    } else {
                        "transform_values"
                    }
                )
            );
            assert!(found[0].correction.is_none());
            for expr in [
                format!("(/(?<h>.)/ =~ {arg}.to_s + h.to_s).to_s"),
                format!("(h,z = {arg}).to_s"),
            ] {
                let (left, right) = if arg == "k" {
                    (expr, "v".into())
                } else {
                    ("k".into(), expr)
                };
                let source =
                    format!("{{}}.each_with_object({{}}) {{ |(k,v),h| h[{left}] = {right} }}");
                assert!(transforms(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn block_parameters_bodies_and_argument_channels_remain_structural() {
        for params in [
            "k", "(k,v)", "k,v,x", "k,*v", "k,v=1", "k,v:", "k,**v", "k,v,&b", "k,v; tmp",
            "(k,x),v",
        ] {
            let s = format!("{{}}.map {{ |{params}| [k.to_s,v] }}.to_h");
            assert!(transforms(&s).is_empty(), "{s}");
        }
        for s in [
            "{}.map { [_1.to_s,_2] }.to_h",
            "{}.to_h { it }",
            "{}.map(&:to_s).to_h",
            "{}.map { |k,v| effect; [k.to_s,v] }.to_h",
            "{}.map { |k,v| return [k.to_s,v] }.to_h",
            "{}.map(a: 1) { |k,v| [k.to_s,v] }.to_h",
            "{}.map({a: 1}) { |k,v| [k.to_s,v] }.to_h",
            "{}.to_h(a: 1) { |k,v| [k.to_s,v] }",
            "{}.to_h({a: 1}) { |k,v| [k.to_s,v] }",
            "{}.each_with_object(a: 1) { |(k,v),h| h[k.to_s]=v }",
            "{}.each_with_object({a: 1}) { |(k,v),h| h[k.to_s]=v }",
            "{}.each_with_object({}) { |k,v,h| h[k.to_s]=v }",
            "{}.each_with_object({}) { |(k,*v),h| h[k.to_s]=v }",
            "{}.each_with_object({}) { |(k,v),h| other[k.to_s]=v }",
            "{}.map { |k,v| [k.to_s,(v)] }.to_h",
            "{}.map { |k,v| [k,v] }.to_h",
            "{}.map { |k,v| [k.to_s,v.to_s] }.to_h",
            "{}.map { |k,v| [k.to_s + v,v] }.to_h",
        ] {
            assert!(transforms(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn lexical_descendants_not_variable_liveness_define_pinned_scope_policy() {
        // Upstream counts same-name reads in nested/shadowed scopes. Neither
        // receiver-purity nor VariableForce identity filters may hide these.
        for (body, count) in [
            ("[proc { |k| k.to_s }.call,v]", 1),
            ("[proc { |v| v; k.to_s }.call,v]", 0),
            ("[(k.to_s),v]", 1),
            ("[foo(k),v]", 1),
            ("[42,v]", 0),
            ("[v,v]", 0),
            ("[*k,v]", 0),
            ("[k,*v]", 0),
        ] {
            let s = format!("{{}}.map {{ |k,v| {body} }}.to_h");
            assert_eq!(transforms(&s).len(), count, "{s}");
        }
    }
    #[test]
    fn memo_descendant_pattern_includes_symbols_selectors_writes_and_parameters() {
        for expr in [
            "k.to_s + (object.h ||= 1).to_s",
            "k.to_s + (object.h &&= 1).to_s",
            "k.to_s + (object.h += 1).to_s",
            "h[k.to_s]",
            "h(k)",
            "k.h",
            "k.to_s + :h.to_s",
            "k.to_s + (h = 1).to_s",
            "k.to_s + proc { |h| h }.call.to_s",
            "k.to_s + proc { h = 1 }.call.to_s",
        ] {
            let s = format!("{{}}.each_with_object({{}}) {{ |(k,v),h| h[{expr}]=v }}");
            assert!(transforms(&s).is_empty(), "{s}");
        }
        let s = "{}.each_with_object({}) { |(k,v),h| h[k.to_s + 'h']=v }";
        assert_eq!(transforms(s).len(), 1);
    }
    #[test]
    fn heredoc_membership_uses_ast_edges_not_byte_ranges() {
        for (expr, count) in [("k.to_s", 1), ("k.to_s + v", 0), ("h[k]", 0)] {
            let s = format!(
                "{{}}.each_with_object({{}}) {{ |(k,v),h| h[<<~TEXT]=v }}\n#{{{expr}}}\nTEXT\n"
            );
            assert_eq!(transforms(&s).len(), count, "{s}");
        }
    }
    #[test]
    fn implicit_receiver_blocks_are_not_plain_hash_producing_blocks() {
        for param in ["_1", "it"] {
            for body in ["[k.to_s, v]", "[k, v.to_s]"] {
                let s = format!("x.group_by {{ {param}.type }}.to_h {{ |k,v| {body} }}");
                assert!(
                    !crate::pipeline::lint_str(&s)
                        .iter()
                        .any(|o| o.cop.starts_with("Style/HashTransform")),
                    "{s}"
                );
            }
        }
    }
    #[test]
    fn outer_literal_blocks_are_not_block_pass_arguments() {
        for s in [
            "{}.map { |k,v| [k.to_s,v] }.to_h(&b)",
            "{}.map { |k,v| [k,v.to_s] }.to_h(&b)",
        ] {
            assert!(
                !crate::pipeline::lint_str(s)
                    .iter()
                    .any(|o| o.cop.starts_with("Style/HashTransform")),
                "{s}"
            );
        }
        for namespace in ["Hash", "::Hash", "Other::Hash"] {
            let s = format!("{namespace}[{{}}.map {{ |k,v| [k.to_s,v] }}] do\n foo\nend");
            let found: Vec<_> = crate::pipeline::lint_str(&s)
                .into_iter()
                .filter(|o| o.cop == "Style/HashTransformKeys")
                .collect();
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!(found[0].end_line, 1);
        }
    }
    #[test]
    fn every_applicable_pinned_expectation() {
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/hash_transform_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 85);
        for c in cases {
            let found: Vec<_> = crate::pipeline::lint_str(&c.source)
                .into_iter()
                .filter(|o| o.cop == c.cop)
                .collect();
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                c.positions,
                "{}: {}",
                c.cop,
                c.source
            );
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }
}
