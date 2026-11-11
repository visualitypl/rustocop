//! `Lint/AssignmentInCondition` — flags `=` in the condition of an `if`,
//! `unless`, `while` or `until` (modifiers, `elsif` and ternaries included),
//! where `==` was likely meant.
//!
//! Matches RuboCop's semantics with its default `AllowSafeAssignment: true`:
//! an assignment wrapped in its own parentheses (`if (line = gets)`) is a
//! deliberate "safe assignment" and is accepted. Also accepted, as in
//! RuboCop: assignments inside blocks, lambdas, `defined?` or the arguments
//! of a non-setter call (`include?(x = y)`); shorthand assignments (`||=`,
//! `+=`); statements of a multi-statement body (`(a = b; a.c?)`, a branch,
//! `begin … rescue`), whose value is discarded; and the conditions of
//! `begin … end while` loops and pattern guards, which RuboCop does not
//! check. Variable, constant, multiple and setter
//! assignments (`a.b = c`, `a[i] = c`) are reported, at the `=`.
//!
//! NodeContext distinguishes pattern guards (`in x if y`) from modifier
//! conditionals; only each real condition's intentionally pruned logic is walked.

use ruby_prism::{CallNode, InNode, Location, Node, ParenthesesNode, StatementsNode, Visit};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/AssignmentInCondition` cop.
pub struct AssignmentInCondition;

const MESSAGE: &str = "Use `==` if you meant to do a comparison or wrap the expression in \
                       parentheses to indicate you meant to assign in a condition.";

impl Cop for AssignmentInCondition {
    fn name(&self) -> &'static str {
        "Lint/AssignmentInCondition"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if cx
            .parent()
            .is_some_and(|parent| parent.as_in_node().is_some())
        {
            return;
        }
        let condition = match node {
            Node::IfNode { .. } => Some(node.as_if_node().unwrap().predicate()),
            Node::UnlessNode { .. } => Some(node.as_unless_node().unwrap().predicate()),
            Node::WhileNode { .. } => node
                .as_while_node()
                .filter(|n| !n.is_begin_modifier())
                .map(|n| n.predicate()),
            Node::UntilNode { .. } => node
                .as_until_node()
                .filter(|n| !n.is_begin_modifier())
                .map(|n| n.predicate()),
            _ => None,
        };
        if let Some(condition) = condition {
            Reporter {
                cop: self,
                file,
                out,
            }
            .check_condition(&condition);
        }
    }
}

/// Reports offenses at an assignment's `=`.
struct Reporter<'a> {
    cop: &'a AssignmentInCondition,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
}

impl Reporter<'_> {
    fn report(&mut self, operator: &Location<'_>) {
        let (line, col) = self.file.line_col(operator.start_offset());
        self.out.push(Offense {
            correction: None,
            cop: self.cop.name(),
            severity: self.cop.severity(),
            message: MESSAGE.to_string(),
            line,
            end_line: line,
            col,
        });
    }

    /// Check one conditional's condition.
    fn check_condition(&mut self, condition: &Node<'_>) {
        ConditionWalker { reporter: self }.visit(condition);
    }
}

/// Visit an `in` clause, treating a guard (`in x if y`, an `IfNode` or
/// `UnlessNode` wrapping the pattern in Prism) as a plain node rather than a
/// conditional.
fn visit_in_node_with_plain_guard<'pr, V: Visit<'pr>>(visitor: &mut V, node: &InNode<'pr>) {
    let pattern = node.pattern();
    if let Some(guard) = pattern.as_if_node() {
        ruby_prism::visit_if_node(visitor, &guard);
    } else if let Some(guard) = pattern.as_unless_node() {
        ruby_prism::visit_unless_node(visitor, &guard);
    } else {
        visitor.visit(&pattern);
    }
    if let Some(statements) = node.statements() {
        visitor.visit_statements_node(&statements);
    }
}

/// Walks one condition, reporting the assignments RuboCop's
/// `traverse_node` reaches.
struct ConditionWalker<'r, 'a> {
    reporter: &'r mut Reporter<'a>,
}

impl ConditionWalker<'_, '_> {
    /// A parenthesized group's statements (`begin` in RuboCop's AST): empty
    /// or a lone assignment is skipped (a safe assignment).
    fn visit_group<'pr>(&mut self, statements: Option<StatementsNode<'pr>>) {
        let Some(statements) = statements else {
            return;
        };
        let body = statements.body();
        let mut iter = body.iter();
        match (iter.next(), iter.next()) {
            (None, _) => {}
            (Some(only), None) if is_safe_assignment(&only) => {}
            _ => self.visit_statements_node(&statements),
        }
    }

    /// Walk a statement whose value is discarded: an assignment is not
    /// reported, but its children are walked.
    fn visit_discarded<'pr>(&mut self, node: &Node<'pr>) {
        match node {
            Node::LocalVariableWriteNode { .. } => ruby_prism::visit_local_variable_write_node(
                self,
                &node.as_local_variable_write_node().unwrap(),
            ),
            Node::InstanceVariableWriteNode { .. } => {
                ruby_prism::visit_instance_variable_write_node(
                    self,
                    &node.as_instance_variable_write_node().unwrap(),
                )
            }
            Node::ClassVariableWriteNode { .. } => ruby_prism::visit_class_variable_write_node(
                self,
                &node.as_class_variable_write_node().unwrap(),
            ),
            Node::GlobalVariableWriteNode { .. } => ruby_prism::visit_global_variable_write_node(
                self,
                &node.as_global_variable_write_node().unwrap(),
            ),
            Node::ConstantWriteNode { .. } => {
                ruby_prism::visit_constant_write_node(self, &node.as_constant_write_node().unwrap())
            }
            Node::ConstantPathWriteNode { .. } => ruby_prism::visit_constant_path_write_node(
                self,
                &node.as_constant_path_write_node().unwrap(),
            ),
            Node::MultiWriteNode { .. } => {
                ruby_prism::visit_multi_write_node(self, &node.as_multi_write_node().unwrap())
            }
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if is_assignment_method(&call) {
                    ruby_prism::visit_call_node(self, &call);
                }
            }
            _ => self.visit(node),
        }
    }
}

impl<'pr> Visit<'pr> for ConditionWalker<'_, '_> {
    // Several statements form a `begin` in RuboCop's AST, and only the last
    // one's value is used: an assignment among them is not the condition.
    fn visit_statements_node(&mut self, node: &StatementsNode<'pr>) {
        let body = node.body();
        if body.iter().nth(1).is_none() {
            ruby_prism::visit_statements_node(self, node);
            return;
        }
        for statement in body.iter() {
            self.visit_discarded(&statement);
        }
    }

    // A plain `begin … end` holds its statements directly in RuboCop's AST
    // (no inner `begin`), so they count as part of the condition; with
    // `rescue` or `ensure` they are grouped again.
    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        if node.rescue_clause().is_some() || node.ensure_clause().is_some() {
            ruby_prism::visit_begin_node(self, node);
            return;
        }
        if let Some(statements) = node.statements() {
            for statement in statements.body().iter() {
                self.visit(&statement);
            }
        }
    }

    // Assignments inside blocks and lambdas are irrelevant; the call
    // carrying a block goes with it.
    fn visit_block_node(&mut self, _node: &ruby_prism::BlockNode<'pr>) {}

    fn visit_lambda_node(&mut self, _node: &ruby_prism::LambdaNode<'pr>) {}

    // An assignment inside `defined?` is never executed.
    fn visit_defined_node(&mut self, _node: &ruby_prism::DefinedNode<'pr>) {}

    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        // A non-setter call's receiver and arguments are not part of the
        // condition's own logic.
        if !is_assignment_method(node) {
            return;
        }
        if let Some(operator) = node.equal_loc() {
            self.reporter.report(&operator);
        }
        ruby_prism::visit_call_node(self, node);
    }

    // `/(?<name>…)/ =~ value` is its own node in RuboCop's AST, whose
    // children are walked, not a skipped `=~` call.
    fn visit_match_write_node(&mut self, node: &ruby_prism::MatchWriteNode<'pr>) {
        let call = node.call();
        if let Some(receiver) = call.receiver() {
            self.visit(&receiver);
        }
        if let Some(arguments) = call.arguments() {
            self.visit_arguments_node(&arguments);
        }
    }

    fn visit_super_node(&mut self, node: &ruby_prism::SuperNode<'pr>) {
        if node
            .block()
            .is_some_and(|block| block.as_block_node().is_some())
        {
            return;
        }
        ruby_prism::visit_super_node(self, node);
    }

    fn visit_parentheses_node(&mut self, node: &ParenthesesNode<'pr>) {
        match node.body() {
            None => {}
            Some(body) => match body.as_statements_node() {
                Some(statements) => self.visit_group(Some(statements)),
                None if is_safe_assignment(&body) => {}
                None => self.visit(&body),
            },
        }
    }

    // Interpolation is a `begin` in RuboCop's AST, like parentheses.
    fn visit_embedded_statements_node(&mut self, node: &ruby_prism::EmbeddedStatementsNode<'pr>) {
        self.visit_group(node.statements());
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.reporter.report(&node.operator_loc());
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        self.reporter.report(&node.operator_loc());
        ruby_prism::visit_instance_variable_write_node(self, node);
    }

    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        self.reporter.report(&node.operator_loc());
        ruby_prism::visit_class_variable_write_node(self, node);
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        self.reporter.report(&node.operator_loc());
        ruby_prism::visit_global_variable_write_node(self, node);
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.reporter.report(&node.operator_loc());
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        self.reporter.report(&node.operator_loc());
        ruby_prism::visit_constant_path_write_node(self, node);
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        self.reporter.report(&node.operator_loc());
        ruby_prism::visit_multi_write_node(self, node);
    }

    // Shorthand assignments to a call (`a.b ||= c`, `a[i] += c`): the
    // target is a non-setter call in RuboCop's AST, so only the value is
    // walked.
    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        self.visit(&node.value());
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        self.visit(&node.value());
    }

    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        self.visit(&node.value());
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.visit(&node.value());
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.visit(&node.value());
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        self.visit(&node.value());
    }

    // A nested conditional reports its own condition; walking it here too
    // would report the same `=` twice.
    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
        if let Some(subsequent) = node.subsequent() {
            self.visit(&subsequent);
        }
    }

    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
        if let Some(else_clause) = node.else_clause() {
            self.visit_else_node(&else_clause);
        }
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        if node.is_begin_modifier() {
            ruby_prism::visit_while_node(self, node);
        } else if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        if node.is_begin_modifier() {
            ruby_prism::visit_until_node(self, node);
        } else if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
    }

    fn visit_in_node(&mut self, node: &InNode<'pr>) {
        visit_in_node_with_plain_guard(self, node);
    }
}

/// RuboCop's `assignment_method?`: a method name ending in `=` that is not a
/// comparison (`a.b=`, `[]=`).
fn is_assignment_method(call: &CallNode<'_>) -> bool {
    let name = call.name().as_slice();
    name.ends_with(b"=") && !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=")
}

/// RuboCop's safe assignment body: a single `=` assignment to a variable,
/// constant or multiple targets, or a setter call written with `=`.
fn is_safe_assignment(node: &Node<'_>) -> bool {
    match node {
        Node::LocalVariableWriteNode { .. }
        | Node::InstanceVariableWriteNode { .. }
        | Node::ClassVariableWriteNode { .. }
        | Node::GlobalVariableWriteNode { .. }
        | Node::ConstantWriteNode { .. }
        | Node::ConstantPathWriteNode { .. }
        | Node::MultiWriteNode { .. } => true,
        Node::CallNode { .. } => node.as_call_node().unwrap().equal_loc().is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/assignment_in_condition_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov), for the default
    //! `AllowSafeAssignment: true`.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/AssignmentInCondition")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    fn assert_no_offenses(sources: &[&str]) {
        for source in sources {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_local_variable_assignment_with_message() {
        let found = offenses("if test = 10\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 9));
        assert_eq!(found[0].message, MESSAGE);
    }

    #[test]
    fn flags_assignment_in_loop_conditions() {
        assert_eq!(positions("while test = 10\nend\n"), vec![(1, 12)]);
        assert_eq!(positions("until test = 10\nend\n"), vec![(1, 12)]);
    }

    #[test]
    fn flags_every_kind_of_equals_assignment() {
        assert_eq!(positions("if @test = 10\nend\n"), vec![(1, 10)]);
        assert_eq!(positions("if @@test = 10\nend\n"), vec![(1, 11)]);
        assert_eq!(positions("if $test = 10\nend\n"), vec![(1, 10)]);
        assert_eq!(positions("if TEST = 10\nend\n"), vec![(1, 9)]);
        assert_eq!(positions("if a[3] = 10\nend\n"), vec![(1, 9)]);
        assert_eq!(positions("if test.method = 10\nend\n"), vec![(1, 16)]);
        assert_eq!(positions("if test&.method = 10\nend\n"), vec![(1, 17)]);
    }

    #[test]
    fn flags_assignment_after_other_operands() {
        assert_eq!(
            positions("if test == 10 || foobar = 1\nend\n"),
            vec![(1, 25)]
        );
        assert_eq!(
            positions("raise StandardError unless (foo ||= bar) || a = b\n"),
            vec![(1, 47)]
        );
    }

    #[test]
    fn flags_condition_inside_a_block() {
        assert_eq!(positions("foo { x if y = z }\n"), vec![(1, 14)]);
    }

    #[test]
    fn flags_unparenthesized_assignment_in_parenthesized_logic() {
        for keyword in ["if", "while", "until"] {
            for operator in ["&&", "||"] {
                let col = keyword.len() as u32 + 22;
                let source = format!("{keyword} (foo == bar {operator} test = 10)\nend\n");
                assert_eq!(positions(&source), vec![(1, col)], "{source:?}");
                let source = format!("{keyword} ((foo == bar {operator} test = 10))\nend\n");
                assert_eq!(positions(&source), vec![(1, col + 1)], "{source:?}");
                let source = format!("do_something {keyword} (foo == bar {operator} test = 10)\n");
                assert_eq!(positions(&source), vec![(1, col + 13)], "{source:?}");
            }
        }
    }

    #[test]
    fn accepts_comparisons_blocks_and_shorthand_assignments() {
        assert_no_offenses(&[
            "if test == 10\nend\n",
            "return 1 if any_errors? { o = inspect(file) }\n",
            "return 1 if any_errors? { o = file }.present?\n",
            "return 1 if any_errors? { o = inspect(_1) }\n",
            "return 1 if any_errors? { o = _1 }.present?\n",
            "return 1 if any_errors? { o = inspect(it) }\n",
            "return 1 if any_errors? { o = it }.present?\n",
            "if x?(bar) || y? { z = baz }\n  foo\nend\n",
            "raise StandardError unless foo ||= bar\n",
            "if ()\nend\n",
            "unless ()\nend\n",
            "return unless %i[asc desc].include?(order = params[:order])\n",
            "if defined?(foo.bar = ())\n  foo.bar = false\nend\n",
            "if defined?(test = 10)\nend\n",
        ]);
    }

    #[test]
    fn accepts_discarded_assignments_in_multi_statement_groups() {
        assert_no_offenses(&[
            "if !a || (\n  b = c\n  b == 'd'\n)\n  foo\nend\n",
            "if a || (\n  foo\n  b = c\n)\n  bar\nend\n",
            "if a || (\n  obj.foo = c\n  obj.foo == 'd'\n)\n  bar\nend\n",
            "if (\n  b = c\n  b == 'd'\n)\n  foo\nend\n",
        ]);
    }

    #[test]
    fn accepts_safe_assignments() {
        assert_no_offenses(&[
            "if (test = 10)\nend\n",
            "if (test[0] = 10)\nend\n",
            "if (test = foo && bar == baz)\nend\n",
            "if (test = foo || bar == baz)\nend\n",
        ]);
    }

    // The examples below were checked against RuboCop 1.92 with both parser
    // engines.

    #[test]
    fn flags_assignments_reached_through_other_nodes() {
        assert_eq!(
            positions("def m\n  if super(x = 1); end\nend\n"),
            vec![(2, 14)]
        );
        assert_eq!(
            positions("def m\n  if yield(x = 1); end\nend\n"),
            vec![(2, 14)]
        );
        assert_eq!(positions("if x = y = 1; end\n"), vec![(1, 6), (1, 10)]);
        assert_eq!(positions("if [a = 1]; end\n"), vec![(1, 7)]);
        assert_eq!(positions("if begin; x = 1; end; end\n"), vec![(1, 13)]);
        assert_eq!(positions("if a.b = c = 1; end\n"), vec![(1, 8), (1, 12)]);
        assert_eq!(positions("if a.b=(1); end\n"), vec![(1, 7)]);
        assert_eq!(positions("if x ||= y = 1; end\n"), vec![(1, 12)]);
        assert_eq!(positions("if a.b ||= y = 1; end\n"), vec![(1, 14)]);
        assert_eq!(positions("if x ? y = 1 : z; end\n"), vec![(1, 10)]);
        assert_eq!(positions("a = 1 if b = 2\n"), vec![(1, 12)]);
        assert_eq!(positions("if A::B = 1; end\n"), vec![(1, 9)]);
        assert_eq!(positions("unless x = 1; end\n"), vec![(1, 10)]);
        assert_eq!(positions("if /(?<a>.)/ =~ [c = 1]; end\n"), vec![(1, 20)]);
        assert_eq!(
            positions("if (for a in b; c = 1; end); end\n"),
            vec![(1, 19)]
        );
        assert_eq!(positions("if (x = 1) && y = 2; end\n"), vec![(1, 17)]);
        assert_eq!(
            positions("if x = 1 then 2 elsif y = 2 then 3 end\n"),
            vec![(1, 6), (1, 25)]
        );
    }

    #[test]
    fn flags_single_statement_bodies_inside_a_condition() {
        assert_eq!(positions("if begin; x = 1; y; end; end\n"), vec![(1, 13)]);
        assert_eq!(
            positions("if begin; x = 1; rescue; end; end\n"),
            vec![(1, 13)]
        );
        assert_eq!(
            positions("if begin; y; rescue; x = 1; end; end\n"),
            vec![(1, 24)]
        );
        assert_eq!(positions("if (if z then a = 1 end); end\n"), vec![(1, 17)]);
        assert_eq!(
            positions("if (case z; when 1 then a = 1; end); end\n"),
            vec![(1, 27)]
        );
        assert_eq!(positions("if (def m; a = 1; end); end\n"), vec![(1, 14)]);
    }

    #[test]
    fn accepts_statements_of_multi_statement_bodies_inside_a_condition() {
        // RuboCop's AST groups these in a `begin`, like parentheses; only a
        // plain `begin … end` holds its statements directly.
        assert_no_offenses(&[
            "if begin; x = 1; y; rescue; end; end\n",
            "if begin; x = 1; y; ensure; z; end; end\n",
            "if begin; y; rescue; x = 1; z; end; end\n",
            "if (z ? (a = 1; b) : c); end\n",
            "if (if z then a = 1; b end); end\n",
            "if (if z then b else a = 1; c end); end\n",
            "if (case z; when 1 then a = 1; b; end); end\n",
            "if (def m; a = 1; b; end); end\n",
            "if \"#{x = 1; y}\"; end\n",
            "if (x; y = 1; z); end\n",
            "if a and\n  begin\n    require 'b'\n    C = d || e\n  rescue LoadError\n  end\nend\n",
        ]);
    }

    #[test]
    fn reports_nested_conditions_once() {
        assert_eq!(positions("if (foo if y = 1); end\n"), vec![(1, 14)]);
        assert_eq!(
            positions("while (a unless b = 1) && c = 2; end\n"),
            vec![(1, 19), (1, 29)]
        );
    }

    #[test]
    fn checks_guards_and_post_loops_only_from_an_enclosing_condition() {
        assert_eq!(
            positions("if (case x; in a if b = 1 then true; end); end\n"),
            vec![(1, 23)]
        );
        assert_eq!(
            positions("if (begin; end while x = 1); end\n"),
            vec![(1, 24)]
        );
    }

    #[test]
    fn accepts_other_lookalikes() {
        assert_no_offenses(&[
            "case x\nin a if b = 1\nend\n",
            "case x\nin a unless b = 1\nend\n",
            "begin; end while x = 1\n",
            "begin; end until x = 1\n",
            "if foo(x = 1); end\n",
            "if !(x = 1); end\n",
            "if (x = 1; y); end\n",
            "if (y; x = 1); end\n",
            "if \"#{x = 1}\"; end\n",
            "if foo(a).b ||= 1; end\n",
            "if a[b = 1] += 1; end\n",
            "while (a, b = foo); end\n",
            "x = (y = 1) ? 2 : 3\n",
            "if -> { x = 1 }; end\n",
            "def m\n  if super(x = 1) { }; end\nend\n",
            "if a.b(x = 1) == c; end\n",
            "if /(?<a>.)/ =~ (c = 1); end\n",
            "if /(?<a>#{d = 1})/ =~ e; end\n",
            "case x = 1\nwhen 1 then 2\nend\n",
            "x = 1\nif x\n  y = 2\nend\n",
        ]);
    }
}
