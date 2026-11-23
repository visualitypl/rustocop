//! `Style/SelectByRegexp` — prefer grep/grep_v for regexp predicates.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

use super::select_by::{hash_receiver, one_parenthesis, plain_call, uses_parameter};

pub struct SelectByRegexp;
impl Cop for SelectByRegexp {
    fn name(&self) -> &'static str {
        "Style/SelectByRegexp"
    }
    fn unsafe_fixes(&self) -> bool {
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
        if !matches!(
            call.name().as_slice(),
            b"select" | b"filter" | b"find_all" | b"reject"
        ) || call.receiver().is_some_and(|n| hash_receiver(&n))
        {
            return;
        }
        let Some(block) = call.block().and_then(|n| n.as_block_node()) else {
            return;
        };
        let Some(body) = block.body().and_then(|n| n.as_statements_node()) else {
            return;
        };
        if body.body().len() != 1 {
            return;
        }
        let Some(mut check) = body.body().iter().next() else {
            return;
        };
        let negated = check.as_call_node().is_some_and(|c| {
            plain_call(&c) && c.name().as_slice() == b"!" && ast::call_arguments(&c).is_empty()
        });
        if negated {
            let Some(inner) = check
                .as_call_node()
                .and_then(|c| c.receiver())
                .and_then(one_parenthesis)
            else {
                return;
            };
            check = inner;
        }
        // Named regexp captures are a MatchWriteNode around the =~ call in Prism.
        let Some(predicate) = check
            .as_match_write_node()
            .map(|n| n.call())
            .or_else(|| check.as_call_node())
        else {
            return;
        };
        if !plain_call(&predicate) {
            return;
        }
        let mismatch = predicate.name().as_slice() == b"!~";
        if !matches!(predicate.name().as_slice(), b"match?" | b"=~" | b"!~")
            || (negated && mismatch)
        {
            return;
        }
        let args = ast::call_arguments(&predicate);
        if args.len() != 1
            || (predicate.name().as_slice() == b"match?" && predicate.receiver().is_none())
        {
            return;
        }
        if !predicate
            .receiver()
            .is_some_and(|n| uses_parameter(&block, &n))
            && !uses_parameter(&block, &args[0])
        {
            return;
        }
        let replacement = if (call.name().as_slice() == b"reject") != (negated || mismatch) {
            "grep_v"
        } else {
            "grep"
        };
        // find_regexp: the operand opposite the block variable.
        let text = |n: &Node<'_>| n.location().as_slice().to_vec();
        let regexp: Option<Vec<u8>> = if check.as_match_write_node().is_some() {
            predicate.receiver().map(|n| text(&n))
        } else {
            let lvar = |n: &Node<'_>| {
                n.as_local_variable_read_node().is_some()
                    || n.as_it_local_variable_read_node().is_some()
            };
            let implicit = block.parameters().is_some_and(|p| {
                p.as_numbered_parameters_node().is_some() || p.as_it_parameters_node().is_some()
            });
            let first_parameter = block
                .parameters()
                .and_then(|p| p.as_block_parameters_node())
                .and_then(|p| p.parameters())
                .and_then(|p| p.requireds().iter().next())
                .map(|p| p.location().as_slice().to_vec());
            match predicate.receiver() {
                Some(receiver)
                    if lvar(&receiver)
                        && (implicit
                            || first_parameter.as_deref()
                                == Some(receiver.location().as_slice())) =>
                {
                    args.first().map(text)
                }
                receiver if args.first().is_some_and(lvar) => receiver.map(|n| text(&n)),
                _ => None,
            }
        };
        let correction = regexp
            .filter(|_| cx.corrections_requested())
            .and_then(|regexp| {
                Some(Correction {
                    range: call.message_loc()?.start_offset()..block.closing_loc().end_offset(),
                    replacement: format!("{replacement}({})", String::from_utf8_lossy(&regexp)),
                })
            });
        let (line, col) = file.line_col(call.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(call.location()),
            correction,
            message: format!(
                "Prefer `{replacement}` to `{}` with a regexp match.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop select_by_regexp_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SelectByRegexp")
            .collect()
    }
    #[test]
    fn methods_operands_negations_parameters_receivers_and_exact_messages() {
        for method in ["select", "filter", "find_all", "reject"] {
            for (params, var) in [("|x| ", "x"), ("", "_1"), ("", "it")] {
                for predicate in [
                    format!("{var}.match?(/regexp/)"),
                    format!("/regexp/.match?({var})"),
                    format!("{var} =~ /regexp/"),
                    format!("/regexp/ =~ {var}"),
                    format!("/(?<capture>a)/ =~ {var}"),
                    format!("{var} =~ factory.pattern"),
                    format!("{var} =~ REGEXP"),
                ] {
                    for (expr, negated) in [
                        (predicate.clone(), false),
                        (format!("!({predicate})"), true),
                        (format!("not ({predicate})"), true),
                    ] {
                        for receiver in [
                            "array.",
                            "array&.",
                            "",
                            "[].",
                            "Set.new.",
                            "('a'..'z').",
                            "side_effect().lazy.",
                        ] {
                            let source = format!("{receiver}{method} {{ {params}{expr} }}");
                            let found = offenses(&source);
                            assert_eq!(found.len(), 1, "{source}");
                            assert_eq!((found[0].line, found[0].col), (1, 1));
                            let replacement = if (method == "reject") != negated {
                                "grep_v"
                            } else {
                                "grep"
                            };
                            assert_eq!(
                                found[0].message,
                                format!(
                                    "Prefer `{replacement}` to `{method}` with a regexp match."
                                )
                            );
                            assert!(found[0].correction.is_none());
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn mismatch_and_multiline_positions() {
        for method in ["select", "filter", "find_all", "reject"] {
            for expr in [
                "x !~ /regexp/",
                "/regexp/ !~ x",
                "x !~ other",
                "!x.match?(/regexp/)",
            ] {
                let source = format!("# header\n  array.{method} do |x|\n    {expr}\n  end\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
                let replacement = if method == "reject" { "grep" } else { "grep_v" };
                assert_eq!(
                    found[0].message,
                    format!("Prefer `{replacement}` to `{method}` with a regexp match.")
                );
            }
        }
    }
    #[test]
    fn binding_arity_control_flow_and_predicate_lookalikes() {
        for source in [
            "array.select",
            "array.select(&:even?)",
            "array.select {}",
            "array.select { |x| x.even? }",
            "array.select { |x,y| y.match?(/r/) }",
            "array.select { _1 =~ _2 }",
            "array.select { |x| y.match?(/r/) }",
            "y='a'; array.select { |x| /r/.match?(y) }",
            "array.select { |x| /r/.match?(foo(x)) }",
            "array.select { /r/.match?(foo(_1)) }",
            "array.select { /r/.match?(foo(it)) }",
            "array.select { |x| match?(x) }",
            "it='a'; array.select { it.match?(/r/) }",
            "array.select { |x| next if x.empty?; x.match?(/r/) }",
            "array.select { |x| next x.match?(/r/) }",
            "array.select { |x| return x.match?(/r/) }",
            "array.select { |x| x&.match?(/r/) }",
            "array.select { |x| (x.match?(/r/)) }",
            "array.select { |x| x.match?(/r/, 1) }",
            "array.select { |x| x.match?(/r/) { something } }",
            "array.select { |x| !!x.match?(/r/) }",
            "array.select { |x| !(x !~ /r/) }",
            "array.select { |x| x == /r/ }",
            "array.select { |x| /r/ === x }",
            "array.select { |x| x.match(/r/) }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn exact_parameter_and_negation_shapes() {
        for source in [
            "xs.select { |x,| x.match?(/a/) }",
            "xs.select { |x| x =~ x }",
            "pattern=/a/; xs.select { |x| pattern =~ x }",
            "xs.select { !(/(?<capture>a)/ =~ _1) }",
            "xs.select { not it.match?(/a/) }",
            "xs.select(unusual_argument) { |x| x.match?(/a/) }",
            // The pinned detector does not prove match arity or pattern type.
            "xs.select { |x| x.match?(*patterns) }",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        for source in [
            "xs.select { |x; local| x.match?(/a/) }",
            "xs.select { |x=1| x.match?(/a/) }",
            "xs.select { |*x| x.match?(/a/) }",
            "xs.select { |(x)| x.match?(/a/) }",
            "xs.select { |x,&b| x.match?(/a/) }",
            "xs.select { |x| !(begin; x.match?(/a/); end) }",
            "xs.select { |x| !((x.match?(/a/))) }",
            "xs.select { |x| x.match?(/a/, &callback) }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn hash_and_env_exemptions_are_immediate_and_structural() {
        for receiver in [
            "{}",
            "{foo: :bar}",
            "Hash.new",
            "Hash.new(:default)",
            "Hash.new { |h,k| :default }",
            "Hash[h]",
            "Other::Hash.new",
            "::Hash.new",
            "to_h",
            "foo.to_h",
            "foo.to_hash",
            "ENV",
            "::ENV",
            "Hash&.new",
            "foo&.to_h",
            "Hash.new(&callback)",
        ] {
            let s = format!("{receiver}.select {{ |x| x.match?(/r/) }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        for receiver in [
            "foo.to_h.bar",
            "Other::ENV",
            "Hash",
            "Hash = Other; Hash.new.other",
            "foo.to_h { something }",
            "Hash.new { _1 }",
        ] {
            let s = format!("{receiver}.select {{ |x| x.match?(/r/) }}");
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
    }
}
