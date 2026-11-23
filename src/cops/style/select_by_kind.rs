//! `Style/SelectByKind` — prefer grep/grep_v for a kind check.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

use super::select_by::{hash_receiver, uses_parameter};
pub struct SelectByKind;
impl Cop for SelectByKind {
    fn name(&self) -> &'static str {
        "Style/SelectByKind"
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
        let Some(mut check) = body.body().iter().next().and_then(|n| n.as_call_node()) else {
            return;
        };
        let negated = check.name().as_slice() == b"!"
            && !check.is_safe_navigation()
            && check.block().is_none()
            && check.arguments().is_none_or(|a| a.arguments().is_empty());
        if negated {
            let Some(inner) = check.receiver().and_then(|n| n.as_call_node()) else {
                return;
            };
            check = inner;
        }
        if check.is_safe_navigation()
            || check.block().is_some()
            || !matches!(check.name().as_slice(), b"is_a?" | b"kind_of?")
            || check.arguments().is_none_or(|a| a.arguments().len() != 1)
            || !check.receiver().is_some_and(|n| uses_parameter(&block, &n))
        {
            return;
        }
        let replacement = if (call.name().as_slice() == b"reject") != negated {
            "grep_v"
        } else {
            "grep"
        };
        let correction = cx
            .corrections_requested()
            .then(|| {
                let class = ast::call_arguments(&check).into_iter().next()?;
                Some(Correction {
                    range: call.message_loc()?.start_offset()..block.closing_loc().end_offset(),
                    replacement: format!(
                        "{replacement}({})",
                        String::from_utf8_lossy(class.location().as_slice())
                    ),
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
                "Prefer `{replacement}` to `{}` with a kind check.",
                String::from_utf8_lossy(call.name().as_slice())
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop select_by_kind_spec.rb (MIT, (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/SelectByKind")
            .collect()
    }
    #[test]
    fn methods_negations_parameters_receivers_and_exact_messages() {
        for method in ["select", "filter", "find_all", "reject"] {
            for kind in ["is_a?", "kind_of?"] {
                for negated in [false, true] {
                    let prefer = if (method == "reject") != negated {
                        "grep_v"
                    } else {
                        "grep"
                    };
                    for (params, var) in [("|x| ", "x"), ("", "_1"), ("", "it")] {
                        for receiver in [
                            "array.",
                            "array&.",
                            "",
                            "[].",
                            "foo.to_a.",
                            "side_effect().lazy.",
                        ] {
                            let s = format!(
                                "{receiver}{method} {{ {params}{}{var}.{kind}(Foo::Bar) }}",
                                if negated { "!" } else { "" }
                            );
                            let found = offenses(&s);
                            assert_eq!(found.len(), 1, "{s}");
                            assert_eq!((found[0].line, found[0].col), (1, 1));
                            assert_eq!(
                                found[0].message,
                                format!("Prefer `{prefer}` to `{method}` with a kind check.")
                            );
                            assert!(found[0].correction.is_none());
                        }
                    }
                }
            }
        }
        let found = offenses("array.select do |x|\n  x.is_a?(Foo)\nend");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 3));
    }
    #[test]
    fn known_hash_receivers_are_exempt() {
        for receiver in [
            "{}",
            "{ foo: :bar }",
            "Hash.new",
            "Hash.new(:default)",
            "Hash.new { |h,k| :default }",
            "Hash[h]",
            "Hash[:foo,0,:bar,1]",
            "to_h",
            "foo.to_h",
            "to_hash",
            "foo.to_hash",
            "ENV",
            "::ENV",
            "Hash&.new",
            "Hash&.new { |h,k| :default }",
            "foo&.to_h",
            "foo&.to_hash",
            "Other::Hash.new",
        ] {
            let s = format!("{receiver}.select {{ |x| x.is_a?(Foo) }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
    #[test]
    fn hash_constructors_and_conversions_with_passed_blocks_are_exempt() {
        // A block pass is a send argument upstream, not a literal block wrapper.
        for receiver in [
            "Hash.new(&callback)",
            "Hash&.new(1, &callback)",
            "Hash.[](&callback)",
            "foo.to_h(&callback)",
            "foo&.to_hash(1, &callback)",
        ] {
            let s = format!("{receiver}.select {{ |x| x.is_a?(Foo) }}");
            assert!(offenses(&s).is_empty(), "{s}");
        }
        for receiver in ["Hash.[] { 1 }", "foo.to_h { 1 }", "foo.to_hash { 1 }"] {
            let s = format!("{receiver}.select {{ |x| x.is_a?(Foo) }}");
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
    }
    #[test]
    fn constructor_exemption_only_wraps_ordinary_blocks_upstream() {
        for receiver in ["Hash.new { _1 }", "Hash.new { it }", "Hash&.new { _1 }"] {
            let s = format!("{receiver}.select {{ |x| x.is_a?(Foo) }}");
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
    }
    #[test]
    fn body_binding_arity_and_control_flow_lookalikes() {
        for s in [
            "array.select",
            "array.select(&:even?)",
            "array.select { |x| x.even? }",
            "array.select { |x,y| y.is_a?(Foo) }",
            "array.select { |x| y.is_a?(Foo) }",
            "y = 1; array.select { |x| y.is_a?(Foo) }",
            "it = 1; array.select { it.is_a?(Foo) }",
            "array.select { _1.is_a?(_2) }",
            "array.select { |x; y| x.is_a?(Foo) }",
            "array.select { |*x| x.is_a?(Foo) }",
            "array.select { |(x)| x.is_a?(Foo) }",
            "array.select { |x| next if x.nil?; x.is_a?(Foo) }",
            "array.select { |x| return x.is_a?(Foo) }",
            "array.select { |x| break x.is_a?(Foo) }",
            "array.select { |x| next x.is_a?(Foo) }",
            "array.select { |x| x&.is_a?(Foo) }",
            "array.select { |x| x.instance_of?(Foo) }",
            "array.select { |x| x.is_a?(Foo,Bar) }",
            "array.select { |x| x.is_a?(Foo) {} }",
            "array.select { |x| (x.is_a?(Foo)) }",
            "array.select { |x| !!x.is_a?(Foo) }",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn dynamic_classes_namespaced_env_and_trailing_comma_follow_pinned_shapes() {
        // Neither runtime constant identity nor === equivalence is proven.
        for s in [
            "Other::ENV.select { |x| x.is_a?(Foo) }",
            "Hash = Custom; array.select { |x| x.is_a?(Hash) }",
            "array.select { |x,| x.is_a?(factory()) }",
            "array.select { |x| not x.kind_of?(klass) }",
            "select(1) { |x| x.is_a?(Foo) }",
        ] {
            let found = offenses(s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found[0].correction.is_none());
        }
    }
}
