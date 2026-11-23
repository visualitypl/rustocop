//! `Style/SelectByRange` — prefer grep/grep_v for range checks.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockNode, CallNode, Node};

use super::select_by::{hash_receiver, one_parenthesis, plain_call, uses_parameter};
pub struct SelectByRange;
impl Cop for SelectByRange {
    fn name(&self) -> &'static str {
        "Style/SelectByRange"
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
            b"select" | b"filter" | b"find_all" | b"reject" | b"find" | b"detect"
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
        let Some(mut check) = body.body().iter().next().and_then(|n| n.as_call_node()) else {
            return;
        };
        let negated = plain_call(&check)
            && check.name().as_slice() == b"!"
            && ast::call_arguments(&check).is_empty();
        if negated {
            let Some(inner) = check
                .receiver()
                .and_then(one_parenthesis)
                .and_then(|n| n.as_call_node())
            else {
                return;
            };
            check = inner;
        }
        if !range_check(&check, &block) {
            return;
        }
        let grep = if (call.name().as_slice() == b"reject") != negated {
            "grep_v"
        } else {
            "grep"
        };
        let replacement = if matches!(call.name().as_slice(), b"find" | b"detect") {
            format!("{grep}(...).first")
        } else {
            grep.into()
        };
        // find_range: `min..max` of between?, else the range's source.
        let text = |n: &Node<'_>| String::from_utf8_lossy(n.location().as_slice()).into_owned();
        let range_literal = if check.name().as_slice() == b"between?" {
            let args = ast::call_arguments(&check);
            format!("{}..{}", text(&args[0]), text(&args[1]))
        } else {
            check
                .receiver()
                .and_then(one_parenthesis)
                .map_or_else(String::new, |r| text(&r))
        };
        let suffix = if replacement.ends_with(".first") {
            ".first"
        } else {
            ""
        };
        let correction = cx
            .corrections_requested()
            .then(|| {
                Some(Correction {
                    range: call.message_loc()?.start_offset()..block.closing_loc().end_offset(),
                    replacement: format!("{grep}({range_literal}){suffix}"),
                })
            })
            .flatten();
        let (line, col) = file.line_col(call.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(call.location()),
            correction,
            message: format!(
                "Prefer `{replacement}` to `{}` with a range check.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
        });
    }
}

fn range_check(call: &CallNode<'_>, block: &BlockNode<'_>) -> bool {
    if !plain_call(call) {
        return false;
    }
    let args = ast::call_arguments(call);
    match call.name().as_slice() {
        b"between?" => {
            args.len() == 2 && call.receiver().is_some_and(|n| uses_parameter(block, &n))
        }
        b"cover?" | b"include?" => {
            args.len() == 1
                && uses_parameter(block, &args[0])
                && call
                    .receiver()
                    .and_then(one_parenthesis)
                    .is_some_and(|n| n.as_range_node().is_some())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop select_by_range_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SelectByRange")
            .collect()
    }
    #[test]
    fn methods_predicates_negations_parameters_and_receivers() {
        for method in ["select", "filter", "find_all", "reject", "find", "detect"] {
            for (params, var) in [("|x| ", "x"), ("", "_1"), ("", "it")] {
                for predicate in [
                    format!("{var}.between?(1, 10)"),
                    format!("(1..10).cover?({var})"),
                    format!("(1..10).include?({var})"),
                    format!("(1...10).cover?({var})"),
                    format!("(..10).cover?({var})"),
                    format!("(1..).include?({var})"),
                ] {
                    for (expr, negated) in [
                        (predicate.clone(), false),
                        (format!("!{predicate}"), true),
                        (format!("!({predicate})"), true),
                        (format!("not {predicate}"), true),
                    ] {
                        let grep = if (method == "reject") != negated {
                            "grep_v"
                        } else {
                            "grep"
                        };
                        let replacement = if matches!(method, "find" | "detect") {
                            format!("{grep}(...).first")
                        } else {
                            grep.into()
                        };
                        for receiver in ["array.", "array&.", "", "[].", "side_effect().lazy."] {
                            let source = format!("{receiver}{method} {{ {params}{expr} }}");
                            let found = offenses(&source);
                            assert_eq!(found.len(), 1, "{source}");
                            assert_eq!((found[0].line, found[0].col), (1, 1), "{source}");
                            assert_eq!(
                                found[0].message,
                                format!("Prefer `{replacement}` to `{method}` with a range check."),
                                "{source}"
                            );
                            assert!(found[0].correction.is_none());
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn binding_arity_body_and_control_flow_lookalikes() {
        for source in [
            "array.select",
            "array.select(&:even?)",
            "array.select { |x| x.even? }",
            "array.select { |x| y.between?(1, 10) }",
            "array.select { |x| (1..10).cover?(y) }",
            "y=1; array.select { |x| y.between?(1, 10) }",
            "it=1; array.select { it.between?(1, 10) }",
            "array.select { |x,y| x.between?(1, 10) }",
            "array.select { |x; y| x.between?(1, 10) }",
            "array.select { |*x| x.between?(1, 10) }",
            "array.select { |(x)| x.between?(1, 10) }",
            "array.select { |x=1| x.between?(1, 10) }",
            "array.select { _1.between?(1, _2) }",
            "array.select { |x| next if x.even?; x.between?(1, 10) }",
            "array.select { |x| return x.between?(1, 10) }",
            "array.select { |x| break x.between?(1, 10) }",
            "array.select { |x| next x.between?(1, 10) }",
            "array.select { |x| x&.between?(1, 10) }",
            "array.select { |x| (1..10)&.cover?(x) }",
            "array.select { |x| x.between?(1) }",
            "array.select { |x| x.between?(1, 10, 20) }",
            "array.select { |x| (1..10).cover?(x, y) }",
            "array.select { |x| x.between?(1, 10) {} }",
            "array.select { |x| (x.between?(1, 10)) }",
            "array.select { |x| !!x.between?(1, 10) }",
            "array.select { |x| !((x.between?(1, 10))) }",
            "array.select { |x| ((1..10)).cover?(x) }",
            "array.select { |x| range.cover?(x) }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn hash_receiver_exemptions() {
        for receiver in [
            "{}",
            "{foo: :bar}",
            "Hash.new",
            "::Hash.new(:default)",
            "Other::Hash.new",
            "Hash[h]",
            "Hash.new { |h,k| 0 }",
            "Hash&.new",
            "Hash.new(&callback)",
            "foo.to_h",
            "foo&.to_hash",
            "to_hash(1)",
            "foo.to_h(&callback)",
            "ENV",
            "::ENV",
        ] {
            let source = format!("{receiver}.select {{ |x| x.between?(1, 10) }}");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for receiver in [
            "Other::ENV",
            "foo.to_h { 1 }",
            "Hash.[] { 1 }",
            "Hash.new { _1 }",
            "Hash.new { it }",
        ] {
            let source = format!("{receiver}.select {{ |x| x.between?(1, 10) }}");
            assert_eq!(offenses(&source).len(), 1, "{source}");
        }
    }
    #[test]
    fn dynamic_bounds_trailing_comma_arguments_and_multiline_block() {
        for source in [
            "select(1) { |x| x.between?(minimum(), maximum()) }",
            "array.find(fallback) { |x,| x.between?(x, x) }",
            "array.select { |x| (minimum()..maximum()).include?(x) }",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
        let found = offenses("# hi\n  array.select do |x|\n    x.between?(1, 10)\n  end\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 4));
    }
    #[test]
    fn passed_block_is_a_bound_argument_in_the_pinned_send_shape() {
        for predicate in [
            "x.between?(1, &callback)",
            "!x.between?(1, &callback)",
            "!(x.between?(1, &callback))",
            "not x.between?(1, &callback)",
            "x.between?(1, &callback).!",
        ] {
            let source = format!("xs.select {{ |x| {predicate} }}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none());
        }
        for predicate in [
            "x.between?(1,10,&callback)",
            "(1..10).cover?(x,&callback)",
            "x.between?(1,10).!(&callback)",
        ] {
            let source = format!("xs.select {{ |x| {predicate} }}");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn between_check_and_exact_location_message() {
        let found = offenses("# example\n  array.select { |x| x.between?(1, 10) }\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
        assert_eq!(
            found[0].message,
            "Prefer `grep` to `select` with a range check."
        );
        assert!(found[0].correction.is_none());
    }
}
