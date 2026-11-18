//! `Style/PredicateWithKind` — prefer predicate pattern arguments.
//! Detection only: `===` and kind checks need not have equivalent semantics.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockNode, Node};

pub struct PredicateWithKind;
impl Cop for PredicateWithKind {
    fn name(&self) -> &'static str {
        "Style/PredicateWithKind"
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
            b"any?" | b"all?" | b"none?" | b"one?"
        ) {
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
        let Some(kind) = body.body().iter().next().and_then(|n| n.as_call_node()) else {
            return;
        };
        if kind.is_safe_navigation()
            || kind.block().is_some()
            || !matches!(
                kind.name().as_slice(),
                b"is_a?" | b"kind_of?" | b"instance_of?"
            )
        {
            return;
        }
        let Some(receiver) = kind.receiver() else {
            return;
        };
        if !uses_parameter(&block, &receiver) {
            return;
        }
        let Some(args) = kind.arguments() else { return };
        if args.arguments().len() != 1 {
            return;
        }
        let class = args.arguments().iter().next().unwrap();
        let class = String::from_utf8_lossy(class.location().as_slice());
        let method = String::from_utf8_lossy(call.name().as_slice());
        let (line, col) = file.line_col(call.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(block.location().end_offset() - 1),
            message: format!(
                "Prefer `{method}({class})` to `{method} {{ ... }}` with a kind check."
            ),
            correction: cx
                .corrections_requested()
                .then(|| {
                    Some(Correction {
                        range: call.message_loc()?.start_offset()..block.closing_loc().end_offset(),
                        replacement: format!("{method}({class})"),
                    })
                })
                .flatten(),
        });
    }
}

// Bounded local shape checks only: no subtree walk, reparse or scope scan.
// The receiver must be the block's parameter, not an outer/local lookalike.
fn uses_parameter(block: &BlockNode<'_>, receiver: &Node<'_>) -> bool {
    let Some(parameters) = block.parameters() else {
        return false;
    };
    if let Some(numbered) = parameters.as_numbered_parameters_node() {
        return numbered.maximum() == 1
            && receiver
                .as_local_variable_read_node()
                .is_some_and(|n| n.name().as_slice() == b"_1");
    }
    if parameters.as_it_parameters_node().is_some() {
        return receiver.as_it_local_variable_read_node().is_some();
    }
    let Some(parameters) = parameters.as_block_parameters_node() else {
        return false;
    };
    if !parameters.locals().is_empty() {
        return false;
    }
    let Some(parameters) = parameters.parameters() else {
        return false;
    };
    if parameters.requireds().len() != 1
        || !parameters.optionals().is_empty()
        || parameters
            .rest()
            .is_some_and(|n| n.as_implicit_rest_node().is_none())
        || !parameters.posts().is_empty()
        || !parameters.keywords().is_empty()
        || parameters.keyword_rest().is_some()
        || parameters.block().is_some()
    {
        return false;
    }
    let Some(parameter) = parameters
        .requireds()
        .iter()
        .next()
        .and_then(|n| n.as_required_parameter_node())
    else {
        return false;
    };
    receiver
        .as_local_variable_read_node()
        .is_some_and(|n| n.name().as_slice() == parameter.name().as_slice())
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/predicate_with_kind_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == PredicateWithKind.name())
            .collect()
    }

    fn assert_offense(source: &str, method: &str, class: &str, line: u32, col: u32, end_line: u32) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!(
            (found[0].line, found[0].col, found[0].end_line),
            (line, col, end_line),
            "{source}"
        );
        assert_eq!(
            found[0].message,
            format!("Prefer `{method}({class})` to `{method} {{ ... }}` with a kind check.")
        );
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn pinned_numbered_and_it_parameter_blocks() {
        for method in ["any?", "all?", "none?", "one?"] {
            for parameter in ["_1", "it"] {
                for (kind, class) in [("is_a?", "Integer"), ("kind_of?", "String")] {
                    assert_offense(
                        &format!("array.{method} {{ {parameter}.{kind}({class}) }}"),
                        method,
                        class,
                        1,
                        1,
                        1,
                    );
                }
            }
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes() {
        for method in ["any?", "all?", "none?", "one?"] {
            for source in [
                format!("array.{method}"),
                format!("array.{method}(Integer)"),
                format!("array.{method} {{ |x| x.even? }}"),
                format!("array.{method} do |x|\n next if x.nil?\n x.is_a?(Integer)\nend"),
                format!("array.{method} {{ |x| y.is_a?(Integer) }}"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }

    #[test]
    fn rejects_different_bindings_parameters_and_body_shapes() {
        for source in [
            "array.map { |x| x.is_a?(Integer) }",
            "array.any?(&block)",
            "array.any? { |x| }",
            "array.any? { |x| is_a?(Integer) }",
            "array.any? { |x| x&.is_a?(Integer) }",
            "array.any? { |x| x.is_a? }",
            "array.any? { |x| x.is_a?(Integer, String) }",
            "array.any? { |x| x.is_a?(Integer, &block) }",
            "array.any? { |x| x.is_a?(Integer) { true } }",
            "array.any? { |x| (x.is_a?(Integer)) }",
            "array.any? { |x| begin; x.is_a?(Integer); end }",
            "array.any? { |x| return x.is_a?(Integer) }",
            "array.any? { |x| !x.is_a?(Integer) }",
            "array.any? { |x| (x).is_a?(Integer) }",
            "array.any? { |x| self.x.is_a?(Integer) }",
            "array.any? { |x, y| x.is_a?(Integer) }",
            "array.any? { |(x)| x.is_a?(Integer) }",
            "array.any? { |x = nil| x.is_a?(Integer) }",
            "array.any? { |*x| x.is_a?(Integer) }",
            "array.any? { |x, *rest| x.is_a?(Integer) }",
            "array.any? { |x, **kw| x.is_a?(Integer) }",
            "array.any? { |x, &block| x.is_a?(Integer) }",
            "array.any? { |x:| x.is_a?(Integer) }",
            "array.any? { |x; y| x.is_a?(Integer) }",
            "array.any? { |; x| x.is_a?(Integer) }",
            "array.any? { _2.is_a?(Integer) }",
            "array.any? { _1.is_a?(_2) }",
            "it = Object.new; array.any? { it.is_a?(Integer) }",
            "outer { |it| array.any? { it.is_a?(Integer) } }",
            "outer { |x| array.any? { x.is_a?(Integer) } }",
            "array.any? { x = Object.new; x.is_a?(Integer) }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn arbitrary_class_expressions_and_shadowed_dispatch_are_corrected() {
        use crate::cops::testing::correction_offenses;
        for class in [
            "::Integer",
            "Other::Integer",
            "x",
            "lookup(x)",
            "self.klass",
            "__LINE__",
            "*classes",
            "nil",
            "kind: Integer",
        ] {
            let source = format!(
                "module M\n  Integer = :shadow\n  array.any? {{ |x| x.is_a?({class}) }}\nend"
            );
            assert_offense(&source, "any?", class, 3, 3, 3);
            let found = correction_offenses(&PredicateWithKind, &source);
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].correction.as_ref().unwrap().replacement,
                format!("any?({class})")
            );
        }
        for prefix in [
            "def array.any?(*args); yield Object.new; end",
            "class Object; private :is_a?; end",
            "Object.define_method(:is_a?) { |klass| false }",
            "Integer.define_singleton_method(:===) { |obj| false }",
            "module R; refine Object do; def is_a?(klass); false; end; end; end; using R",
        ] {
            let source = format!("{prefix}\narray.any? {{ |x| x.is_a?(Integer) }}");
            assert_offense(&source, "any?", "Integer", 2, 1, 2);
            assert_eq!(
                correction_offenses(&PredicateWithKind, &source)[0]
                    .correction
                    .as_ref()
                    .unwrap()
                    .replacement,
                "any?(Integer)"
            );
        }
        assert_offense(
            "array.any? { |x,| x.is_a?(Integer) }",
            "any?",
            "Integer",
            1,
            1,
            1,
        );
        assert_offense(
            "array.any? { |it| it.is_a?(Integer) }",
            "any?",
            "Integer",
            1,
            1,
            1,
        );
        // Pinned matcher does not restrict the predicate's arguments.
        assert_offense(
            "array.any?(pattern) { |x| x.is_a?(Integer) }",
            "any?",
            "Integer",
            1,
            1,
            1,
        );
        assert!(!PredicateWithKind.safe_fixes());
    }

    #[test]
    fn source_ranges_cover_the_whole_block_not_chained_calls() {
        assert_offense(
            "consume(\n  array.any? do |x|\n    x.is_a?(Integer)\n  end\n)",
            "any?",
            "Integer",
            2,
            3,
            4,
        );
        assert_offense(
            "  array.any? { |x| x.is_a?(Integer) }.inspect",
            "any?",
            "Integer",
            1,
            3,
            1,
        );
    }

    #[test]
    fn pinned_explicit_parameter_blocks() {
        for method in ["any?", "all?", "none?", "one?"] {
            for (kind, class) in [
                ("is_a?", "Integer"),
                ("kind_of?", "String"),
                ("instance_of?", "Float"),
            ] {
                assert_offense(
                    &format!("array.{method} {{ |x| x.{kind}({class}) }}"),
                    method,
                    class,
                    1,
                    1,
                    1,
                );
            }
            assert_offense(
                &format!("array.{method} do |x|\n  x.is_a?(Integer)\nend"),
                method,
                "Integer",
                1,
                1,
                3,
            );
            assert_offense(
                &format!("{method} {{ |x| x.is_a?(Integer) }}"),
                method,
                "Integer",
                1,
                1,
                1,
            );
            assert_offense(
                &format!("array.{method} {{ |x| x.is_a?(ActiveRecord::Base) }}"),
                method,
                "ActiveRecord::Base",
                1,
                1,
                1,
            );
            assert_offense(
                &format!("array&.{method} {{ |x| x.is_a?(Integer) }}"),
                method,
                "Integer",
                1,
                1,
                1,
            );
        }
    }
}
