//! `Style/CollectionCompact` — fixed empty AllowedReceivers, Ruby >=2.4.
//! Unsafe advice only: receiver dispatch and multi-yield semantics are unknown.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::{BlockNode, CallNode, Node};
pub struct CollectionCompact;
impl Cop for CollectionCompact {
    fn name(&self) -> &'static str {
        "Style/CollectionCompact"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let name = call.name().as_slice();
        if !matches!(
            name,
            b"reject" | b"reject!" | b"select" | b"select!" | b"filter" | b"filter!" | b"grep_v"
        ) {
            return;
        }
        let args = ast::call_arguments(&call);
        let direct = if name == b"grep_v" {
            matches!(args.as_slice(), [arg] if arg.as_nil_node().is_some() || ast::is_global_const(arg, &["NilClass"]))
        } else {
            call.receiver().is_some()
                && matches!(name, b"reject" | b"reject!")
                && matches!(args.as_slice(), [arg] if arg.as_block_argument_node().and_then(|b| b.expression()).and_then(|n| n.as_symbol_node()).is_some_and(|s| s.unescaped() == b"nil?"))
        };
        let end = if direct {
            ast::source_end(node)
        } else {
            if call.receiver().is_none() || !args.is_empty() || name == b"grep_v" {
                return;
            }
            let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
                return;
            };
            if !matches_block(&call, &block) {
                return;
            }
            call.location().end_offset()
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let start = selector.start_offset();
        let good = if name.ends_with(b"!") {
            "compact!"
        } else {
            "compact"
        };
        let bad = single_line(&file.bytes[start..end]);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!("Use `{good}` instead of `{bad}`."),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}
fn no_args(call: &CallNode<'_>) -> bool {
    call.arguments().is_none_or(|a| a.arguments().is_empty()) && call.block().is_none()
}
fn matches_block(call: &CallNode<'_>, block: &BlockNode<'_>) -> bool {
    let Some(body) = block.body().and_then(|b| b.as_statements_node()) else {
        return false;
    };
    if body.body().len() != 1 {
        return false;
    }
    let Some(mut predicate) = body.body().iter().next().and_then(|n| n.as_call_node()) else {
        return false;
    };
    if matches!(
        call.name().as_slice(),
        b"select" | b"select!" | b"filter" | b"filter!"
    ) {
        if predicate.name().as_slice() != b"!" || !no_args(&predicate) {
            return false;
        }
        let Some(inner) = predicate.receiver().and_then(|n| n.as_call_node()) else {
            return false;
        };
        predicate = inner;
    }
    if predicate.name().as_slice() != b"nil?" || !no_args(&predicate) {
        return false;
    }
    let Some(receiver) = predicate.receiver() else {
        return false;
    };
    if receiver.as_local_variable_read_node().is_none()
        && receiver.as_it_local_variable_read_node().is_none()
    {
        return false;
    }
    let Some(params) = block.parameters() else {
        return false;
    };
    if params.as_numbered_parameters_node().is_some() {
        return receiver.location().as_slice() == b"_1";
    }
    if params.as_it_parameters_node().is_some() {
        return receiver.as_it_local_variable_read_node().is_some();
    }
    let Some(params) = params.as_block_parameters_node() else {
        return false;
    };
    let last = params.locals().iter().last().or_else(|| {
        let p = params.parameters()?;
        p.block()
            .map(|b| b.as_node())
            .or_else(|| p.keyword_rest())
            .or_else(|| p.keywords().iter().last())
            .or_else(|| p.posts().iter().last())
            .or_else(|| p.rest().filter(|r| r.as_implicit_rest_node().is_none()))
            .or_else(|| p.optionals().iter().last())
            .or_else(|| p.requireds().iter().last())
    });
    last.is_some_and(|arg| arg.location().as_slice() == receiver.location().as_slice())
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop collection_compact_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == CollectionCompact.name())
            .collect()
    }
    #[test]
    fn fixed_methods_receivers_navigation_and_parameter_forms() {
        for method in [
            "reject", "reject!", "select", "select!", "filter", "filter!",
        ] {
            let neg = if method.starts_with("reject") {
                ""
            } else {
                "!"
            };
            for recv in [
                "array",
                "hash",
                "params",
                "factory()",
                "array.lazy",
                "array.to_enum",
            ] {
                for dot in [".", "&."] {
                    for (param, var) in [("|e| ", "e"), ("|k,v| ", "v"), ("", "_1"), ("", "it")] {
                        let tail = format!("{method} {{ {param}{neg}{var}.nil? }}");
                        let s = format!("# header\n  {recv}{dot}{tail}\n");
                        let o = offenses(&s);
                        assert_eq!(o.len(), 1, "{s}");
                        assert_eq!(
                            (o[0].line, o[0].col, o[0].end_line),
                            (2, (3 + recv.len() + dot.len()) as u32, 2)
                        );
                        let good = if method.ends_with('!') {
                            "compact!"
                        } else {
                            "compact"
                        };
                        assert_eq!(o[0].message, format!("Use `{good}` instead of `{tail}`."));
                        assert!(
                            correction_offenses(&CollectionCompact, &s)[0]
                                .correction
                                .is_none()
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn block_pass_grep_and_global_constants() {
        for s in [
            "a.reject(&:nil?)",
            "a&.reject! &:nil?",
            "a.grep_v(nil)",
            "a&.grep_v(NilClass)",
            "a.grep_v(::NilClass)",
            "grep_v(nil)",
            "a.grep_v(nil) { side_effect }",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
        assert_eq!(
            offenses("a.grep_v(nil) { side_effect }")[0].message,
            "Use `compact` instead of `grep_v(nil)`."
        );
    }
    #[test]
    fn captured_locals_wrong_arity_and_control_flow_are_not_matching_bodies() {
        for s in [
            "a.reject { |k,v| k.nil? }",
            "a.reject { |e| captured.nil? }",
            "e = nil; a.reject { e.nil? }",
            "a.reject { _2.nil? }",
            "a.reject { |(k,v)| v.nil? }",
            "a.reject { |e=nil| e.nil? }",
            "a.reject { |*e| e.nil? }",
            "a.reject { |e:| e.nil? }",
            "a.reject { |&e| e.nil? }",
            "a.reject { |e; x| e.nil? }",
            "a.reject { |e| side_effect; e.nil? }",
            "a.reject { |e| next e.nil? }",
            "a.select { |e| break !e.nil? }",
            "a.reject { |e| return e.nil? }",
            "a.reject { |e| e.nil?(other) }",
            "a.reject { |e| e.nil? { effect } }",
            "a.reject(0) { |e| e.nil? }",
            "a.reject(extra, &:nil?)",
            "a.reject(&block)",
            "a.select(&:nil?)",
            "a.filter(&:nil?)",
            "a.reject { |e| !e.nil? }",
            "a.select { |e| e.nil? }",
            "reject { |e| e.nil? }",
            "select { |e| !e.nil? }",
            "a.delete_if(&:nil?)",
            "a.grep_v(Other::NilClass)",
            "a.grep_v(nil, extra)",
            "a.grep_v(nil, &b)",
            "a.compact",
            "a.reject { |e| (e.nil?) }",
            "a.reject { |e| begin e.nil? end }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn source_parameter_policy_and_unsafe_type_assumptions_are_intentional() {
        // Upstream uses the last argument's *source*, including block-local
        // shadows, not arity/value-flow inference. Never turn this into an edit.
        for s in [
            "a.reject { |e,| e.nil? }",
            "a.reject { |e; x| x.nil? }",
            "[[1,2],[3,nil]].reject { |k,v| v.nil? }",
            "a.reject { |it| it.nil? }",
            "a&.select { |e| e&.nil?&.! }",
            "a.reject do |e|\n e.nil?\nend",
        ] {
            assert_eq!(offenses(s).len(), 1, "{s}");
        }
    }
    #[test]
    fn bounded_local_work_on_many_candidates_and_wide_parameters() {
        let row = "a.reject { |k,v| v.nil? }\na.filter { !it.nil? }\na.grep_v(nil)\n";
        assert_eq!(offenses(&row.repeat(2000)).len(), 6000);
        let params = (0..6000)
            .map(|i| format!("v{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            offenses(&format!("a.reject {{ |{params}| v5999.nil? }}")).len(),
            1
        );
    }
}
