//! `Lint/ToEnumArguments` — current-method enumerator argument forwarding.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ToEnumArguments;
impl Cop for ToEnumArguments {
    fn name(&self) -> &'static str {
        "Lint/ToEnumArguments"
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
        if call.is_safe_navigation()
            || !matches!(call.name().as_slice(), b"to_enum" | b"enum_for")
            || call.receiver().is_some_and(|r| r.as_self_node().is_none())
        {
            return;
        }
        // ANY nearest definition, even across intervening class/block scopes:
        // this is the pinned ancestor matcher, not runtime method resolution.
        let Some(def) = cx.ancestors().find_map(|n| n.as_def_node()) else {
            return;
        };
        let args = ast::call_arguments(&call);
        let Some((selector, args)) = args.split_first() else {
            return;
        };
        let selected = selector.as_call_node().is_some_and(|c| {
            !c.is_safe_navigation()
                && c.receiver().is_none()
                && matches!(c.name().as_slice(), b"__method__" | b"__callee__")
                && c.block().is_none()
                && ast::call_arguments(&c).is_empty()
        }) || symbol(selector, file)
            .is_some_and(|s| file.match_pattern_source(def.name().as_slice()) == s);
        if !selected {
            return;
        }
        let params: Vec<_> = def
            .parameters()
            .map(|p| {
                p.requireds()
                    .iter()
                    .chain(p.optionals().iter())
                    .chain(p.rest())
                    .chain(p.posts().iter())
                    .chain(p.keywords().iter())
                    .chain(p.keyword_rest())
                    .collect()
            })
            .unwrap_or_default();
        let projected = file
            .match_pattern_composes_identifiers()
            .then(|| super::non_atomic_file_operation::projected_local_spans(node, cx, file))
            .flatten();
        let mut index = 0;
        let all_present = params.iter().all(|param| {
            let arg = args.get(index);
            if positional(param) || param.as_rest_parameter_node().is_some() {
                index += 1;
            }
            arg.is_some_and(|a| argument_matches(a, param, cx, file, projected.as_ref()))
        });
        let extra_positional = !params.iter().any(|p| {
            p.as_rest_parameter_node().is_some() || p.as_forwarding_parameter_node().is_some()
        }) && !args
            .iter()
            .any(|a| a.as_splat_node().is_some() || a.as_forwarding_arguments_node().is_some())
            && args
                .iter()
                .filter(|a| {
                    !hash(a)
                        && a.as_assoc_splat_node().is_none()
                        && a.as_block_argument_node().is_none()
                })
                .count()
                > params.iter().filter(|p| positional(p)).count();
        let variadic_keywords = params.iter().any(|p| {
            p.as_keyword_rest_parameter_node().is_some()
                || p.as_forwarding_parameter_node().is_some()
        });
        let expandable_keywords = args.iter().any(|a| {
            a.as_forwarding_arguments_node().is_some()
                || elements(a)
                    .iter()
                    .any(|n| n.as_assoc_splat_node().is_some())
        });
        let extra_keywords = !variadic_keywords
            && !expandable_keywords
            && args
                .iter()
                .flat_map(elements)
                .filter_map(|n| n.as_assoc_node())
                .any(|pair| {
                    symbol(&pair.key(), file).is_some_and(|key| {
                        !params
                            .iter()
                            .filter_map(keyword_name)
                            .any(|name| file.match_pattern_source(name) == key)
                    })
                });
        if all_present && !extra_positional && !extra_keywords {
            return;
        }
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, file);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            message: "Ensure you correctly provided all the arguments.".into(),
            correction: None,
        });
    }
}
fn positional(p: &Node<'_>) -> bool {
    p.as_required_parameter_node().is_some() || p.as_optional_parameter_node().is_some()
}
fn hash(n: &Node<'_>) -> bool {
    n.as_keyword_hash_node().is_some() || n.as_hash_node().is_some()
}
fn elements<'pr>(n: &Node<'pr>) -> Vec<Node<'pr>> {
    if let Some(h) = n.as_keyword_hash_node() {
        h.elements().iter().collect()
    } else if let Some(h) = n.as_hash_node() {
        h.elements().iter().collect()
    } else {
        vec![]
    }
}
fn keyword_name<'pr>(p: &Node<'pr>) -> Option<&'pr [u8]> {
    p.as_required_keyword_parameter_node()
        .map(|n| n.name().as_slice())
        .or_else(|| {
            p.as_optional_keyword_parameter_node()
                .map(|n| n.name().as_slice())
        })
}
fn source(n: &Node<'_>, file: &SourceFile) -> Vec<u8> {
    let bytes = file.match_pattern_source(n.location().as_slice());
    bytes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| (*b != b'\r' || bytes.get(i + 1) != Some(&b'\n')).then_some(*b))
        .collect()
}
// Match the pinned emitted sym, not runtime symbol equality or raw physical
// payload parts. Reuse the existing main-AST projection: it applies buffer
// normalization before token grouping, retains exactly one emitted child's
// bytes (which can exclude a cooked tail), and treats quoted labels separately.
// No candidate parsing, new decoder, or changes to old literal identities.
fn symbol(n: &Node<'_>, file: &SourceFile) -> Option<Vec<u8>> {
    super::symbol_conversion::symbol_value(&n.as_symbol_node()?, file)
}
fn argument_matches(
    a: &Node<'_>,
    p: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    projected: Option<&std::collections::HashSet<(usize, usize)>>,
) -> bool {
    if p.as_required_parameter_node().is_some() || p.as_rest_parameter_node().is_some() {
        return source(a, file) == source(p, file);
    }
    if let Some(p) = p.as_optional_parameter_node() {
        return file.match_pattern_source(p.name().as_slice()) == source(a, file);
    }
    if let Some(name) = keyword_name(p) {
        return a.as_keyword_hash_node().is_some()
            && elements(a)
                .iter()
                .filter_map(|n| n.as_assoc_node())
                .any(|pair| {
                    let value = pair.value();
                    // Shorthand `x:` wraps its local read (or nonlocal send)
                    // in ImplicitNode. Classify the value, not the wrapper;
                    // projected-name/span checks still apply to bare sends.
                    let implicit = value.as_implicit_node().is_some();
                    let value = value.as_implicit_node().map_or(value, |n| n.value());
                    let local = value
                        .as_local_variable_read_node()
                        .map(|n| n.name().as_slice())
                        .or_else(|| {
                            let loc = value.location();
                            projected
                                .filter(|spans| {
                                    spans.contains(&(loc.start_offset(), loc.end_offset()))
                                })
                                .and_then(|_| value.as_call_node())
                                .filter(|c| {
                                    c.is_variable_call()
                                        && c.receiver().is_none()
                                        && c.arguments().is_none()
                                        && c.block().is_none()
                                })
                                .map(|c| c.name().as_slice())
                        })
                        .or_else(|| {
                            implicit
                                .then(|| projected_shorthand_local(&value, cx, file))
                                .flatten()
                        });
                    symbol(&pair.key(), file)
                        .is_some_and(|key| file.match_pattern_source(name) == key)
                        && local.is_some_and(|n| {
                            file.match_pattern_source(n) == file.match_pattern_source(name)
                        })
                });
    }
    if p.as_keyword_rest_parameter_node().is_some() {
        return a.as_keyword_hash_node().is_some()
            && elements(a)
                .iter()
                .any(|n| n.as_assoc_splat_node().is_some() && source(n, file) == source(p, file));
    }
    p.as_forwarding_parameter_node().is_some() && a.as_forwarding_arguments_node().is_some()
}
// Prism's implicit nonlocal send does not carry is_variable_call. Only this
// exact shorthand value may use lexical projection without that flag. Query
// existing VariableForce declarations at the call's scope (the implicit value
// introduces no scope), retaining source-buffer identity and heredoc order.
fn projected_shorthand_local<'pr>(
    value: &Node<'pr>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<&'pr [u8]> {
    if !file.match_pattern_composes_identifiers() {
        return None;
    }
    let call = value.as_call_node()?;
    if call.receiver().is_some() || call.arguments().is_some() || call.block().is_some() {
        return None;
    }
    let root = cx.ancestors().last()?;
    let locals = file.visible_locals(root);
    let name = call.name().as_slice();
    let projected = file.match_pattern_source(name);
    let heredocs = file.heredocs(root);
    let order = super::non_atomic_file_operation::heredoc_order;
    locals
        .declaration_names()
        .any(|alias| {
            alias != name
                && file.match_pattern_source(alias) == projected
                && locals.lexically_declared_before_with(alias, value, cx, |start| {
                    order(heredocs, start) <= order(heredocs, value.location().start_offset())
                })
        })
        .then_some(name)
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop to_enum_arguments_spec.rb (MIT, Copyright 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Lint/ToEnumArguments")
            .collect()
    }
    #[test]
    fn continued_symbols_use_emitted_children_not_raw_or_decoded_emptiness() {
        for payload in ["\\\n", "\\u{}\\\n\\u{}", "\\n\\\n"] {
            let s = format!("def m(); to_enum(:m, {{:\"{payload}\" => 1}}); end\n");
            assert!(offenses(&s).is_empty(), "{s:?}");
        }
    }
    #[test]
    fn one_emitted_symbol_child_counts_even_with_unemitted_cooked_tail() {
        for newline in ["\n", "\r\n", "\r\r\n"] {
            let payload = format!("a{newline}\\n\\{newline}");
            let s = format!("def m(); to_enum(:m, {{:\"{payload}\" => 1}}); end\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s:?}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 10, 3));
            assert_eq!(
                o[0].message,
                "Ensure you correctly provided all the arguments."
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn static_symbol_selectors_labels_and_noninterpolating_neighbours() {
        for key in [":\"\"", ":''", "%s[]", ":\"a\nb\""] {
            assert!(offenses(&format!("def m(); to_enum(:m, {key} => 1); end")).is_empty());
        }
        for key in [":\"\\u{}\"", ":'\\\n'", "%s[\\\n]", "\"\":", "\"\\\n\":"] {
            let separator = if key.ends_with(':') { "" } else { " =>" };
            let s = format!("def m(); to_enum(:m, {key}{separator} 1); end");
            assert_eq!(offenses(&s).len(), 1, "{s:?}");
        }
        for selector in [":\"m\\\n\"", ":'m'", "%s[m]", ":\"\\x6d\""] {
            assert_eq!(
                offenses(&format!("def m(x); to_enum({selector}); end")).len(),
                1
            );
        }
        assert!(offenses("def m(x); to_enum(:\"m\nb\"); end").is_empty());
    }
    #[test]
    fn required_optional_rest_and_keywords() {
        for (params, args) in [
            ("x", ""),
            ("x", ", x, extra"),
            ("x, y=1", ", x"),
            ("*args", ""),
            ("x:", ", x: other"),
            ("x:", ", {x: x}"),
            ("x:", ", x: x, y: 1"),
            ("**kwargs", ""),
        ] {
            let s = format!("def m({params})\n  to_enum(:m{args})\nend\n");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col), (2, 3));
            assert_eq!(
                o[0].message,
                "Ensure you correctly provided all the arguments."
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn shorthand_keywords_forward_required_and_optional_locals() {
        for s in [
            "def m(x:); to_enum(:m, x:); end",
            "def m(x: 1); self.enum_for(__callee__, x:); end",
            "def m(x:, y: 1, **kw); to_enum(:m, x:, y:, **kw); end",
            "def m(x:); proc { to_enum(:m, x:) }; end",
            "# coding: UTF8-MAC\ndef m(e\u{301}:); to_enum(:m, é:); end",
            "# coding: UTF8-MAC\ndef m(é: 1); to_enum(:m, e\u{301}:); end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn shorthand_lookalikes_still_require_matching_keyword_locals() {
        for s in [
            "def m(x:); to_enum(:m, y:); end",
            "def m(x:); to_enum(:m, {x:}); end",
            "def m(x:); class << self; to_enum(:m, x:); end; end",
            "def m(x:); to_enum(:m, x: self.x); end",
            "# coding: UTF-8\ndef m(e\u{301}:); to_enum(:m, é:); end",
            "# coding: UTF-8\ndef m(é: 1); to_enum(:m, e\u{301}:); end",
            "# coding: UTF8-MAC\ndef m(e\u{301}:); class << self; to_enum(:m, é:); end; end",
        ] {
            let o = offenses(s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                "Ensure you correctly provided all the arguments."
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn magic_selector_with_a_literal_block_is_not_a_send_selector() {
        assert!(offenses("def m(x); to_enum(__method__ { nil }); end").is_empty());
    }
    #[test]
    fn projected_descendant_does_not_turn_an_explicit_parent_send_into_a_local() {
        let s = "# coding: UTF8-MAC\ndef m(e\u{301}:); to_enum(:m, e\u{301}: e\u{301}(é)); end";
        assert_eq!(offenses(s).len(), 1);
    }
    #[test]
    fn matching_and_lookalikes() {
        for s in [
            "to_enum(:m)",
            "def m(x); other.to_enum(:m); end",
            "def m(x); self&.to_enum(:m); end",
            "def m(x); to_enum(:other); end",
            "def m(x); to_enum(T.must(__callee__)); end",
            "def m(x, &b); enum_for(__method__, x); end",
            "def m(x,y=1,*args,x:,y:1,**kw,&b); to_enum(__callee__,x,y,*args,x:x,y:y,**kw); end",
            "def m(...); to_enum(:m,...); end",
            "def m(*); to_enum(:m,*); end",
            "def m(**); to_enum(:m,**); end",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
