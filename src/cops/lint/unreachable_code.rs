//! `Lint/UnreachableCode` — flags statements that follow a flow-of-control
//! statement in the same body.
//!
//! Matches RuboCop's semantics. In every statement sequence (a method,
//! block, branch or `begin` body, the program, parentheses), every statement
//! after a non-final flow statement is reported. Flow statements are:
//!
//! - `return`, `next`, `break`, `retry`, `redo`;
//! - `raise`, `fail`, `throw`, `exit`, `exit!` and `abort` called without a
//!   receiver or on `Kernel`/`::Kernel`, unless (receiver-less) the method
//!   was seen redefined earlier in the file or the call sits inside an
//!   `instance_eval` block, where `self` is unknown;
//! - parentheses or `begin` bodies containing one; an `if`/`unless`/ternary
//!   whose branches all end in one (an `else` is required); a `case` whose
//!   `when`/`in` branches and `else` all do.
//!
//! Like RuboCop, a redefinition only counts once it has been looked at as a
//! non-final statement of a sequence that was checked before the call.
//!
//! The cop walks the tree itself from the program node, in RuboCop's
//! (pre-)order, to carry that state; an `unless` is walked `else` body
//! first, as the parser gem stores it as `(if c else-body body)`.

use std::collections::HashSet;

use ruby_prism::{CallNode, Node, StatementsNode, UnlessNode, Visit};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/UnreachableCode` cop.
pub struct UnreachableCode;

impl Cop for UnreachableCode {
    fn name(&self) -> &'static str {
        "Lint/UnreachableCode"
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
        let mut walker = Walker {
            cop: self,
            file,
            out,
            redefined: HashSet::new(),
            instance_eval_depth: 0,
        };
        walker.visit_program_node(&program);
    }
}

/// Methods that end the flow unless redefined.
const FLOW_METHODS: [&[u8]; 6] = [b"raise", b"fail", b"throw", b"exit", b"exit!", b"abort"];

/// Walks the whole tree, checking each statement sequence in pre-order.
struct Walker<'a, 'pr> {
    cop: &'a UnreachableCode,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    /// Flow methods seen redefined so far.
    redefined: HashSet<&'pr [u8]>,
    /// How many `instance_eval` blocks enclose the current node.
    instance_eval_depth: usize,
}

impl<'pr> Visit<'pr> for Walker<'_, 'pr> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        // The parser gem's block wraps the whole call, receiver included.
        let instance_eval = node.name().as_slice() == b"instance_eval"
            && node
                .block()
                .is_some_and(|block| block.as_block_node().is_some());
        self.instance_eval_depth += usize::from(instance_eval);
        ruby_prism::visit_call_node(self, node);
        self.instance_eval_depth -= usize::from(instance_eval);
    }

    fn visit_unless_node(&mut self, node: &UnlessNode<'pr>) {
        // The parser gem stores `unless c; A; else; B; end` as
        // `(if c B A)`: RuboCop walks the `else` body first.
        self.visit(&node.predicate());
        if let Some(else_clause) = node.else_clause() {
            self.visit_else_node(&else_clause);
        }
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
    }

    fn visit_statements_node(&mut self, node: &StatementsNode<'pr>) {
        self.check_sequence(node);
        ruby_prism::visit_statements_node(self, node);
    }
}

impl<'pr> Walker<'_, 'pr> {
    /// RuboCop's `on_begin`: report everything after a non-final flow
    /// statement. (One-statement sequences never report, and RuboCop looks
    /// at no statement of them.)
    fn check_sequence(&mut self, node: &StatementsNode<'pr>) {
        let body = node.body();
        if body.len() < 2 {
            return;
        }
        let last = body.len() - 1;
        let mut flow_reached = false;
        for (index, statement) in body.iter().enumerate() {
            if flow_reached {
                let (line, col) = super::array_literal_in_regexp::reference_line_col(
                    statement.location().start_offset(),
                    self.file,
                );
                self.out.push(Offense {
                    correction: None,
                    cop: self.cop.name(),
                    severity: self.cop.severity(),
                    message: "Unreachable code detected.".to_string(),
                    line,
                    end_line: self.file.last_line_of(statement.location()),
                    col,
                });
            } else if index < last && self.is_flow(&statement) {
                flow_reached = true;
            }
        }
    }

    /// RuboCop's `flow_expression?`; records redefinitions it comes across.
    fn is_flow(&mut self, node: &Node<'pr>) -> bool {
        match node {
            Node::ReturnNode { .. }
            | Node::NextNode { .. }
            | Node::BreakNode { .. }
            | Node::RetryNode { .. }
            | Node::RedoNode { .. } => true,
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                is_flow_method_call(&call) && self.reports_flow_call(&call)
            }
            Node::ParenthesesNode { .. } => node
                .as_parentheses_node()
                .unwrap()
                .body()
                .is_some_and(|body| self.is_flow_body(&body)),
            Node::BeginNode { .. } => {
                let begin = node.as_begin_node().unwrap();
                // With `rescue`/`ensure`, the parser gem's `kwbegin` holds
                // that clause, which never counts.
                begin.rescue_clause().is_none()
                    && begin.ensure_clause().is_none()
                    && begin
                        .statements()
                        .is_some_and(|body| self.is_flow_statements(&body))
            }
            Node::IfNode { .. } => {
                let branch = node.as_if_node().unwrap();
                let (Some(body), Some(other)) = (branch.statements(), branch.subsequent()) else {
                    return false;
                };
                self.is_flow_statements(&body) && self.is_flow_else(&other)
            }
            Node::UnlessNode { .. } => {
                // The parser gem's `(if c else-body body)`: `else` first.
                let branch = node.as_unless_node().unwrap();
                let (Some(body), Some(other)) = (branch.statements(), branch.else_clause()) else {
                    return false;
                };
                self.is_flow_else(&other.as_node()) && self.is_flow_statements(&body)
            }
            Node::CaseNode { .. } => {
                let case = node.as_case_node().unwrap();
                self.is_flow_case(case.else_clause(), case.conditions().iter())
            }
            Node::CaseMatchNode { .. } => {
                let case = node.as_case_match_node().unwrap();
                self.is_flow_case(case.else_clause(), case.conditions().iter())
            }
            Node::DefNode { .. } => {
                let name = node.as_def_node().unwrap().name().as_slice();
                if FLOW_METHODS.contains(&name) {
                    self.redefined.insert(name);
                }
                false
            }
            _ => false,
        }
    }

    /// A branch body: flow if any of its statements is.
    fn is_flow_statements(&mut self, body: &StatementsNode<'pr>) -> bool {
        body.body().iter().any(|statement| self.is_flow(&statement))
    }

    fn is_flow_body(&mut self, body: &Node<'pr>) -> bool {
        match body.as_statements_node() {
            Some(statements) => self.is_flow_statements(&statements),
            None => self.is_flow(body),
        }
    }

    /// An `else` clause (its body) or an `elsif` (the whole `if`).
    fn is_flow_else(&mut self, node: &Node<'pr>) -> bool {
        match node.as_else_node() {
            Some(clause) => clause
                .statements()
                .is_some_and(|body| self.is_flow_statements(&body)),
            None => self.is_flow(node),
        }
    }

    /// RuboCop's `check_case`: the `else` and then every branch.
    fn is_flow_case(
        &mut self,
        else_clause: Option<ruby_prism::ElseNode<'pr>>,
        branches: impl Iterator<Item = Node<'pr>>,
    ) -> bool {
        let Some(else_clause) = else_clause else {
            return false;
        };
        if !self.is_flow_else(&else_clause.as_node()) {
            return false;
        }
        for branch in branches {
            let body = match &branch {
                Node::WhenNode { .. } => branch.as_when_node().unwrap().statements(),
                Node::InNode { .. } => branch.as_in_node().unwrap().statements(),
                _ => None,
            };
            if !body.is_some_and(|body| self.is_flow_statements(&body)) {
                return false;
            }
        }
        true
    }

    /// RuboCop's `report_on_flow_command?` for a flow method call.
    fn reports_flow_call(&self, call: &CallNode<'pr>) -> bool {
        if call.receiver().is_some() {
            return true;
        }
        self.instance_eval_depth == 0 && !self.redefined.contains(call.name().as_slice())
    }
}

/// `raise`, `fail`, `throw`, `exit`, `exit!` or `abort` without a receiver
/// or on `Kernel`/`::Kernel`, without `&.` or a literal block.
fn is_flow_method_call(call: &CallNode<'_>) -> bool {
    if !FLOW_METHODS.contains(&call.name().as_slice())
        || call.is_safe_navigation()
        || call
            .block()
            .is_some_and(|block| block.as_block_node().is_some())
    {
        return false;
    }
    match call.receiver() {
        None => true,
        Some(receiver) => is_kernel(&receiver),
    }
}

/// `Kernel` or `::Kernel`.
fn is_kernel(node: &Node<'_>) -> bool {
    match node {
        Node::ConstantReadNode { .. } => {
            node.as_constant_read_node().unwrap().name().as_slice() == b"Kernel"
        }
        Node::ConstantPathNode { .. } => {
            let path = node.as_constant_path_node().unwrap();
            path.parent().is_none() && path.name().is_some_and(|name| name.as_slice() == b"Kernel")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/unreachable_code_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). As in RuboCop's
    //! Prism runs, `retry` outside `rescue` is a syntax error and is only
    //! exercised inside one.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UnreachableCode")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    /// The spec's `wrap`: the snippet inside `def something` /
    /// `array.each do |item|`, indented by four spaces (snippet line `n` is
    /// file line `n + 2`).
    fn wrap(body: &str) -> String {
        let body: String = body.lines().map(|line| format!("    {line}\n")).collect();
        format!("def something\n  array.each do |item|\n{body}  end\nend\n")
    }

    /// Asserts the source parses: a fixture with a syntax error would pass
    /// vacuously.
    fn assert_parses(source: &str) {
        let result = ruby_prism::parse(source.as_bytes());
        assert!(
            result.errors().next().is_none(),
            "fixture must parse: {source}"
        );
    }

    fn assert_flags(source: &str, expected: &[(u32, u32)]) {
        assert_parses(source);
        assert_eq!(positions(source), expected, "for {source}");
    }

    fn assert_accepts(source: &str) {
        assert_parses(source);
        assert!(offenses(source).is_empty(), "should not flag {source}");
    }

    /// Every flow statement the spec loops over, minus `retry`.
    const FLOWS: [&str; 10] = [
        "return", "next", "break", "redo", "throw", "raise", "fail", "exit", "exit!", "abort",
    ];

    /// The redefinable ones.
    const METHODS: [&str; 6] = ["throw", "raise", "fail", "exit", "exit!", "abort"];

    #[test]
    fn flags_every_statement_after_a_flow_statement() {
        let source = wrap("return\nbar\nbaz\n");
        let found = offenses(&source);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col), (4, 5));
        assert_eq!((found[1].line, found[1].col), (5, 5));
        assert_eq!(found[0].message, "Unreachable code detected.");
    }

    #[test]
    fn flags_statement_after_flow() {
        for t in FLOWS {
            assert_flags(&wrap(&format!("{t}\nbar\n")), &[(4, 5)]);
            assert_flags(&wrap(&format!("begin\n  {t}\n  bar\nend\n")), &[(5, 7)]);
        }
    }

    #[test]
    fn flags_flow_in_all_if_branches() {
        for t in FLOWS {
            assert_flags(
                &wrap(&format!("if cond\n  {t}\nelse\n  {t}\nend\nbar\n")),
                &[(8, 5)],
            );
            assert_flags(
                &wrap(&format!(
                    "if cond\n  something\n  {t}\nelse\n  something2\n  {t}\nend\nbar\n"
                )),
                &[(10, 5)],
            );
            assert_flags(
                &wrap(&format!(
                    "if cond\n  something\n  {t}\nelsif cond2\n  something2\n  {t}\nelse\n  something3\n  {t}\nend\nbar\n"
                )),
                &[(13, 5)],
            );
        }
    }

    #[test]
    fn flags_flow_in_all_case_branches() {
        for t in FLOWS {
            assert_flags(
                &wrap(&format!(
                    "case cond\nwhen 1\n  something\n  {t}\nwhen 2\n  something2\n  {t}\nelse\n  something3\n  {t}\nend\nbar\n"
                )),
                &[(14, 5)],
            );
            assert_flags(
                &wrap(&format!(
                    "case cond\nin 1\n  something\n  {t}\nin 2\n  something2\n  {t}\nelse\n  something3\n  {t}\nend\nbar\n"
                )),
                &[(14, 5)],
            );
        }
    }

    #[test]
    fn accepts_conditional_and_final_flow() {
        for t in FLOWS {
            assert_accepts(&wrap(&format!("{t} if cond\nbar\n")));
            assert_accepts(&wrap(&format!("{t} if cond\n")));
            assert_accepts(&wrap(&format!("if cond\n  {t}\nelse\n  {t}\nend\n")));
        }
    }

    #[test]
    fn accepts_flow_missing_from_some_branch() {
        for t in FLOWS {
            for body in [
                format!("if cond\n  something\n  {t}\nelse\n  something2\nend\nbar\n"),
                format!("if cond\n  something\n  {t}\nend\nbar\n"),
                format!("if cond\n  something\nelse\n  something2\n  {t}\nend\nbar\n"),
                format!(
                    "if cond\n  something\n  {t}\nelsif cond2\n  something2\nelse\n  something3\n  {t}\nend\nbar\n"
                ),
                format!(
                    "case cond\nwhen 1\n  something\n  {t}\nwhen 2\n  something2\n  {t}\nend\nbar\n"
                ),
                format!(
                    "case cond\nin 1\n  something\n  {t}\nin 2\n  something2\n  {t}\nend\nbar\n"
                ),
            ] {
                assert_accepts(&wrap(&body));
            }
        }
    }

    #[test]
    fn flags_redefined_method_called_after_instance_eval() {
        for t in METHODS {
            let source = format!(
                "class Dummy\n  def {t}; end\nend\n\nd = Dummy.new\nd.instance_eval do\n  {t}\n  bar\nend\n\n{t}\nbar\n"
            );
            assert_flags(&source, &[(12, 1)]);
        }
    }

    #[test]
    fn flags_method_redefined_only_in_a_nested_definition() {
        for t in METHODS {
            assert_flags(
                &format!("def foo\n  def {t}; end\nend\n\n{t}\nbar\n"),
                &[(6, 1)],
            );
            assert_flags(
                &format!("def foo\n  def self.{t}; end\nend\n\n{t}\nbar\n"),
                &[(6, 1)],
            );
            // The nested body is checked after the outer sequence.
            assert_flags(
                &format!("def foo\n  def {t}; end\n  1\nend\n{t}\nbar\n"),
                &[(6, 1)],
            );
        }
    }

    #[test]
    fn accepts_redefined_methods() {
        for t in METHODS {
            assert_accepts(&wrap(&format!("def {t}; end\n{t}\nbar\n")));
            assert_accepts(&wrap(&format!("def {t}\n  {t}\n  bar\nend\n\n{t}\nbar\n")));
            assert_accepts(&wrap(&format!("def self.{t}; end\n{t}\nbar\n")));
            assert_accepts(&wrap(&format!(
                "def self.{t}\n  {t}\n  bar\nend\n\n{t}\nbar\n"
            )));
        }
    }

    #[test]
    fn accepts_calls_inside_instance_eval() {
        for t in METHODS {
            let class = format!("class Dummy\n  def {t}; end\nend\n\nd = Dummy.new\n");
            for block in [
                format!("d.instance_eval do\n  {t}\n  bar\nend\n"),
                format!("d.instance_eval do\n  {t}\n  _1\nend\n"),
                format!("d.instance_eval do\n  {t}\n  it\nend\n"),
                format!(
                    "d.instance_eval do\n  d2 = Dummy.new\n  d2.instance_eval do\n    {t}\n    bar\n  end\nend\n"
                ),
                format!("d.instance_eval do\n  Dummy.{t}\n  foo\nend\n"),
            ] {
                assert_accepts(&format!("{class}{block}"));
            }
        }
    }

    #[test]
    fn flags_kernel_calls_even_when_redefined() {
        for t in METHODS {
            assert_flags(&format!("def {t}; end\n\nKernel.{t}\nfoo\n"), &[(4, 1)]);
            assert_flags(&format!("def {t}; end\n\n::Kernel.{t}\nfoo\n"), &[(4, 1)]);
            assert_flags(
                &format!(
                    "class Dummy\n  def {t}; end\nend\n\nd = Dummy.new\nd.instance_eval do\n  Kernel.{t}\n  foo\nend\n"
                ),
                &[(8, 3)],
            );
            assert_accepts(&format!("def {t}; end\n\nDummy.{t}\nfoo\n"));
        }
    }

    #[test]
    fn flags_retry_inside_rescue() {
        assert_flags("begin\n  a\nrescue\n  retry\n  bar\nend\n", &[(5, 3)]);
    }

    #[test]
    fn flags_flow_in_parentheses_and_nested_bodies() {
        assert_flags("def m\n  (return)\n  a\nend\n", &[(3, 3)]);
        assert_flags("def m\n  (a; return; b)\n  c\nend\n", &[(2, 15), (3, 3)]);
        assert_flags(
            "def m\n  unless a\n    raise\n  else\n    return\n  end\n  b\nend\n",
            &[(7, 3)],
        );
        assert_flags("def m\n  a ? (return) : (raise)\n  b\nend\n", &[(3, 3)]);
        assert_flags("def m\n  return 1\n  b\nend\n", &[(3, 3)]);
        assert_flags("raise ArgumentError, 'x'\nb\n", &[(2, 1)]);
        assert_flags("exit(1); b\n", &[(1, 10)]);
        // Every sequence is checked, including inside unreachable code.
        assert_flags(
            "def m\n  return\n  if a\n    return\n    b\n  end\nend\n",
            &[(3, 3), (5, 5)],
        );
    }

    #[test]
    fn unless_branches_are_checked_else_first() {
        // The redefinition in the `unless` body comes after the `else` body.
        assert_flags(
            "unless x\n  def raise; end\n  foo\nelse\n  raise\n  bar\nend\n",
            &[(6, 3)],
        );
        // `flow_expression?` looks at the `else` body first too: here the
        // `else` body is not flow, so the body's `def raise` is never seen.
        assert_flags(
            "def m\n  unless x\n    def raise; end\n  else\n    a\n  end\n  b\nend\ndef n\n  raise\n  c\nend\n",
            &[(11, 3)],
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "def m\n  return if a\n  b\nend\n",
            "def m\n  a.raise\n  b\nend\n",
            "def m\n  Kernel&.raise\n  b\nend\n",
            "def m\n  raise do\n  end\n  b\nend\n",
            "def m\n  begin\n    return\n  rescue\n    a\n  end\n  b\nend\n",
            "def m\n  case a\n  when 1 then return\n  end\n  b\nend\n",
            "def m\n  case a\n  when 1\n  else return\n  end\n  b\nend\n",
            "def m\n  if a\n    return\n  else\n  end\n  b\nend\n",
            "def m\n  while a\n    return\n  end\n  b\nend\n",
            "def m\n  -> { return }\n  b\nend\n",
            "def m\n  raises\n  b\nend\n",
            "def m\n  return\nend\n",
            // The parser gem stores `unless` as `(if c else-body body)`, so
            // RuboCop sees the `else` branch first.
            "unless x\n  raise\n  foo\nelse\n  def raise; end\n  bar\nend\n",
        ] {
            assert_accepts(source);
        }
    }
}
