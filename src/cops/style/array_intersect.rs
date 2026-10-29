//! `Style/ArrayIntersect`: Ruby >= 3.1 intersection predicates, detection only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct ArrayIntersect;
impl Cop for ArrayIntersect {
    fn name(&self) -> &'static str {
        "Style/ArrayIntersect"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        let captures = if call.block().is_some_and(|b| b.as_block_node().is_some()) {
            block_intersection(&call)
        } else {
            predicate_intersection(&call)
        };
        let Some((receiver, argument, dot, negated)) = captures else {
            return;
        };
        if negated && dot == "&." {
            return;
        }
        let bang = if negated { "!" } else { "" };
        let replacement = format!(
            "{bang}{}{dot}intersect?({})",
            String::from_utf8_lossy(receiver.location().as_slice()),
            String::from_utf8_lossy(argument.location().as_slice())
        );
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `{replacement}` instead of `{}`.",
                String::from_utf8_lossy(node.location().as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
        });
    }
}
type Match<'pr> = (Node<'pr>, Node<'pr>, &'static str, bool);
fn call_dot(call: &CallNode<'_>) -> Option<&'static str> {
    match call.call_operator_loc()?.as_slice() {
        b"." => Some("."),
        b"&." => Some("&."),
        b"::" => Some("::"),
        _ => None,
    }
}
fn no_arguments(call: &CallNode<'_>) -> bool {
    call.arguments().is_none_or(|a| a.arguments().is_empty()) && call.block().is_none()
}
fn predicate_intersection<'pr>(call: &CallNode<'pr>) -> Option<Match<'pr>> {
    let name = call.name().as_slice();
    let (dot, intersection, negated) = if matches!(name, b"any?" | b"none?" | b"empty?") {
        if !no_arguments(call) {
            return None;
        }
        (call_dot(call)?, call.receiver()?, name != b"any?")
    } else {
        if call.block().is_some() {
            return None;
        }
        let args = ast::call_arguments(call);
        let negated = match name {
            b">" | b"!=" | b"==" => {
                let [arg] = args.as_slice() else {
                    return None;
                };
                if !arg
                    .as_integer_node()
                    .is_some_and(|i| i.value().to_u32_digits().1.iter().all(|v| *v == 0))
                {
                    return None;
                }
                name == b"=="
            }
            b"positive?" | b"zero?" if args.is_empty() => name == b"zero?",
            _ => return None,
        };
        let size = call.receiver()?.as_call_node()?;
        if !matches!(size.name().as_slice(), b"count" | b"length" | b"size") || !no_arguments(&size)
        {
            return None;
        }
        (call_dot(&size)?, size.receiver()?, negated)
    };
    let (receiver, argument) = intersection_operands(intersection)?;
    Some((receiver, argument, dot, negated))
}
fn intersection_operands(node: Node<'_>) -> Option<(Node<'_>, Node<'_>)> {
    let (call, operator) = if let Some(parens) = node.as_parentheses_node() {
        let body = parens.body()?.as_statements_node()?;
        if body.body().len() != 1 {
            return None;
        }
        (body.body().first()?.as_call_node()?, true)
    } else {
        (node.as_call_node()?, false)
    };
    // A literal block wraps the send in RuboCop's AST; a block pass is
    // instead an argument, counted below by the shared AST helper.
    if call.block().is_some_and(|b| b.as_block_node().is_some())
        || (operator && (call.is_safe_navigation() || call.name().as_slice() != b"&"))
        || (!operator && call.name().as_slice() != b"intersection")
    {
        return None;
    }
    let mut args = ast::call_arguments(&call);
    if args.len() != 1 {
        return None;
    }
    Some((call.receiver()?, args.pop()?))
}
fn block_intersection<'pr>(call: &CallNode<'pr>) -> Option<Match<'pr>> {
    let negated = match call.name().as_slice() {
        b"any?" => false,
        b"none?" => true,
        _ => return None,
    };
    if call.arguments().is_some_and(|a| !a.arguments().is_empty()) {
        return None;
    }
    let receiver = call.receiver()?;
    let block = call.block()?.as_block_node()?;
    let parameters = block.parameters()?;
    let name = if let Some(parameters) = parameters.as_block_parameters_node() {
        if !parameters.locals().is_empty() {
            return None;
        }
        let parameters = parameters.parameters()?;
        if parameters.requireds().len() != 1
            || !parameters.optionals().is_empty()
            || parameters
                .rest()
                .is_some_and(|r| r.as_implicit_rest_node().is_none())
            || !parameters.posts().is_empty()
            || !parameters.keywords().is_empty()
            || parameters.keyword_rest().is_some()
            || parameters.block().is_some()
        {
            return None;
        }
        parameters
            .requireds()
            .first()?
            .as_required_parameter_node()?
            .name()
            .as_slice()
    } else if parameters
        .as_numbered_parameters_node()
        .is_some_and(|p| p.maximum() == 1)
    {
        b"_1"
    } else if parameters.as_it_parameters_node().is_some() {
        b"it"
    } else {
        return None;
    };
    let body = block.body()?.as_statements_node()?;
    if body.body().len() != 1 {
        return None;
    }
    let member = body.body().first()?.as_call_node()?;
    if member.is_safe_navigation()
        || member.block().is_some()
        || !matches!(member.name().as_slice(), b"include?" | b"member?")
    {
        return None;
    }
    let argument = member.receiver()?;
    if member.name().as_slice() == b"include?" && argument.as_array_node().is_none() {
        return None;
    }
    let args = ast::call_arguments(&member);
    let [arg] = args.as_slice() else {
        return None;
    };
    let matches = if name == b"it" && parameters.as_it_parameters_node().is_some() {
        arg.as_it_local_variable_read_node().is_some()
    } else {
        arg.as_local_variable_read_node()
            .is_some_and(|r| r.name().as_slice() == name)
    };
    matches.then_some((receiver, argument, call_dot(call)?, negated))
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop array_intersect_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov). ActiveSupport stays disabled.
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ArrayIntersect.name())
            .collect()
    }
    fn diagnostic(s: &str, replacement: &str) {
        let o = offenses(&format!("# header\n  {s}\n"));
        assert_eq!(o.len(), 1, "{s}");
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, 3, 2), "{s}");
        assert_eq!(
            o[0].message,
            format!("Use `{replacement}` instead of `{s}`.")
        );
        let fixes = crate::cops::testing::correction_offenses(&ArrayIntersect, s);
        assert_eq!(fixes.len(), 1);
        assert!(fixes[0].correction.is_none());
    }
    #[test]
    fn pinned_predicates_and_exact_messages() {
        for base in ["(a & b)", "a.intersection(b)"] {
            for (predicate, bang) in [("any?", ""), ("none?", "!"), ("empty?", "!")] {
                diagnostic(
                    &format!("{base}.{predicate}"),
                    &format!("{bang}a.intersect?(b)"),
                );
            }
        }
        diagnostic("a&.intersection(b)&.any?", "a&.intersect?(b)");
        diagnostic(
            "(customer_country_codes & SUPPORTED_COUNTRIES).empty?",
            "!customer_country_codes.intersect?(SUPPORTED_COUNTRIES)",
        );
    }
    #[test]
    fn every_pinned_size_comparison_and_navigation() {
        for method in ["count", "length", "size"] {
            for base in ["(a & b)", "a.intersection(b)"] {
                for (suffix, bang) in [
                    (" > 0", ""),
                    (" != 0", ""),
                    (" == 0", "!"),
                    (".zero?", "!"),
                    (".positive?", ""),
                ] {
                    diagnostic(
                        &format!("{base}.{method}{suffix}"),
                        &format!("{bang}a.intersect?(b)"),
                    );
                }
            }
            diagnostic(
                &format!("a&.intersection(b)&.{method}&.positive?"),
                "a&.intersect?(b)",
            );
        }
    }
    #[test]
    fn sole_block_pass_operands_have_exact_predicate_and_size_diagnostics() {
        // The pinned single-argument wildcard includes a BlockPass, even &nil.
        // These are unsafe suggestions, not executable replacements or edits.
        for argument in ["&b", "&nil", "&:foo", "&factory()"] {
            for base in [
                format!("a.intersection({argument})"),
                format!("a&.intersection({argument})"),
                format!("(a.&({argument}))"),
            ] {
                for (tail, dot, bang) in [
                    (".any?", ".", ""),
                    (".empty?", ".", "!"),
                    (".none?", ".", "!"),
                    ("&.any?", "&.", ""),
                    (".size > 0", ".", ""),
                    (".length == 0", ".", "!"),
                    ("&.count.positive?", "&.", ""),
                    (".size.zero?", ".", "!"),
                ] {
                    diagnostic(
                        &format!("{base}{tail}"),
                        &format!("{bang}a{dot}intersect?({argument})"),
                    );
                }
            }
        }
    }
    #[test]
    fn anonymous_block_pass_operands_keep_binding_and_exact_ranges() {
        for base in ["a.intersection(&)", "a&.intersection(&)", "(a.&(&))"] {
            for (tail, dot, bang) in [
                (".any?", ".", ""),
                (".empty?", ".", "!"),
                ("&.any?", "&.", ""),
                (".size > 0", ".", ""),
                ("&.count.positive?", "&.", ""),
            ] {
                let call = format!("{base}{tail}");
                let chain = if tail.contains('>') { "" } else { ".to_s" };
                let source = format!("def f(&)\n  {call}{chain}\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    format!("Use `{bang}a{dot}intersect?(&)` instead of `{call}`.")
                );
                let fixes = crate::cops::testing::correction_offenses(&ArrayIntersect, &source);
                assert_eq!(fixes.len(), 1);
                assert!(fixes[0].correction.is_none());
            }
        }
        let source = "# header\n  a.intersection(\n    &factory()\n  ).any?.to_s\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
        assert_eq!(
            found[0].message,
            "Use `a.intersect?(&factory())` instead of `a.intersection(\n    &factory()\n  ).any?`."
        );
        assert!(found[0].correction.is_none());
    }
    #[test]
    fn block_pass_arity_literal_block_operator_csend_and_negation_exclusions() {
        for argument in ["&b", "&nil", "&:foo", "&factory()"] {
            for base in [
                format!("a.intersection(b, {argument})"),
                format!("a&.intersection(b, {argument})"),
                format!("(a.&(b, {argument}))"),
                format!("(a&.&({argument}))"),
                "a.intersection(b) { effect }".to_string(),
                "a.intersection { effect }".to_string(),
                "(a.&(b) { effect })".to_string(),
            ] {
                for tail in [".any?", ".empty?", "&.any?", ".size > 0", ".length == 0"] {
                    let source = format!("{base}{tail}");
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
            for base in [
                format!("a.intersection({argument})"),
                format!("(a.&({argument}))"),
            ] {
                for tail in [
                    "&.empty?",
                    "&.none?",
                    "&.size.zero?",
                    "&.length == 0",
                    ".any?(&b)",
                    ".any? { effect }",
                ] {
                    let source = format!("{base}{tail}");
                    assert!(offenses(&source).is_empty(), "{source}");
                }
            }
        }
        assert!(offenses("def f(&)\n a.intersection(b, &).any?\n (a&.&(&)).any?\n a.intersection(&)&.none?\nend").is_empty());
    }
    #[test]
    fn pinned_blocks_explicit_numbered_and_it() {
        for method in ["any?", "none?"] {
            let bang = if method == "none?" { "!" } else { "" };
            for (params, var) in [("|e| ", "e"), ("", "_1"), ("", "it")] {
                for (other, membership) in [
                    ("b", "member?"),
                    ("[1, 2]", "include?"),
                    ("%w[foo bar]", "include?"),
                    ("[]", "include?"),
                    ("%i[a b]", "include?"),
                ] {
                    diagnostic(
                        &format!("a.{method} {{ {params}{other}.{membership}({var}) }}"),
                        &format!("{bang}a.intersect?({other})"),
                    );
                }
            }
        }
        diagnostic("a&.any? { |e| b.member?(e) }", "a&.intersect?(b)");
        diagnostic("a.any? { |e,| b.member?(e) }", "a.intersect?(b)");
    }
    #[test]
    fn pinned_no_offenses_and_arity_block_lookalikes() {
        for s in [
            "(a & b).any? { |x| false }",
            "(a & b).any?(&:block)",
            "(a & b).any? { do_something(_1) }",
            "([1] & [2]).present?",
            "([1] & [2]).blank?",
            "a.intersection(b).present?",
            "a.intersection(b).blank?",
            "a.intersection.any?",
            "a.intersection(b, c).any?",
            "intersection(b).any?",
            "intersection(b).size == 0",
            "a&.intersection(b)&.none?",
            "a&.intersection(b)&.empty?",
            "a&.intersection(b)&.size.zero?",
            "a.any? { |e| b.include?(e) }",
            "a.any? { |e| CONST.include?(e) }",
            "a.any? { |e| include?(e) }",
            "a.any? { |e| member?(e) }",
            "a.none? { member?(_1) }",
            "a&.none? { |e| b.member?(e) }",
            "a&.none? { b.include?(_1) }",
            "a.any? { |x, y| b.member?(x) }",
            "a.any? { |x = 1| b.member?(x) }",
            "a.any? { |x; y| b.member?(x) }",
            "a.any? { |x| b.member?(other) }",
            "a.any? { b.member?(_2) }",
            "a.any?(1) { b.member?(_1) }",
            "a.any? { |e| effect; b.member?(e) }",
            "a.any? { |e| (b.member?(e)) }",
            "a.any? { |e| b&.member?(e) }",
            "(a & b).any?(1)",
            "(a & b).count(1) > 0",
            "(a & b).count { true }.positive?",
            "a.intersection(b, &blk).any?",
            "((a & b)).any?",
            "(a.intersection(b)).any?",
            "a.intersect?(b)",
            "a & b",
            "a.intersection(b).size >= 0",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
        for method in ["count", "length", "size"] {
            for base in ["(a & b)", "a.intersection(b)"] {
                for op in [">", "=="] {
                    assert!(offenses(&format!("{base}.{method} {op} 1")).is_empty());
                }
            }
        }
    }
    #[test]
    fn fixed_ruby_gate_and_dispatch_operator_spelling() {
        use crate::cops::policy::RubyVersion;
        assert!(!ArrayIntersect.ruby_versions().supports(RubyVersion(3, 0)));
        assert!(ArrayIntersect.ruby_versions().supports(RubyVersion(3, 1)));
        assert!(ArrayIntersect.ruby_versions().supports(RubyVersion(3, 4)));
        diagnostic("a.intersection(b)::any?", "a::intersect?(b)");
        diagnostic("a.intersection(b)::size.positive?", "a::intersect?(b)");
        diagnostic("a::any? { |e| b.member?(e) }", "a::intersect?(b)");
        // Fixed matcher gates on the final predicate/size dot, not type flow.
        diagnostic("a&.intersection(b).any?", "a.intersect?(b)");
        diagnostic("a.intersection(b)&.any?", "a&.intersect?(b)");
    }
    #[test]
    fn source_spelling_precedence_and_multiline_highlight() {
        diagnostic("(a & b & c).any?", "a & b.intersect?(c)"); // pinned suggestion, NOT a safe edit
        let o = offenses("a.intersection(b).any?.to_s");
        assert_eq!(o.len(), 1);
        assert_eq!(
            o[0].message,
            "Use `a.intersect?(b)` instead of `a.intersection(b).any?`."
        );
        let o = offenses("  (a &\n   b).any?\n");
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].line, o[0].col, o[0].end_line), (1, 3, 2));
        assert_eq!(
            o[0].message,
            "Use `a.intersect?(b)` instead of `(a &\n   b).any?`."
        );
    }
}
