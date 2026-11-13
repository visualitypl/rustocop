//! `Lint/UselessAssignment` — flags assignments to local variables whose
//! value is never read.
//!
//! Matches RuboCop's semantics, on the scope analysis in
//! [`crate::cops::variables`] (a port of RuboCop's `VariableForce`): an
//! assignment is useless unless a reference can read its value (directly,
//! from another branch, on a later loop iteration, after an exception, from
//! a block that captures the variable, or through zero-arity `super` and
//! `binding`). As in RuboCop:
//!
//! - Variables named `_` or `_foo` are never reported, nor are parameters
//!   (only their reassignments) or pattern variables.
//! - An assignment in a loop is not reported when the loop's condition reads
//!   the variable.
//! - In `a = b = value` and `a = call(b = value)`, only the outer assignment
//!   is reported when both are useless.
//! - The message suggests `_name` for multiple-assignment targets, the plain
//!   operator for an operator assignment that is a scope's last expression
//!   (`Use `||` instead of `||=`.`), and otherwise a similarly named
//!   variable or method, as RuboCop's `DidYouMean`-based suggestion does.
//!
//! The offense is reported at the variable name (a named capture's regexp).

use std::ops::Range;

use ruby_prism::{Node, Visit};

use crate::{
    cops::{
        Cop, NodeContext,
        name_similarity::find_similar_name,
        variables::{Assignment, AssignmentKind, ScopeAnalysis, ScopeId},
    },
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/UselessAssignment` cop.
pub struct UselessAssignment;

impl Cop for UselessAssignment {
    fn name(&self) -> &'static str {
        "Lint/UselessAssignment"
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(program) = node.as_program_node() else {
            return;
        };
        let analysis = ScopeAnalysis::for_useless_assignments(&program, file);
        // Assignments whose nested assignments are not reported (RuboCop's
        // `ignore_node`), across scopes, in RuboCop's checking order.
        let mut ignored: Vec<Range<usize>> = Vec::new();
        for scope in analysis.scopes_in_exit_order() {
            let mut similar_names: Option<Vec<String>> = None;
            for variable in analysis.scope(scope).variables() {
                let variable = analysis.variable(variable);
                if variable.should_be_unused() {
                    continue;
                }
                for &id in variable.assignments.iter().rev() {
                    if analysis.is_used(id) {
                        continue;
                    }
                    let assignment = analysis.assignment(id);
                    let extent = extent(assignment);
                    if ignored
                        .iter()
                        .any(|outer| outer.start <= extent.start && extent.end <= outer.end)
                    {
                        continue;
                    }
                    if reads_in_loop_condition(assignment, variable.name, file) {
                        continue;
                    }
                    let name = reference_name(variable.name, file);
                    let mut message = format!("Useless assignment to variable - `{name}`.");
                    match assignment.kind {
                        AssignmentKind::Multiple => message.push_str(&format!(
                            " Use `_` or `_{name}` as a variable name to indicate that it won't be used."
                        )),
                        AssignmentKind::Operator => {
                            if is_return_value(&analysis, scope, assignment) {
                                let operator = operator(assignment);
                                let plain = operator.strip_suffix('=').unwrap_or(&operator);
                                message.push_str(&format!(
                                    " Use `{plain}` instead of `{operator}`."
                                ));
                            }
                        }
                        _ => {
                            let names = similar_names.get_or_insert_with(|| {
                                analysis
                                    .variable_like_names(scope)
                                    .into_iter()
                                    .map(|name| reference_name(name, file))
                                    .collect()
                            });
                            let names: Vec<&str> = names.iter().map(String::as_str).collect();
                            if let Some(similar) = find_similar_name(&name, &names) {
                                message.push_str(&format!(" Did you mean `{similar}`?"));
                            }
                        }
                    }
                    let (line, col) = super::array_literal_in_regexp::reference_line_col(
                        assignment.name_range.start,
                        file,
                    );
                    out.push(Offense {
                        correction: None,
                        cop: self.name(),
                        severity: self.severity(),
                        message,
                        line,
                        end_line: file.line_of(assignment.name_range.end.saturating_sub(1)),
                        col,
                    });
                    if is_chained(assignment) {
                        ignored.push(extent);
                    }
                }
            }
        }
    }
}

/// Project names through the same source buffer as the pinned parser before
/// rendering or similarity lookup. Other VariableForce consumers keep raw identities.
fn reference_name(name: &[u8], file: &SourceFile) -> String {
    file.match_pattern_source(name)
        .utf8_chunks()
        .map(|chunk| {
            let mut text = chunk.valid().to_owned();
            for byte in chunk.invalid() {
                text.push_str(&format!("\\x{byte:02X}"));
            }
            text
        })
        .collect()
}

/// The range of the assignment node in RuboCop's AST: an operator
/// assignment's is just the variable name.
fn extent(assignment: &Assignment<'_>) -> Range<usize> {
    if assignment.kind == AssignmentKind::Operator {
        return assignment.name_range.clone();
    }
    let location = assignment.node.location();
    location.start_offset()..location.end_offset()
}

/// RuboCop's `chained_assignment?`: `a = b = 1`, or a value that is a
/// method call (`a = foo(b = 1)`, `a = -b = 1`).
fn is_chained(assignment: &Assignment<'_>) -> bool {
    let Some(write) = assignment.node.as_local_variable_write_node() else {
        return false;
    };
    match write.value() {
        Node::LocalVariableWriteNode { .. } => true,
        Node::CallNode { .. } => {
            let value = write.value();
            let call = value.as_call_node().unwrap();
            // `a&.b`, calls with a literal block and `/re/ =~ x`
            // (`match_with_lvasgn`) are not `send` nodes.
            let regexp_match = call.name().as_slice() == b"=~"
                && call
                    .receiver()
                    .is_some_and(|receiver| receiver.as_regular_expression_node().is_some());
            !regexp_match
                && !call.is_safe_navigation()
                && !call
                    .block()
                    .is_some_and(|block| block.as_block_node().is_some())
        }
        _ => false,
    }
}

/// RuboCop's `variable_in_loop_condition?`: the nearest enclosing `while` or
/// `until` reads the variable in its condition.
fn reads_in_loop_condition(assignment: &Assignment<'_>, name: &[u8], file: &SourceFile) -> bool {
    struct Reads<'n> {
        name: &'n [u8],
        file: &'n SourceFile,
        found: bool,
    }
    impl<'pr> Visit<'pr> for Reads<'_> {
        fn visit_local_variable_read_node(
            &mut self,
            node: &ruby_prism::LocalVariableReadNode<'pr>,
        ) {
            self.found |= self.file.match_pattern_source(node.name().as_slice())
                == self.file.match_pattern_source(self.name);
        }
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            if self.file.match_pattern_composes_identifiers()
                && node.is_variable_call()
                && node.receiver().is_none()
                && node.arguments().is_none()
                && node.block().is_none()
            {
                self.found |= self.file.match_pattern_source(node.name().as_slice())
                    == self.file.match_pattern_source(self.name);
            }
            ruby_prism::visit_call_node(self, node);
        }
    }
    let Some(condition) = &assignment.loop_condition else {
        return false;
    };
    let mut reads = Reads {
        name,
        file,
        found: false,
    };
    reads.visit(condition);
    reads.found
}

/// Whether an operator assignment is its scope's last expression (RuboCop's
/// `return_value_node_of_scope`).
fn is_return_value(
    analysis: &ScopeAnalysis<'_>,
    scope: ScopeId,
    assignment: &Assignment<'_>,
) -> bool {
    let Some(body) = analysis.scope(scope).body() else {
        return false;
    };
    let Some(statements) = body.as_statements_node() else {
        return false;
    };
    let statements = statements.body();
    let mut last = statements.iter().last();
    // A lone parenthesized body is RuboCop's `begin`: its last child.
    if statements.iter().nth(1).is_none()
        && let Some(only) = &last
        && let Some(parentheses) = only.as_parentheses_node()
    {
        last = parentheses
            .body()
            .and_then(|body| body.as_statements_node())
            .and_then(|body| body.body().iter().last());
    }
    let Some(last) = last else {
        return false;
    };
    let (a, b) = (last.location(), assignment.node.location());
    (a.start_offset(), a.end_offset()) == (b.start_offset(), b.end_offset())
}

/// The operator of an operator assignment, e.g. `||=`.
fn operator(assignment: &Assignment<'_>) -> String {
    let node = &assignment.node;
    let operator = if let Some(write) = node.as_local_variable_operator_write_node() {
        write.binary_operator_loc()
    } else if let Some(write) = node.as_local_variable_or_write_node() {
        write.operator_loc()
    } else if let Some(write) = node.as_local_variable_and_write_node() {
        write.operator_loc()
    } else {
        return String::new();
    };
    String::from_utf8_lossy(operator.as_slice()).into_owned()
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/useless_assignment_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). One example the
    //! spec marks pending (RuboCop misses it) is left out.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let result = ruby_prism::parse(source.as_bytes());
        assert_eq!(result.errors().count(), 0, "fixture must parse: {source:?}");
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessAssignment")
            .collect()
    }

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    fn message(name: &str) -> String {
        format!("Useless assignment to variable - `{name}`.")
    }

    /// A RuboCop spec example: its description, source and offenses.
    struct Case {
        desc: &'static str,
        source: &'static str,
        offenses: &'static [(u32, u32, &'static str)],
    }

    #[test]
    fn rubocop_spec_examples() {
        let mut failures = Vec::new();
        for case in RUBOCOP_SPEC {
            let expected: Vec<(u32, u32, String)> = case
                .offenses
                .iter()
                .map(|&(line, col, message)| (line, col, message.to_string()))
                .collect();
            let found = found(case.source);
            if found != expected {
                failures.push(format!(
                    "{}\n{}\n  expected: {expected:?}\n     found: {found:?}",
                    case.desc, case.source
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} examples differ:\n\n{}",
            failures.len(),
            RUBOCOP_SPEC.len(),
            failures.join("\n\n")
        );
    }

    #[test]
    fn parallel_rhs_captures_keep_target_values_live() {
        // Ruby lexical binding evidence, not upstream's known
        // VariableForce omission: these closures can observe the later value.
        for source in [
            "a, b = [-> { b }, 2]; p a.call",
            "a, b = [->(x = b) { x }, 2]; p a.call",
            "a, b = [-> { -> { b } }, 2]; p a.call.call",
            "a, b = [proc { b }, 2]; p a.call",
            "a, b = [-> { def (build(<<~TXT)).m; end }, 2]; p a\n#{b}\nTXT\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}: {:?}", found(source));
        }
        // An eager RHS read must not keep the later assignment alive.
        assert_eq!(positions("a, b = b, 2; p a"), [(1, 4)]);
        let source = "a, b = [->(b) { b }, 2]; p a";
        let result = found(source);
        assert_eq!(
            result,
            [(
                1,
                4,
                format!(
                    "{} Use `_` or `_b` as a variable name to indicate that it won't be used.",
                    message("b")
                )
            )]
        );
    }

    #[test]
    fn eager_parallel_rhs_read_does_not_hide_unused_closure_writes() {
        let source = "a, b = [b, -> { b = 3 }]; p a";
        assert_eq!(
            found(source),
            [
                (
                    1,
                    4,
                    format!(
                        "{} Use `_` or `_b` as a variable name to indicate that it won't be used.",
                        message("b")
                    )
                ),
                (1, 17, message("b")),
            ]
        );
        assert_eq!(
            positions("a, b = [nil, -> { b = 3 }]; p a"),
            [(1, 4), (1, 19)]
        );
        for source in [
            "a, b = [b, -> { b = 3 }]; p b",
            "a, b = [-> { b }, -> { b = 3 }]; p a",
            "a, b = [->(x = b) { x }, -> { b = 3 }]; p a",
            "a, b = [-> { binding }, -> { b = 3 }]; p a",
        ] {
            assert!(
                !offenses(source).iter().any(|o| o.message.contains("`b`")),
                "{source}: {:?}",
                found(source)
            );
        }
    }

    #[test]
    fn named_capture_offense_spans_the_complete_regexp() {
        let found = offenses("/(?<foo>\na\n)/ =~ x\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 3));
        assert_eq!(found[0].message, message("foo"));
    }

    #[test]
    fn mixed_mac_names_share_liveness_but_utf8_names_do_not() {
        assert!(offenses("# coding: UTF8-MAC\né = 1; p e\u{301}\n").is_empty());
        assert_eq!(
            positions("# coding: UTF8-MAC\né = 1; e\u{301} = 2; p é\n"),
            vec![(2, 1)]
        );
        assert_eq!(
            positions("# coding: UTF-8\né = 1; p e\u{301}\n"),
            vec![(2, 1)]
        );
    }

    #[test]
    fn operator_recovery_greek_sigma_suggestion_uses_ruby_case_mapping() {
        let found = offenses("aΟΣ = 1\naΟσ = 2\naΟς = 3\np aΟσ, aΟς\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 1));
        assert_eq!(
            found[0].message,
            "Useless assignment to variable - `aΟΣ`. Did you mean `aΟσ`?"
        );
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn encoded_variable_names_use_the_reference_buffer() {
        assert_eq!(
            found("# coding: UTF8-MAC\ne\u{301} = 1\n"),
            [(2, 1, message("é"))]
        );
    }

    #[test]
    fn flags_unused_assignment_with_message() {
        assert_eq!(found("def m\n  foo = 1\nend\n"), [(2, 3, message("foo"))]);
    }

    #[test]
    fn reports_only_the_outer_of_nested_useless_assignments() {
        // A chained assignment, or one whose value is a method call, hides
        // the assignments inside it.
        for source in [
            "x = a[y = 1]\n",
            "x = foo(y = 1)\n",
            "x = foo(y = 1, &b)\n",
            "x = !(y = 1)\n",
            "a = b = c = 1\n",
        ] {
            assert_eq!(positions(source), [(1, 1)], "for {source:?}");
        }
        // Safe navigation, literal blocks and parentheses do not.
        assert_eq!(positions("x = foo&.bar(y = 1)\n"), [(1, 1), (1, 14)]);
        assert_eq!(positions("x = foo(y = 1) { }\n"), [(1, 1), (1, 9)]);
        assert_eq!(positions("x = (y = 1)\n"), [(1, 1), (1, 6)]);
        // `/re/ =~ …` is `match_with_lvasgn` in RuboCop's AST, not a `send`.
        assert_eq!(
            positions("def m1\n  a = /x/ =~ foo(b = 1)\nend\n"),
            [(2, 3), (2, 18)]
        );
        assert_eq!(positions("x = /#{z}/ =~ foo(y = 1)\n"), [(1, 1)]);
        assert_eq!(positions("a = b = c = 1\np b\n"), [(1, 1)]);
    }

    #[test]
    fn assignments_equal_to_one_in_a_later_loop_count_as_used() {
        // RuboCop compares loop assignments structurally, so the `if`
        // branch's `foo = 1` passes for the loop's.
        let source =
            "def m\n  if c\n    foo = 1\n  end\n  while d\n    foo = 1\n    p foo\n  end\nend\n";
        assert!(offenses(source).is_empty());
        let source =
            "def m\n  if c\n    foo = 2\n  end\n  while d\n    foo = 1\n    p foo\n  end\nend\n";
        assert_eq!(positions(source), [(3, 5)]);
    }

    #[test]
    fn block_bearing_and_reflective_binding_reads_are_live() {
        for acquisition in [
            "binding(&nil)",
            "binding(&-> {})",
            "Kernel.binding(&nil)",
            "K.binding(&nil)",
            "(self.binding ||= nil)",
            "(Kernel.binding ||= nil)",
            "send(:binding)",
        ] {
            let source =
                format!("def f\n  a = 2\n  {acquisition}.local_variable_get(:a)\nend\np f\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        // Unknown reflective mutation is not a liveness reference.
        let found = offenses("def f\n  a = 2\n  stash.local_variable_set(:a, 3)\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 3));
        assert_eq!(found[0].message, "Useless assignment to variable - `a`.");
    }

    #[test]
    fn implicit_reads_by_super_and_binding() {
        // Unlike upstream's childless-argument shortcut, nil is still a
        // positional argument: core binding raises before exposing locals.
        assert_eq!(
            positions("def m(a)\n  a = 1\n  binding(nil)\nend\n"),
            [(2, 3)]
        );
        assert!(offenses("def m(x)\n  x = 2\n  foo { super }\nend\n").is_empty());
        assert_eq!(
            positions("def m(a)\n  a = 1\n  binding(1)\nend\n"),
            [(2, 3)]
        );
        assert_eq!(positions("def m(x)\n  x = 2\n  super()\nend\n"), [(2, 3)]);
        assert_eq!(positions("def m(*)\n  x = 1\n  super\nend\n"), [(2, 3)]);
    }

    #[test]
    fn scopes_and_blocks() {
        assert_eq!(positions("l = ->(x) { y = x }\n"), [(1, 1), (1, 13)]);
        assert_eq!(positions("define_method(:m) { |a| a = 1 }\n"), [(1, 25)]);
        assert_eq!(positions("class << self\n  x = 1\nend\n"), [(2, 3)]);
        assert_eq!(positions("foo { x = _1 }\n"), [(1, 7)]);
        assert_eq!(positions("foo { x = it }\n"), [(1, 7)]);
        for source in [
            "x = 1\nfoo { x = 2 }\nputs x\n",
            "x = 1\ny = -> { x }\ny.call\n",
            "x = 1\ndef x.foo; end\n",
            "x = nil\nloop { x ||= 1; break }\np x\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn loops_and_rescue() {
        assert_eq!(positions("while (line = gets)\nend\n"), [(1, 8)]);
        assert_eq!(positions("x = 1 while false\n"), [(1, 1)]);
        for source in [
            "for x in y; end\nx\n",
            "begin\n  x = 1\nend until x\n",
            "x = 0\nuntil (x += 1) > 3\nend\n",
            "begin\n  x = 1\n  raise\nrescue\n  x += 1\n  retry if x < 3\nend\n",
            "x = 1\nbegin\n  x = 2\nensure\n  p x\nend\n",
            "x = 1\nbegin\n  x = 2\n  foo\nrescue\n  p x\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn pattern_variables_are_not_assignments() {
        assert!(offenses("case v\nin [a, *b] then 1\nin { c: } then 2\nend\n").is_empty());
        assert!(offenses("v => [a]\n").is_empty());
        assert_eq!(positions("case v\nin [a] if (b = a)\nend\n"), [(2, 12)]);
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "_ = 1\n",
            "_unused = 1\n",
            "def m(a, b = 1, *c, d:, **e, &f); end\n",
            "foo { |a, (b, c); d| }\n",
            "@a = 1\n",
            "$a = 1\n",
            "A = 1\n",
            "a.b = 1\n",
            "a[0] = 1\n",
            "x = 1\nputs \"#{x}\"\n",
            "/(?<x>\\d)/ =~ s\np x\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(offenses("foo = 1 # rubocop:disable Lint/UselessAssignment\n").is_empty());
    }

    const RUBOCOP_SPEC: &[Case] = &[
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition accepts with parentheses",
            source: r##"a = nil
puts a if (a = 123)
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition accepts without parentheses",
            source: r##"a = nil
puts a unless a = 123
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition accepts when the reassignment is nested in the condition expression",
            source: r##"a = nil
puts a if (a = 123) != 0
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition accepts when the reference is inside string interpolation",
            source: r##"nerrs = 0
raise "there were #{nerrs}" if (nerrs = get_nerrs) != 0
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition still registers a useless assignment that follows the modifier",
            source: r##"a = 0
puts "x #{a}" if (a = 1) != 0
a = 2
"##,
            offenses: &[(3, 1, "Useless assignment to variable - `a`.")],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition still registers an earlier assignment shadowed before the modifier",
            source: r##"a = nil
a = 5
puts "x #{a}" if (a = 123) != 0
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `a`.")],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition does not suppress a same-named variable in a different scope",
            source: r##"a = nil
def foo
  a = 10
  puts "x #{a}" if (a = 1) != 0
end
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `a`.")],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier condition still registers an earlier assignment when the reference follows the modifier",
            source: r##"x = 0
y = 1 if (x = 2) > 0
puts x, y
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `x`.")],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier loop condition accepts with parentheses",
            source: r##"a = nil
puts a while (a = false)
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and assigned again in a modifier loop condition accepts without parentheses",
            source: r##"a = nil
puts a until a = true
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in a method registers an offense",
            source: r##"class SomeClass
  foo = 1
  puts foo
  def some_method
    foo = 2
    bar = 3
    puts bar
  end
end
"##,
            offenses: &[(5, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in a singleton method defined with self keyword registers an offense",
            source: r##"class SomeClass
  foo = 1
  puts foo
  def self.some_method
    foo = 2
    bar = 3
    puts bar
  end
end
"##,
            offenses: &[(5, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in a singleton method defined with variable name registers an offense",
            source: r##"1.times do
  foo = 1
  puts foo
  instance = Object.new
  def instance.some_method
    foo = 2
    bar = 3
    puts bar
  end
end
"##,
            offenses: &[(6, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in a class registers an offense",
            source: r##"1.times do
  foo = 1
  puts foo
  class SomeClass
    foo = 2
    bar = 3
    puts bar
  end
end
"##,
            offenses: &[(5, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in a class subclassing another class stored in local variable registers an offense",
            source: r##"1.times do
  foo = 1
  puts foo
  array_class = Array
  class SomeClass < array_class
    foo = 2
    bar = 3
    puts bar
  end
end
"##,
            offenses: &[(6, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in a singleton class registers an offense",
            source: r##"1.times do
  foo = 1
  puts foo
  instance = Object.new
  class << instance
    foo = 2
    bar = 3
    puts bar
  end
end
"##,
            offenses: &[(6, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in a module registers an offense",
            source: r##"1.times do
  foo = 1
  puts foo
  module SomeModule
    foo = 2
    bar = 3
    puts bar
  end
end
"##,
            offenses: &[(5, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and referenced when defining a module does not register an offense",
            source: r##"x = Object.new
module x::Foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in `for` registers an offense",
            source: r##"for item in items
end
"##,
            offenses: &[(
                1,
                5,
                "Useless assignment to variable - `item`. Did you mean `items`?",
            )],
        },
        Case {
            desc: "when a variable is assigned before `for` registers an offense when it is not referenced",
            source: r##"node = foo
for node in bar
  return node if baz?
end
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `node`.")],
        },
        Case {
            desc: "when a variable is assigned before `for` registers no offense when the variable is referenced in the collection",
            source: r##"node = foo
for node in node.children
  return node if bar?
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in `for` with multiple variables registers an offense",
            source: r##"for i, j in items
  do_something(i)
end
"##,
            offenses: &[(1, 8, "Useless assignment to variable - `j`.")],
        },
        Case {
            desc: "when a variable is assigned and referenced in `for` does not register an offense",
            source: r##"for item in items
  do_something(item)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned and unreferenced in top level registers an offense",
            source: r##"foo = 1
bar = 2
puts bar
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned a list of values without brackets and unreferenced registers an offense and corrects to an array literal",
            source: r##"foo = 1,
      2
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned a list of values without brackets and unreferenced registers an offense and corrects a splat to an array literal",
            source: r##"foo = *bar
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned a list of values without brackets and unreferenced registers an offense and keeps an array literal as is",
            source: r##"foo = [1, 2]
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned with operator assignment in top level registers an offense",
            source: r##"foo ||= 1
"##,
            offenses: &[(
                1,
                1,
                "Useless assignment to variable - `foo`. Use `||` instead of `||=`.",
            )],
        },
        Case {
            desc: "when a variable is first assigned with `&&=` registers an offense but does not autocorrect (it would raise `NameError`)",
            source: r##"def foo
  bar &&= 1
end
"##,
            offenses: &[(
                2,
                3,
                "Useless assignment to variable - `bar`. Use `&&` instead of `&&=`.",
            )],
        },
        Case {
            desc: "when a variable is assigned multiple times but unreferenced registers offenses for each assignment",
            source: r##"def some_method
  foo = 1
  bar = 2
  foo = 3
  puts bar
end
"##,
            offenses: &[
                (2, 3, "Useless assignment to variable - `foo`."),
                (4, 3, "Useless assignment to variable - `foo`."),
            ],
        },
        Case {
            desc: "when a referenced variable is reassigned but not re-referenced registers an offense for the non-re-referenced assignment",
            source: r##"def some_method
  foo = 1
  puts foo
  foo = 3
end
"##,
            offenses: &[(4, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when an unreferenced variable is reassigned and re-referenced registers an offense for the unreferenced assignment",
            source: r##"def some_method
  foo = 1
  foo = 3
  puts foo
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when an unreferenced variable is reassigned in a block accepts",
            source: r##"def const_name(node)
  const_names = []
  const_node = node

  loop do
    namespace_node, name = *const_node
    const_names << name
    break unless namespace_node
    break if namespace_node.type == :cbase
    const_node = namespace_node
  end

  const_names.reverse.join('::')
end
"##,
            offenses: &[],
        },
        Case {
            // Intentional divergence: the only read precedes the
            // write-only block, so it cannot observe that block's assignment.
            desc: "when a referenced variable is reassigned in a block accepts",
            source: r##"def some_method
  foo = 1
  puts foo
  1.times do
    foo = 2
  end
end
"##,
            offenses: &[(5, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a block local variable is declared but not assigned accepts",
            source: r##"1.times do |i; foo|
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a block local variable is assigned and unreferenced registers offenses for the assignment",
            source: r##"1.times do |i; foo|
  foo = 2
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a block local variable is assigned and unreferenced registers offenses for self assignment in numblock",
            source: r##"do_something { foo += _1 }
"##,
            offenses: &[(
                1,
                16,
                "Useless assignment to variable - `foo`. Use `+` instead of `+=`.",
            )],
        },
        Case {
            desc: "when a block local variable is assigned and unreferenced registers offenses for self assignment in itblock",
            source: r##"do_something { foo += it }
"##,
            offenses: &[(
                1,
                16,
                "Useless assignment to variable - `foo`. Use `+` instead of `+=`.",
            )],
        },
        Case {
            desc: "when a variable is assigned in loop body and unreferenced registers an offense",
            source: r##"def some_method
  while true
    foo = 1
  end
end
"##,
            offenses: &[(3, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is reassigned at the end of loop body and would be referenced in next iteration accepts",
            source: r##"def some_method
  total = 0
  foo = 0

  while total < 100
    total += foo
    foo += 1
  end

  total
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned at the end of loop body and would be referenced in loop condition accepts",
            source: r##"def some_method
  total = 0
  foo = 0

  while foo < 100
    total += 1
    foo += 1
  end

  total
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a setter is invoked with operator assignment in loop body accepts",
            source: r##"def some_method
  obj = {}

  while obj[:count] < 100
    obj[:count] += 1
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned before a block registers an offense",
            source: r##"def some_method
  foo = 1
  foo = 2
  bar {
    foo = 3
    puts foo
  }
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is reassigned before a block and the variable is never referenced registers offenses for all the assignments",
            source: r##"def some_method
  foo = 1
  foo = 2
  bar {
    foo = 3
  }
end
"##,
            offenses: &[
                (2, 3, "Useless assignment to variable - `foo`."),
                (3, 3, "Useless assignment to variable - `foo`."),
                (5, 5, "Useless assignment to variable - `foo`."),
            ],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and never referenced registers offenses for all the assignments",
            source: r##"prev = nil
[1, 2, 3].each { |item| prev = item }
"##,
            offenses: &[
                (1, 1, "Useless assignment to variable - `prev`."),
                (2, 25, "Useless assignment to variable - `prev`."),
            ],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and never referenced registers offenses when the variable is reassigned in a lambda",
            source: r##"value = nil
callback = -> { value = 1 }
callback.call
"##,
            offenses: &[
                (1, 1, "Useless assignment to variable - `value`."),
                (2, 17, "Useless assignment to variable - `value`."),
            ],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and never referenced registers offenses in a numblock",
            source: r##"prev = nil
[1, 2, 3].each { prev = _1 }
"##,
            offenses: &[
                (1, 1, "Useless assignment to variable - `prev`."),
                (2, 18, "Useless assignment to variable - `prev`."),
            ],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and never referenced registers offenses in an itblock",
            source: r##"prev = nil
[1, 2, 3].each { prev = it }
"##,
            offenses: &[
                (1, 1, "Useless assignment to variable - `prev`."),
                (2, 18, "Useless assignment to variable - `prev`."),
            ],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and referenced after it accepts",
            source: r##"found = false
items.each { |item| found = true if item.ok? }
puts found
"##,
            offenses: &[],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and referenced in another block accepts",
            source: r##"value = nil
writer = -> { value = 1 }
reader = -> { value }
writer.call
reader.call
"##,
            offenses: &[],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and read through `binding` accepts",
            source: r##"value = nil
foo { value = 1 }
binding
"##,
            offenses: &[],
        },
        Case {
            desc: "when an outer variable is reassigned in a block and its name starts with an underscore accepts",
            source: r##"_prev = nil
[1, 2, 3].each { |item| _prev = item }
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in another branch before a block accepts",
            source: r##"def some_method
  if baz
    foo = 1
  else
    foo = 2
    bar {
      foo = 3
    }
  end

  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in another case branch before a block accepts",
            source: r##"def some_method
  case baz
  when 1
    foo = 1
  else
    foo = 2
    bar {
      foo = 3
    }
  end

  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when assigning in branch accepts",
            source: r##"def some_method
  changed = false

  if Random.rand > 1
    changed = true
  end

  [].each do
    changed = true
  end

  puts changed
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when assigning in case accepts",
            source: r##"def some_method
  changed = false

  case Random.rand
  when 0.5
    changed = true
  when 1..20
    changed = false
  when 21..70
    changed = true
  end

  [].each do
    changed = true
  end

  puts changed
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a referenced variable is reassigned but not re-referenced in a method defined in loop registers an offense",
            source: r##"while true
  def some_method
    foo = 1
    puts foo
    foo = 3
  end
end
"##,
            offenses: &[(5, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable that has same name as outer scope variable is not referenced in a method defined in loop registers an offense",
            source: r##"foo = 1

while foo < 100
  foo += 1
  def some_method
    foo = 1
  end
end
"##,
            offenses: &[(6, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned in single branch if and unreferenced registers an offense",
            source: r##"def some_method(flag)
  if flag
    foo = 1
  end
end
"##,
            offenses: &[(3, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when an unreferenced variable is reassigned in same branch and referenced after the branching registers an offense for the unreferenced assignment",
            source: r##"def some_method(flag)
  if flag
    foo = 1
    foo = 2
  end

  foo
end
"##,
            offenses: &[(3, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is reassigned in single branch if and referenced after the branching accepts",
            source: r##"def some_method(flag)
  foo = 1

  if flag
    foo = 2
  end

  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in a loop while loop accepts",
            source: r##"def while(param)
  ret = 1

  while param != 10
    param += 2
    ret = param + 1
  end

  ret
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in a loop post while loop accepts",
            source: r##"def post_while(param)
  ret = 1

  begin
    param += 2
    ret = param + 1
  end while param < 40

  ret
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in a loop until loop accepts",
            source: r##"def until(param)
  ret = 1

  until param == 10
    param += 2
    ret = param + 1
  end

  ret
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in a loop post until loop accepts",
            source: r##"def post_until(param)
  ret = 1

  begin
    param += 2
    ret = param + 1
  end until param == 10

  ret
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in a loop for loop accepts",
            source: r##"def for(param)
  ret = 1

  for x in param...10
    param += x
    ret = param + 1
  end

  ret
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in each branch of if and referenced after the branching accepts",
            source: r##"def some_method(flag)
  if flag
    foo = 2
  else
    foo = 3
  end

  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned in single branch if and referenced in the branch registers an offense for the unreferenced assignment",
            source: r##"def some_method(flag)
  foo = 1

  if flag
    foo = 2
    puts foo
  end
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned in each branch of if and referenced in the else branch registers an offense for the assignment in the if branch",
            source: r##"def some_method(flag)
  if flag
    foo = 2
  else
    foo = 3
    puts foo
  end
end
"##,
            offenses: &[(3, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is reassigned and unreferenced in an if branch while the variable is referenced in the paired else branch registers an offense for the reassignment in the if branch",
            source: r##"def some_method(flag)
  foo = 1

  if flag
    puts foo
    foo = 2
  else
    puts foo
  end
end
"##,
            offenses: &[(6, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when there's an unreferenced assignment in top level if branch while the variable is referenced in the paired else branch registers an offense for the assignment in the if branch",
            source: r##"if flag
  foo = 1
else
  puts foo
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when there's an unreferenced reassignment in an if branch while the variable is referenced in the paired elsif branch registers an offense for the reassignment in the if branch",
            source: r##"def some_method(flag_a, flag_b)
  foo = 1

  if flag_a
    puts foo
    foo = 2
  elsif flag_b
    puts foo
  end
end
"##,
            offenses: &[(6, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when there's an unreferenced reassignment in an if branch while the variable is referenced in a case branch in the paired else branch registers an offense for the reassignment in the if branch",
            source: r##"def some_method(flag_a, flag_b)
  foo = 1

  if flag_a
    puts foo
    foo = 2
  else
    case
    when flag_b
      puts foo
    end
  end
end
"##,
            offenses: &[(6, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when an assignment in an if branch is referenced in another if branch accepts",
            source: r##"def some_method(flag_a, flag_b)
  if flag_a
    foo = 1
  end

  if flag_b
    puts foo
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in branch of modifier if that references the variable in its conditional clauseand referenced after the branching accepts",
            source: r##"def some_method(flag)
  foo = 1 unless foo
  puts foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in branch of modifier if that references the variable in its conditional clauseand unreferenced registers an offense",
            source: r##"def some_method(flag)
  foo = 1 unless foo
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned on each side of && and referenced after the && accepts",
            source: r##"def some_method
  (foo = do_something_returns_object_or_nil) && (foo = 1)
  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when an unreferenced variable is reassigned on the left side of && and referenced after the && registers an offense for the unreferenced assignment",
            source: r##"def some_method
  foo = 1
  (foo = do_something_returns_object_or_nil) && do_something
  foo
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when an unreferenced variable is reassigned on the right side of && and referenced after the && accepts",
            source: r##"def some_method
  foo = 1
  do_something_returns_object_or_nil && foo = 2
  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when an unreferenced variable is reassigned on the right side of &&= and referenced after the &&= accepts",
            source: r##"def some_method(bar)
  foo = 1
  bar &&= (foo = 2)
  [foo, bar]
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when an unreferenced variable is reassigned on the right side of ||= and referenced after the ||= accepts",
            source: r##"def some_method(bar)
  foo = 1
  bar ||= (foo = 2)
  [foo, bar]
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when an unreferenced variable is reassigned on the right side of += and referenced after the += accepts",
            source: r##"def some_method(bar)
  foo = 1
  bar += (foo = 2)
  [foo, bar]
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned while referencing itself in rhs and referenced accepts",
            source: r##"def some_method
  foo = [1, 2]
  foo = foo.map { |i| i + 1 }
  puts foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned while referencing itself in rhs and referenced registers an offense when the reassignment is the last statement",
            source: r##"foo = [1, 2]
foo = foo.map { |i| i + 1 }
"##,
            offenses: &[(2, 1, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is reassigned with binary operator assignment and referenced accepts",
            source: r##"def some_method
  foo = 1
  foo += 1
  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned with logical operator assignment and referenced accepts",
            source: r##"def some_method
  foo = do_something_returns_object_or_nil
  foo ||= 1
  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned with binary operator assignment while assigning to itself in rhs then referenced registers an offense for the assignment in rhs",
            source: r##"def some_method
  foo = 1
  foo += foo = 2
  foo
end
"##,
            offenses: &[(3, 10, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned first with ||= and referenced accepts",
            source: r##"def some_method
  foo ||= 1
  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned with ||= at the last expression of the scope registers an offense",
            source: r##"def some_method
  foo = do_something_returns_object_or_nil
  foo ||= 1
end
"##,
            offenses: &[(
                3,
                3,
                "Useless assignment to variable - `foo`. Use `||` instead of `||=`.",
            )],
        },
        Case {
            desc: "when a variable is assigned with ||= before the last expression of the scope registers an offense",
            source: r##"def some_method
  foo = do_something_returns_object_or_nil
  foo ||= 1
  some_return_value
end
"##,
            offenses: &[(3, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned with multiple assignment and unreferenced registers an offense",
            source: r##"def some_method
  foo, bar = do_something
  puts foo
end
"##,
            offenses: &[(
                2,
                8,
                "Useless assignment to variable - `bar`. Use `_` or `_bar` as a variable name to indicate that it won't be used.",
            )],
        },
        Case {
            desc: "when a variable is assigned as an argument to a method given to multiple assignment registers an offense",
            source: r##"def some_method
  a, b = func(c = 3)
  [a, b]
end
"##,
            offenses: &[(2, 15, "Useless assignment to variable - `c`.")],
        },
        Case {
            desc: "when a variable is assigned as an argument to a method given to multiple assignment and later used does not register an offense",
            source: r##"def some_method
  a, b = func(c = 3)
  [a, b, c]
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when variables are assigned using chained assignment and remain unreferenced registers an offense",
            source: r##"def some_method
  foo = bar = do_something
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when same name variables are assigned using chained assignment registers an offense",
            source: r##"def some_method
  foo = foo = do_something
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when variables are assigned using unary operator in chained assignment and remain unreferenced registers an offense",
            source: r##"def some_method
  foo = -bar = do_something
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when variables are assigned with sequential assignment using the comma operator and unreferenced registers an offense",
            source: r##"def some_method
  foo = 1, bar = 2
end
"##,
            offenses: &[
                (2, 3, "Useless assignment to variable - `foo`."),
                (2, 12, "Useless assignment to variable - `bar`."),
            ],
        },
        Case {
            desc: "when a variable is reassigned with multiple assignment while referencing itself in rhs and referenced accepts",
            source: r##"def some_method
  foo = 1
  foo, bar = do_something(foo)
  puts foo, bar
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when part of a multiple assignment is enclosed in parentheses registers an offense when the variable in parentheses is not used",
            source: r##"def some_method
  foo, (bar, baz) = do_something
  puts foo, bar
end
"##,
            offenses: &[(
                2,
                14,
                "Useless assignment to variable - `baz`. Use `_` or `_baz` as a variable name to indicate that it won't be used.",
            )],
        },
        Case {
            desc: "when part of a multiple assignment is enclosed in parentheses registers an offense when the variable in nested parentheses is not used",
            source: r##"def some_method
  foo, (bar, (baz, qux)) = do_something
  puts foo, bar, baz
end
"##,
            offenses: &[(
                2,
                20,
                "Useless assignment to variable - `qux`. Use `_` or `_qux` as a variable name to indicate that it won't be used.",
            )],
        },
        Case {
            desc: "when part of a multiple assignment is enclosed in parentheses with splat registers an offense when the variable is not used",
            source: r##"def some_method
  (foo, bar), *baz = do_something
  puts foo, bar
end
"##,
            offenses: &[(
                2,
                16,
                "Useless assignment to variable - `baz`. Use `_` or `_baz` as a variable name to indicate that it won't be used.",
            )],
        },
        Case {
            desc: "when part of a multiple assignment is enclosed in parentheses with splat registers an offense when the variable is not used in nested assignment",
            source: r##"def some_method
  foo, (*bar, baz) = do_something
  puts foo, baz
end
"##,
            offenses: &[(
                2,
                10,
                "Useless assignment to variable - `bar`. Use `_` or `_bar` as a variable name to indicate that it won't be used.",
            )],
        },
        Case {
            desc: "when a variable is assigned with rest assignment and unreferenced registers an offense",
            source: r##"def some_method
  foo, *bar = do_something
  puts foo
end
"##,
            offenses: &[(
                2,
                9,
                "Useless assignment to variable - `bar`. Use `_` or `_bar` as a variable name to indicate that it won't be used.",
            )],
        },
        Case {
            desc: "when a variable is assigned in loop body and referenced in post while condition accepts",
            source: r##"begin
  a = (a || 0) + 1
  puts a
end while a <= 2
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in loop body and referenced in post until condition accepts",
            source: r##"begin
  a = (a || 0) + 1
  puts a
end until a > 2
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in main body of begin with rescue but unreferenced registers an offense",
            source: r##"begin
  do_something
  foo = true
rescue
  do_anything
end
"##,
            offenses: &[(3, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned in main body of begin, rescue and else then referenced after the begin accepts",
            source: r##"begin
  do_something
  foo = :in_begin
rescue FirstError
  foo = :in_first_rescue
rescue SecondError
  foo = :in_second_rescue
else
  foo = :in_else
end

puts foo
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in main body of begin then referenced after the begin accepts",
            source: r##"begin
  status = :initial
  connect_sometimes_fails!
  status = :connected
  fetch_sometimes_fails!
  status = :fetched
rescue
  do_something
end

puts status
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in main body of begin then referenced in rescue accepts",
            source: r##"begin
  status = :initial
  connect_sometimes_fails!
  status = :connected
  fetch_sometimes_fails!
  status = :fetched
rescue
  puts status
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in main body of begin then referenced in ensure accepts",
            source: r##"begin
  status = :initial
  connect_sometimes_fails!
  status = :connected
  fetch_sometimes_fails!
  status = :fetched
ensure
  puts status
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in a conditional in main body of begin then referenced in rescue accepts",
            source: r##"def some_method(items)
  step = :load
  begin
    unless items.empty?
      step = :prepare
      prepare(items)
      step = :process
      process(items)
    end
  rescue StandardError => e
    puts "failed during #{step}: #{e.message}"
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in a nested conditional in main body of begin then referenced in rescue accepts",
            source: r##"def some_method(items, flag)
  step = :load
  begin
    unless items.empty?
      if flag
        step = :prepare
        prepare(items)
        step = :process
        process(items)
      end
    end
  rescue StandardError
    puts step
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in a conditional in main body of begin with ensure then referenced in an outer rescue accepts",
            source: r##"def some_method(flag)
  status = :initial
  begin
    begin
      if flag
        status = :connected
        fetch_sometimes_fails!
        status = :fetched
      end
    ensure
      cleanup
    end
  rescue StandardError
    puts status
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in a conditional in main body of begin then referenced after the begin accepts",
            source: r##"def some_method(flag)
  status = :initial
  begin
    if flag
      status = :connected
      fetch_sometimes_fails!
      status = :fetched
    end
  rescue StandardError
    do_something
  end

  puts status
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in a conditional in main body of begin then referenced in ensure accepts",
            source: r##"def some_method(flag)
  status = :initial
  begin
    if flag
      status = :connected
      fetch_sometimes_fails!
      status = :fetched
    end
  ensure
    puts status
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is reassigned multiple times in a conditional in main body of begin then referenced later in the main body registers an offense",
            source: r##"def some_method(flag)
  begin
    if flag
      status = :connected
      fetch_sometimes_fails!
      status = :fetched
    end
    puts status
  rescue StandardError
    do_something
  end
end
"##,
            offenses: &[(4, 7, "Useless assignment to variable - `status`.")],
        },
        Case {
            desc: "when a variable is reassigned multiple times in a conditional in a block and the block is in main body of begin in the outer scope registers an offense",
            source: r##"def some_method(items, flag)
  begin
    items.each do
      status = :initial
      if flag
        status = :connected
        fetch_sometimes_fails!
        status = :fetched
      end
      puts status
    end
  rescue StandardError
    do_something
  end
end
"##,
            offenses: &[(6, 9, "Useless assignment to variable - `status`.")],
        },
        Case {
            desc: "when a variable is reassigned multiple times in rescue and referenced after the begin registers an offense",
            source: r##"foo = false

begin
  do_something
rescue
  foo = true
  foo = true
end

puts foo
"##,
            offenses: &[(6, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is reassigned multiple times in rescue with ensure then referenced after the begin registers an offense",
            source: r##"foo = false

begin
  do_something
rescue
  foo = true
  foo = true
ensure
  do_anything
end

puts foo
"##,
            offenses: &[(6, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is reassigned multiple times in ensure with rescue then referenced after the begin registers an offense",
            source: r##"begin
  do_something
rescue
  do_anything
ensure
  foo = true
  foo = true
end

puts foo
"##,
            offenses: &[(6, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned at the end of rescue and would be referenced with retry accepts",
            source: r##"retried = false

begin
  do_something
rescue
  fail if retried
  retried = true
  retry
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned with operator assignment in rescue and would be referenced with retry accepts",
            source: r##"retry_count = 0

begin
  do_something
rescue
  fail if (retry_count += 1) > 3
  retry
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in main body of begin, rescue and else and reassigned in ensure then referenced after the begin registers offenses for each assignment before ensure",
            source: r##"begin
  do_something
  foo = :in_begin
rescue FirstError
  foo = :in_first_rescue
rescue SecondError
  foo = :in_second_rescue
else
  foo = :in_else
ensure
  foo = :in_ensure
end

puts foo
"##,
            offenses: &[
                (3, 3, "Useless assignment to variable - `foo`."),
                (5, 3, "Useless assignment to variable - `foo`."),
                (7, 3, "Useless assignment to variable - `foo`."),
                (9, 3, "Useless assignment to variable - `foo`."),
            ],
        },
        Case {
            desc: "when a rescued error variable is wrongly tried to be referenced in another rescue body registers an offense",
            source: r##"begin
  do_something
rescue FirstError => error
rescue SecondError
  p error # => nil
end
"##,
            offenses: &[(3, 22, "Useless assignment to variable - `error`.")],
        },
        Case {
            desc: "when a rescued error variable is not used and no exception type is specified registers an offense",
            source: r##"begin
  do_something
rescue => error
end
"##,
            offenses: &[(3, 11, "Useless assignment to variable - `error`.")],
        },
        Case {
            desc: "when a method argument is reassigned and zero arity super is called accepts",
            source: r##"def some_method(foo)
  foo = 1
  super
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a local variable is unreferenced and zero arity super is called registers an offense",
            source: r##"def some_method(bar)
  foo = 1
  super
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a method argument is reassigned but not passed to super registers an offense",
            source: r##"def some_method(foo, bar)
  foo = 1
  super(bar)
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a named capture is unreferenced in top level registers an offense",
            source: r##"/(?<foo>w+)/ =~ 'FOO'
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a named capture is unreferenced in other than top level registers an offense",
            source: r##"def some_method
  /(?<foo>\w+)/ =~ 'FOO'
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a named capture is referenced accepts",
            source: r##"def some_method
  /(?<foo>w+)(?<bar> +)/ =~ 'FOO'
  puts foo
  puts bar
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is referenced in rhs of named capture expression accepts",
            source: r##"def some_method
  foo = 'some string'
  /(?<foo>w+)/ =~ foo
  puts foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a pattern match variable is assigned with `in` and referenced in a block does not register an offense",
            source: r##"def some_method
  foo in { bar: bar }
  baz { bar -= 1 }
  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a pattern match variable is assigned with `in` and unreferenced in a block registers an offense",
            source: r##"def some_method
  foo in { bar: bar }
  baz { qux -= 1 }
  foo
end
"##,
            offenses: &[(
                3,
                9,
                "Useless assignment to variable - `qux`. Use `-` instead of `-=`.",
            )],
        },
        Case {
            desc: "when a pattern match variable is assigned with `=>` and referenced in a block does not register an offense",
            source: r##"def some_method
  foo => { bar: bar }
  baz { bar -= 1 }
  foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a pattern match variable is assigned with `=>` and unreferenced in a block registers an offense",
            source: r##"def some_method
  foo => { bar: bar }
  baz { qux -= 1 }
  foo
end
"##,
            offenses: &[(
                3,
                9,
                "Useless assignment to variable - `qux`. Use `-` instead of `-=`.",
            )],
        },
        Case {
            desc: "when a variable is assigned in begin and referenced outside accepts",
            source: r##"def some_method
  begin
    foo = 1
  end
  puts foo
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is shadowed by a block argument and unreferenced registers an offense",
            source: r##"def some_method
  foo = 1
  1.times do |foo|
    puts foo
  end
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is not used and the name starts with _ accepts",
            source: r##"def some_method
  _foo = 1
  bar = 2
  puts bar
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a method argument is not used accepts",
            source: r##"def some_method(arg)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when an optional method argument is not used accepts",
            source: r##"def some_method(arg = nil)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a block method argument is not used accepts",
            source: r##"def some_method(&block)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a splat method argument is not used accepts",
            source: r##"def some_method(*args)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when an optional keyword method argument is not used accepts",
            source: r##"def some_method(name: value)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a keyword splat method argument is used accepts",
            source: r##"def some_method(name: value, **rest_keywords)
  p rest_keywords
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a keyword splat method argument is not used accepts",
            source: r##"def some_method(name: value, **rest_keywords)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when an anonymous keyword splat method argument is defined accepts",
            source: r##"def some_method(name: value, **)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a block argument is not used accepts",
            source: r##"1.times do |i|
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when there is only one AST node and it is unused variable registers an offense",
            source: r##"foo = 1
"##,
            offenses: &[(1, 1, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned while being passed to a method taking block and the variable is used accepts",
            source: r##"some_method(foo = 1) do
end
puts foo
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned while being passed to a method taking block and the variable is not used registers an offense",
            source: r##"some_method(foo = 1) do
end
"##,
            offenses: &[(1, 13, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when a variable is assigned and passed to a method followed by method taking block accepts",
            source: r##"pattern = '*.rb'
Dir.glob(pattern).map do |path|
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a useless assignment wraps a block containing another useless assignment registers offenses and corrects",
            source: r##"foo = do_something {
  bar = do_something_else
}
"##,
            offenses: &[
                (1, 1, "Useless assignment to variable - `foo`."),
                (2, 3, "Useless assignment to variable - `bar`."),
            ],
        },
        Case {
            desc: "using numbered block parameter does not register an offense when the variable is used",
            source: r##"var = 42

do_something { _1 == var }
"##,
            offenses: &[],
        },
        Case {
            desc: "using numbered block parameter does not register an offense when the variable is assigned and later used",
            source: r##"var = nil

do_something { var = _1 }

something_else(var)
"##,
            offenses: &[],
        },
        Case {
            desc: "using `it` block parameter does not register an offense when the variable is used",
            source: r##"var = 42

do_something { it == var }
"##,
            offenses: &[],
        },
        Case {
            desc: "using `it` block parameter does not register an offense when the variable is assigned and later used",
            source: r##"var = nil

do_something { var = it }

something_else(var)
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in 2 identical if branches doesn't think 1 of the 2 assignments is useless",
            source: r##"def foo
  if bar
    foo = 1
  else
    foo = 1
  end
  foo.bar.baz
end
"##,
            offenses: &[],
        },
        Case {
            desc: "similar name suggestion when there's a similar variable-like method invocation suggests the method name",
            source: r##"def some_method
  enviromnent = {}
  another_symbol
  puts environment
end
"##,
            offenses: &[(
                2,
                3,
                "Useless assignment to variable - `enviromnent`. Did you mean `environment`?",
            )],
        },
        Case {
            desc: "similar name suggestion when there's a similar variable suggests the variable name",
            source: r##"def some_method
  environment = nil
  another_symbol
  enviromnent = {}
  puts environment
end
"##,
            offenses: &[(
                4,
                3,
                "Useless assignment to variable - `enviromnent`. Did you mean `environment`?",
            )],
        },
        Case {
            desc: "similar name suggestion when there are only less similar names does not suggest any name",
            source: r##"def some_method
  enviromnent = {}
  another_symbol
  puts envelope
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `enviromnent`.")],
        },
        Case {
            desc: "similar name suggestion when there's a similar method invocation with explicit receiver does not suggest any name",
            source: r##"def some_method
  enviromnent = {}
  another_symbol
  puts self.environment
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `enviromnent`.")],
        },
        Case {
            desc: "similar name suggestion when there's a similar method invocation with arguments does not suggest any name",
            source: r##"def some_method
  enviromnent = {}
  another_symbol
  puts environment(1)
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `enviromnent`.")],
        },
        Case {
            desc: "similar name suggestion when there's a similar name but it's in inner scope does not suggest any name",
            source: r##"class SomeClass
  enviromnent = {}

  def some_method(environment)
    puts environment
  end
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `enviromnent`.")],
        },
        Case {
            desc: "inside a `case-match` node does not register an offense when the variable is used",
            source: r##"case '0'
in String
  res = 1
else
  res = 2
end

do_something(res)
"##,
            offenses: &[],
        },
        Case {
            desc: "when duplicate assignments in the loop body while loop registers an offense",
            source: r##"while
  foo = 1
  foo = 1
  p foo
end
"##,
            offenses: &[(2, 3, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when duplicate assignments in the loop body while loop with parenthesized body registers an offense",
            source: r##"while
  (
    foo = 1
    foo = 1
  )
  p foo
end
"##,
            offenses: &[(3, 5, "Useless assignment to variable - `foo`.")],
        },
        Case {
            desc: "when duplicate assignments in `if` branch inside a loop while loop does not register an offense",
            source: r##"while
  if cond
    var += 1
  else
    var -= 1
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when duplicate assignments appear in `if` branch inside a loop and the variable is used outside `while` loop while loop does not register an offense",
            source: r##"var = false
while loop_cond
  if var
    var = false
    foo
  else
    var = true
    bar
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when duplicate assignments appear in nested `if` branches inside a loop and the variable is used outside `while` loop while loop does not register an offense",
            source: r##"def parse_options
  index = -1
  while loop_cond
    index += 1

    if first_cond
      index += 1
    else
      if second_cond
        index += 1
      else
        if third_cond
          index += 1
        end
      end
    end
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when duplicate assignments in `rescue` branch with `retry` does not register an offense",
            source: r##"def testing
rescue Foo
  attempts += 1
  retry
rescue Bar
  attempts += 1
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when duplicate assignments in a case branch inside a loop while loop does not register an offense",
            source: r##"while
  case
  when fizz then foo += 1
  when buzz then foo -= 1
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when duplicate assignments in a case-match branch inside a loop while loop does not register an offense",
            source: r##"while
  case expr
  in fizz then foo += 1
  in buzz then foo -= 1
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in loop body and used in loop condition does not register an offense when variable is used directly in condition",
            source: r##"keep_going = true
while keep_going
  keep_going = false
  if rand < 0.5
    keep_going = true
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in loop body and used in loop condition does not register an offense when variable is used in condition expression",
            source: r##"try = 0
while try < max_tries
  try += 1
  next if weak?
  try = 0
end

raise(CombinationPoolExhaustedError)
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in loop body and used in loop condition does not register an offense when variable is used directly in condition inside a method",
            source: r##"def foo
  keep_going = true
  while keep_going
    keep_going = false
    if rand < 0.5
      keep_going = true
    end
  end
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in loop body and used in loop condition does not register an offense when variable is used in condition expression inside a method",
            source: r##"def generate_pin
  try = 0
  while try < max_tries
    try += 1
    next if weak?
    try = 0
  end

  raise(CombinationPoolExhaustedError)
end
"##,
            offenses: &[],
        },
        Case {
            desc: "when a variable is assigned in loop body and used in loop condition registers an offense when the variable is assigned in a method defined in the loop",
            source: r##"x = 0
while x < 10
  def helper
    x = 1
  end
  x += 1
end
"##,
            offenses: &[(4, 5, "Useless assignment to variable - `x`.")],
        },
    ];
}
