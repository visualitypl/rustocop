//! `Style/SafeNavigation` — guarded receiver calls, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Node, StatementsNode};

const MSG: &str =
    "Use safe navigation (`&.`) instead of checking if an object exists before calling the method.";

// Test-only counters prove clause/receiver work stays bounded on wide sources.
#[cfg(test)]
thread_local! {
    static STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct SafeNavigation;
impl Cop for SafeNavigation {
    fn name(&self) -> &'static str {
        "Style/SafeNavigation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut end = node.location().end_offset();
        let candidate = if let Some(and) = node.as_and_node() {
            let rhs = edge_clause(and.right(), true);
            end = rhs.location().end_offset();
            Some((edge_clause(and.left(), false), strip(rhs), false))
        } else {
            if_parts(node)
        };
        let Some((checked, body, ternary)) = candidate else {
            return;
        };
        if !candidate_call(&checked, &body, ternary, file, node, cx) {
            return;
        }
        let start = if node.as_and_node().is_some() {
            checked.location().start_offset()
        } else {
            node.location().start_offset()
        };
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction: None,
        });
    }
}

fn single(statements: Option<StatementsNode<'_>>) -> Option<Node<'_>> {
    let body = statements?.body();
    (body.len() == 1).then(|| body.iter().next().unwrap())
}
fn strip(node: Node<'_>) -> Node<'_> {
    if let Some(paren) = node.as_parentheses_node()
        && let Some(inner) = single(paren.body().and_then(|n| n.as_statements_node()))
    {
        inner
    } else {
        node
    }
}
// Adjacent source-ordered clauses, including conjunctions under grouping and
// disjunctions. Do not descend into blocks or unrelated method arguments.
fn edge_clause(mut node: Node<'_>, first: bool) -> Node<'_> {
    loop {
        #[cfg(test)]
        STEPS.set(STEPS.get() + 1);
        if node.as_parentheses_node().is_some()
            && !grouped_and(&node)
            && !(first && contains_grouped_and(&node))
        {
            break;
        }
        node = strip(node);
        if let Some(and) = node.as_and_node() {
            node = if first { and.left() } else { and.right() };
        } else if first
            && contains_grouped_and(&node)
            && let Some(inner) = first_and(&node)
        {
            // Upstream collects descendant conjunction clauses even within an
            // OR RHS; the original disjunction remains detection-only.
            node = inner;
        } else {
            break;
        }
    }
    node
}
fn contains_grouped_and(node: &Node<'_>) -> bool {
    #[cfg(test)]
    STEPS.set(STEPS.get() + 1);
    if let Some(n) = node.as_parentheses_node() {
        return single(n.body().and_then(|n| n.as_statements_node()))
            .is_some_and(|n| grouped_and(&n) || contains_grouped_and(&n));
    }
    if let Some(n) = node.as_and_node() {
        return contains_grouped_and(&n.left()) || contains_grouped_and(&n.right());
    }
    if let Some(n) = node.as_or_node() {
        return contains_grouped_and(&n.left()) || contains_grouped_and(&n.right());
    }
    false
}
fn first_and<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    #[cfg(test)]
    STEPS.set(STEPS.get() + 1);
    if let Some(and) = node.as_and_node() {
        return Some(and.as_node());
    }
    if let Some(or) = node.as_or_node() {
        return first_and(&or.left()).or_else(|| first_and(&or.right()));
    }
    let inner = single(
        node.as_parentheses_node()?
            .body()
            .and_then(|n| n.as_statements_node()),
    )?;
    first_and(&inner)
}
fn grouped_and(node: &Node<'_>) -> bool {
    if node.as_and_node().is_some() {
        return true;
    }
    node.as_parentheses_node()
        .and_then(|n| single(n.body().and_then(|n| n.as_statements_node())))
        .is_some_and(|n| grouped_and(&n))
}
fn unary_receiver<'pr>(node: &Node<'pr>, name: &[u8]) -> Option<Node<'pr>> {
    let call = node.as_call_node()?;
    (call.name().as_slice() == name
        && ast::call_arguments(&call).is_empty()
        && call.block().is_none()
        && !call.is_safe_navigation())
    .then(|| call.receiver())
    .flatten()
}
fn if_parts<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Node<'pr>, bool)> {
    if let Some(n) = node.as_unless_node() {
        if n.else_clause().is_some() {
            return None;
        }
        let condition = n.predicate();
        let checked =
            unary_receiver(&condition, b"nil?").or_else(|| unary_receiver(&condition, b"!"))?;
        return Some((checked, single(n.statements())?, false));
    }
    let n = node.as_if_node()?;
    let ternary = n.if_keyword_loc().is_none();
    if n.if_keyword_loc().is_some_and(|l| l.as_slice() == b"elsif")
        || (!ternary && n.subsequent().is_some())
    {
        return None;
    }
    let condition = n.predicate();
    let yes = single(n.statements())?;
    let no = n
        .subsequent()
        .and_then(|n| n.as_else_node())
        .and_then(|n| single(n.statements()));
    if ternary && yes.as_nil_node().is_some() {
        let checked =
            unary_receiver(&condition, b"nil?").or_else(|| unary_receiver(&condition, b"!"))?;
        return Some((checked, no?, true));
    }
    if ternary && !no.is_some_and(|n| n.as_nil_node().is_some()) {
        return None;
    }
    let checked = unary_receiver(&condition, b"!")
        .and_then(|inner| unary_receiver(&inner, b"nil?"))
        .unwrap_or(condition);
    Some((checked, yes, ternary))
}
// Normalize equivalent call spellings and regexp option sets at every depth.
// Only the receiver spine may ignore csend/send differences; arguments and
// blocks retain that distinction. These fingerprints never prove fix safety.
fn matching(left: &Node<'_>, right: &Node<'_>, file: &SourceFile) -> bool {
    #[cfg(test)]
    STEPS.set(STEPS.get() + 1);
    if let (Some(a), Some(b)) = (left.as_call_node(), right.as_call_node()) {
        a.name().as_slice() == b.name().as_slice()
            && match (a.receiver(), b.receiver()) {
                (Some(a), Some(b)) => matching(&a, &b, file),
                (None, None) => true,
                _ => false,
            }
            && ast::call_arguments(&a)
                .iter()
                .map(|n| ast::fingerprint_for_detection(n, file))
                .eq(ast::call_arguments(&b)
                    .iter()
                    .map(|n| ast::fingerprint_for_detection(n, file)))
            && a.block().map(|n| ast::fingerprint_for_detection(&n, file))
                == b.block().map(|n| ast::fingerprint_for_detection(&n, file))
    } else {
        ast::fingerprint_for_detection(left, file) == ast::fingerprint_for_detection(right, file)
    }
}
fn candidate_call<'pr>(
    checked: &Node<'pr>,
    body: &Node<'pr>,
    ternary: bool,
    file: &SourceFile,
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
) -> bool {
    let mut current = None;
    let mut calls = Vec::new();
    // At most two calls after the repeated receiver; stop as soon as that
    // fixed bound is exceeded, rather than walking an arbitrary chain.
    for _ in 0..2 {
        let Some(call) = current.as_ref().unwrap_or(body).as_call_node() else {
            return false;
        };
        let Some(receiver) = call.receiver() else {
            return false;
        };
        calls.push(call);
        if matching(checked, &receiver, file) {
            // A literal block wraps its send in RuboCop's AST. For ternaries,
            // the extra dotless-operator scan stops at that wrapper; ordinary
            // if/AND safety checks still examine all outer sends.
            let block_boundary = calls
                .last()
                .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()));
            return calls.iter().enumerate().all(|(index, call)| {
                let dot = call.call_operator_loc().map(|l| l.as_slice());
                (dot != Some(b"::".as_slice()) || (ternary && index + 1 != calls.len()))
                    && (dot.is_some() || (ternary && block_boundary && index + 1 != calls.len()))
                    && (call.is_safe_navigation() || call.name().as_slice() != b"!")
                    && (ternary || !call.is_attribute_write())
                    && (index == calls.len() - 1
                        || call.is_safe_navigation()
                        || !nil_method(call.name().as_slice()))
            }) && (body.as_call_node().is_none_or(|c| {
                c.is_safe_navigation()
                    || c.name().as_slice() != b"empty?"
                    || c.block().is_some_and(|b| b.as_block_node().is_some())
            })) && (!(calls.len() == 1
                || body.as_call_node().is_some_and(|c| {
                    c.is_safe_navigation() || c.block().is_some_and(|b| b.as_block_node().is_some())
                }))
                || !unsafe_enclosing_send(node, cx, ternary));
        }
        current = Some(receiver);
    }
    false
}
// Upstream's ancestor-send safety loop stops when it reaches a *subsequent*
// send equal to the RHS. For a single call, csend or block-wrapped RHS it instead
// reaches enclosing sends. Ask NodeContext rather than rescanning the root.
fn unsafe_enclosing_send<'pr>(node: &Node<'pr>, cx: &NodeContext<'_, 'pr>, ternary: bool) -> bool {
    let mut child = node;
    for parent in cx.ancestors() {
        #[cfg(test)]
        STEPS.set(STEPS.get() + 1);
        if let Some(call) = parent.as_call_node()
            && !call.is_safe_navigation()
            && child.as_block_node().is_none()
            && (call.name().as_slice() == b"!"
                || nil_method(call.name().as_slice())
                || (!ternary
                    && (call.is_attribute_write()
                        || call
                            .call_operator_loc()
                            .is_none_or(|l| l.as_slice() != b"."))))
        {
            return true;
        }
        child = parent;
    }
    false
}
// Fixed Ruby 3.4 NilClass public methods, plus to_d and the pinned
// AllowedMethods. Only subsequent ordinary calls read this set; the first
// guarded call may itself be a nil method (e.g. x && x.to_i).
fn nil_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"!" | b"!="
            | b"!~"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b"^"
            | b"__id__"
            | b"__send__"
            | b"class"
            | b"clone"
            | b"define_singleton_method"
            | b"display"
            | b"dup"
            | b"enum_for"
            | b"eql?"
            | b"equal?"
            | b"extend"
            | b"freeze"
            | b"frozen?"
            | b"hash"
            | b"inspect"
            | b"instance_eval"
            | b"instance_exec"
            | b"instance_of?"
            | b"instance_variable_defined?"
            | b"instance_variable_get"
            | b"instance_variable_set"
            | b"instance_variables"
            | b"is_a?"
            | b"itself"
            | b"kind_of?"
            | b"method"
            | b"methods"
            | b"nil?"
            | b"object_id"
            | b"private_methods"
            | b"protected_methods"
            | b"public_method"
            | b"public_methods"
            | b"public_send"
            | b"rationalize"
            | b"remove_instance_variable"
            | b"respond_to?"
            | b"send"
            | b"singleton_class"
            | b"singleton_method"
            | b"singleton_methods"
            | b"tap"
            | b"then"
            | b"to_a"
            | b"to_c"
            | b"to_enum"
            | b"to_f"
            | b"to_h"
            | b"to_i"
            | b"to_r"
            | b"to_s"
            | b"yield_self"
            | b"|"
            | b"to_d"
            | b"present?"
            | b"blank?"
            | b"presence"
            | b"try"
            | b"try!"
    )
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `style/safe_navigation_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SafeNavigation")
            .collect()
    }
    #[test]
    fn every_applicable_pinned_call_matches_and_all_unsafe_corrections_are_withheld() {
        use crate::cops::testing::correction_offenses;
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/safe_navigation_pinned.json"
        ))
        .unwrap();
        let mut fixes = 0;
        let mut invalid_yields = 0;
        assert_eq!(cases.as_array().unwrap().len(), 707);
        for case in cases.as_array().unwrap() {
            let mut source = case["source"].as_str().unwrap().to_string();
            if case["spec_line"] == 222 && source.contains("yield ") {
                invalid_yields += 1;
                source = format!("def f\n{source}end\n");
            }
            let found = correction_offenses(&SafeNavigation, &source);
            let mut actual: Vec<_> = found.iter().map(|o| (o.line, o.col)).collect();
            let mut expected: Vec<_> = case["positions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_u64().unwrap() as u32, p[1].as_u64().unwrap() as u32))
                .collect();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected, "{case}");
            assert!(
                found
                    .iter()
                    .all(|o| o.message == MSG && o.correction.is_none()),
                "{case}"
            );
            fixes += usize::from(!case["correction"].is_null());
        }
        assert_eq!((fixes, invalid_yields), (493, 6));
    }
    #[test]
    fn ruby_23_minimum_is_explicit_and_active_at_fixed_34() {
        use crate::cops::policy::{FIXED_RUBY_VERSION, RubyVersion};
        let versions = SafeNavigation.ruby_versions();
        assert_eq!(versions.minimum, Some(RubyVersion(2, 3)));
        assert!(!versions.supports(RubyVersion(2, 2)));
        assert!(versions.supports(RubyVersion(2, 3)));
        assert!(versions.supports(FIXED_RUBY_VERSION));
        assert!(!SafeNavigation.safe_fixes());
        assert!(SafeNavigation.default_enabled());
    }
    #[test]
    fn receiver_structure_arguments_binding_and_scopes_are_not_name_only() {
        for source in [
            "x = nil; x && x.foo",
            "self.x && self.x.foo",
            "obj[1] && obj[1].foo",
            "obj(a: 1) && obj(a: 1).foo",
            "@x.foo unless @x.nil?",
            "x&.foo && x.foo.bar",
            "x && x.foo? && x.bar?",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "x && self.x.foo",
            "self.x && x.foo",
            "obj[1] && obj[2].foo",
            "obj(a: 1) && obj(a: 2).foo",
            "x && x(x = nil).foo",
            "def f(x); y && x.foo; end",
            "x && proc { |x| x.foo }",
            "foo && bar { foo && foo.baz }",
        ] {
            // The inner block's own guard is checked, not paired with the
            // unrelated outer clause.
            let count = usize::from(source == "foo && bar { foo && foo.baz }");
            assert_eq!(offenses(source).len(), count, "{source}");
        }
    }
    #[test]
    fn equivalent_receiver_spellings_match_through_arguments_and_blocks() {
        for source in [
            "fetch(/foo/im) && fetch(/foo/mi).bar",
            "fetch(obj[1]) && fetch(obj.[](1)).bar",
            "/foo/im && /foo/mi.bar",
            "fetch(/#{pattern}/im) && fetch(/#{pattern}/mi).bar",
            "fetch([obj[1], /foo/im]) && fetch([obj.[](1), /foo/mi]).bar",
            "fetch { obj[1] } && fetch { obj.[](1) }.bar",
            "obj[1].fetch(/foo/im) && obj.[](1).fetch(/foo/mi).bar",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 1), "{source}");
            assert_eq!(found[0].message, MSG, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for source in [
            "fetch(/foo/im) && fetch(/foo/i).bar",
            "fetch(/foo/im) && fetch(/bar/mi).bar",
            "fetch(obj[1]) && fetch(obj.[](2)).bar",
            "fetch(obj[1]) && fetch(other.[](1)).bar",
            "fetch(obj[1]) && fetch(obj&.[](1)).bar",
            "fetch(/#{pattern}/im) && fetch(/#{other}/mi).bar",
            "fetch { obj[1] } && fetch { obj.[](2) }.bar",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn fixed_chain_length_nil_methods_and_operator_assignment_exemptions() {
        for source in [
            "x && x.a.b",
            "x && x.to_i",
            "x && x.nil?",
            "x && x.a&.nil?",
            "x ? x.*(2) : nil",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "x && x.a.b.c",
            "x && x.a.nil?",
            "x && x.a.to_d",
            "x && x.a.blank?",
            "x && x.empty?",
            "x.empty? if x",
            "x && x[0]",
            "x.bar = y if x",
            "x + y if x",
            "x ? x.foo[0] : nil",
            "if x; x.foo; x.bar; end",
            "if x; x.foo; else; nil; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn wide_conjunctions_and_large_argument_receivers_have_bounded_main_ast_work() {
        use crate::cops::{ast::CALL_ARGUMENT_VISITS, testing::correction_offenses};
        for count in [100, 1000] {
            let source = format!(
                "[{}]",
                std::iter::repeat_n("foo && ((foo.bar && bar) || qux)", count)
                    .collect::<Vec<_>>()
                    .join(",\n")
            );
            STEPS.set(0);
            assert_eq!(correction_offenses(&SafeNavigation, &source).len(), count);
            assert!(STEPS.get() < count * 30, "{} steps", STEPS.get());
            let args = std::iter::repeat_n("1", count)
                .collect::<Vec<_>>()
                .join(", ");
            CALL_ARGUMENT_VISITS.set(0);
            assert_eq!(
                correction_offenses(&SafeNavigation, &format!("foo({args}) && foo({args}).bar"))
                    .len(),
                1
            );
            assert!(CALL_ARGUMENT_VISITS.get() <= count * 4);
        }
    }
    #[test]
    fn source_line_receivers_compare_runtime_line_values_not_token_spelling() {
        for source in [
            "__LINE__ && __LINE__.foo",
            "__LINE__ && 1.foo",
            "f(__LINE__) && f(1).bar",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "__LINE__ &&\n__LINE__.foo",
            "f(__LINE__) &&\nf(__LINE__).bar",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn reports_guarded_calls_at_the_whole_expression() {
        for source in [
            "foo && foo.bar",
            "foo.bar if foo",
            "foo ? foo.bar : nil",
            "foo.bar unless foo.nil?",
            "foo.bar if !foo.nil?",
            "foo.bar unless !foo",
        ] {
            let found = offenses(&format!("def f\n  {source}\nend\n"));
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(
                found[0].message,
                "Use safe navigation (`&.`) instead of checking if an object exists before calling the method."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn parenthesized_checks_and_ternary_outer_colon_calls() {
        for source in [
            "foo && ((foo.bar && bar) || qux)",
            "(foo) && (foo).bar",
            "foo ? foo.one::two : nil",
            "foo.nil? ? nil : foo.one::two",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "foo && foo.one::two",
            "foo.one::two if foo",
            "foo ? foo::one.two : nil",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn adjacent_conjunction_clauses_and_parenthesized_descendants() {
        for (source, columns) in [
            ("foo && foo.bar && foo.bar.baz", vec![1, 8]),
            ("bar && foo && foo.bar", vec![8]),
            ("foo && foo.x? && bar && bar.y?", vec![1, 18]),
            ("foo && (foo.bar? && (foo.baz? && foo.quux?))", vec![1]),
            ("foo && (foo.bar? || (foo.baz? && foo.quux?))", vec![1]),
            ("foo && (bar && (bar.y?))", vec![9]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                columns,
                "{source}"
            );
        }
    }
    #[test]
    fn prism_literal_blocks_csend_negation_and_single_begin_stripping() {
        for source in [
            "foo && foo.empty? {}",
            "foo.empty? {} if foo",
            "foo && foo&.!",
            "foo ? foo&.one&.! : nil",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "foo && ((foo.bar))",
            "foo && foo.empty?(&:x)",
            "foo && foo.!",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn disjunction_requires_grouped_conjunction_and_blocks_bound_ternary_operator_checks() {
        assert!(offenses("foo && (bar || foo.baz && other)").is_empty());
        for source in ["foo ? foo.one {}[0] : nil", "foo ? foo.one {} + 1 : nil"] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        assert!(offenses("foo && foo.one {}[0]").is_empty());
    }
    #[test]
    fn enclosing_send_safety_uses_context_and_respects_prism_block_ownership() {
        for source in [
            "foo && f(x && x.bar)",
            "outer(x.bar if x)",
            "(x && x.bar).nil?",
            "outer(x && x.bar {})",
            "outer(x && x.a&.b)",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "outer(x && x.a.b)",
            "object.outer(x && x.bar)",
            "outer(x ? x.bar : nil)",
            "foo && bar { x && x.bar }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn conjunction_highlight_stops_at_the_adjacent_clause() {
        let found = offenses("foo && (\n  foo.bar &&\n  unrelated\n)");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
        let found = offenses("foo && (\n  foo.bar\n)");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end_line, 3);
    }
    #[test]
    fn ignores_unguarded_different_or_nil_return_changing_checks() {
        for source in [
            "foo.bar",
            "foo&.bar",
            "nil&.bar",
            "foo && bar.baz",
            "foo.nil? || foo.bar",
            "!foo.nil? && foo.bar",
            "foo.bar if foo.respond_to?(:bar)",
            "foo.bar unless foo",
            "foo.bar if !foo",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
