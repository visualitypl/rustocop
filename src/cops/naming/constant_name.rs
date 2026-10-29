//! `Naming/ConstantName` — flags constants not written in
//! SCREAMING_SNAKE_CASE.
//!
//! Matches RuboCop's semantics: every constant assignment is checked,
//! including multiple assignment, `rescue => Name` and `for Name in`
//! targets, unless the assigned value might be a class or module, which may
//! be CamelCase. Those values are a block (`foo do … end`, `-> { }`,
//! `super { |x| }`, but not a block using `_1` or `it`), a constant, another constant assignment, a
//! method call (not `&.`) on no receiver or a non-literal one (`5.freeze`
//! is checked, `Class.new` is not), or an `if`/`unless`/ternary with a
//! branch that is a single constant. `Name ||= value` judges `value`;
//! `&&=`, `+=` and targets have no value to judge. The offense is reported
//! at the constant's own name (`Bar` in `Foo::Bar = 1`).

use ruby_prism::{Location, Node, StatementsNode};

use crate::{
    cops::{
        Cop, NodeContext,
        ast::{is_const, is_literal},
    },
    offense::Offense,
    source::SourceFile,
};

use super::format::is_screaming_snake_case;

/// The `Naming/ConstantName` cop.
pub struct ConstantName;

impl Cop for ConstantName {
    fn name(&self) -> &'static str {
        "Naming/ConstantName"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some((name, value)) = assignment(node) else {
            return;
        };
        if value.is_some_and(|value| may_be_a_class(&value)) {
            return;
        }
        let written = String::from_utf8_lossy(name.as_slice());
        // An empty name only comes from a tree recovered from a syntax error.
        if written.is_empty() || is_screaming_snake_case(&written) {
            return;
        }
        let (line, col) = file.line_col(name.start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Use SCREAMING_SNAKE_CASE for constants.".to_string(),
            line,
            end_line: line,
            col,
        });
    }
}

/// A constant assignment (RuboCop's `casgn`): the assigned name, and the
/// value RuboCop judges, if any (the right-hand side of `=` and `||=`).
fn assignment<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Option<Node<'pr>>)> {
    Some(match node {
        Node::ConstantWriteNode { .. } => {
            let write = node.as_constant_write_node().unwrap();
            (write.name_loc(), Some(write.value()))
        }
        Node::ConstantPathWriteNode { .. } => {
            let write = node.as_constant_path_write_node().unwrap();
            (write.target().name_loc(), Some(write.value()))
        }
        Node::ConstantOrWriteNode { .. } => {
            let write = node.as_constant_or_write_node().unwrap();
            (write.name_loc(), Some(write.value()))
        }
        Node::ConstantPathOrWriteNode { .. } => {
            let write = node.as_constant_path_or_write_node().unwrap();
            (write.target().name_loc(), Some(write.value()))
        }
        Node::ConstantAndWriteNode { .. } => {
            (node.as_constant_and_write_node().unwrap().name_loc(), None)
        }
        Node::ConstantOperatorWriteNode { .. } => (
            node.as_constant_operator_write_node().unwrap().name_loc(),
            None,
        ),
        Node::ConstantPathAndWriteNode { .. } => (
            node.as_constant_path_and_write_node()
                .unwrap()
                .target()
                .name_loc(),
            None,
        ),
        Node::ConstantPathOperatorWriteNode { .. } => (
            node.as_constant_path_operator_write_node()
                .unwrap()
                .target()
                .name_loc(),
            None,
        ),
        Node::ConstantTargetNode { .. } => (node.location(), None),
        Node::ConstantPathTargetNode { .. } => (
            node.as_constant_path_target_node().unwrap().name_loc(),
            None,
        ),
        _ => return None,
    })
}

/// RuboCop's `allowed_assignment?`: whether `value` might be a class or
/// module, so that the constant may be CamelCase.
fn may_be_a_class(value: &Node<'_>) -> bool {
    match value {
        // `Class.new(...)` and `Struct.new(...)` are method calls on a
        // non-literal receiver, so RuboCop's dedicated matcher adds nothing.
        Node::CallNode { .. } => {
            let call = value.as_call_node().unwrap();
            if let Some(block) = call.block().and_then(|block| block.as_block_node()) {
                return !uses_implicit_parameters(block.parameters());
            }
            !call.is_safe_navigation()
                && call
                    .receiver()
                    .is_none_or(|receiver| !is_literal_receiver(&receiver))
        }
        // `super … do |x| end`: a block, as on any other call.
        Node::SuperNode { .. } => value
            .as_super_node()
            .unwrap()
            .block()
            .and_then(|block| block.as_block_node())
            .is_some_and(|block| !uses_implicit_parameters(block.parameters())),
        Node::ForwardingSuperNode { .. } => value
            .as_forwarding_super_node()
            .unwrap()
            .block()
            .is_some_and(|block| !uses_implicit_parameters(block.parameters())),
        Node::LambdaNode { .. } => {
            !uses_implicit_parameters(value.as_lambda_node().unwrap().parameters())
        }
        Node::ConstantWriteNode { .. } | Node::ConstantPathWriteNode { .. } => true,
        Node::IfNode { .. } | Node::UnlessNode { .. } => {
            let mut branches = Vec::new();
            collect_branches(value, &mut branches);
            branches.iter().any(is_const)
        }
        _ => is_const(value),
    }
}

/// Whether a block's parameters are `_1`-style or `it`: the parser gem's
/// `numblock` and `itblock`, which RuboCop does not count as blocks here.
fn uses_implicit_parameters(parameters: Option<Node<'_>>) -> bool {
    parameters.is_some_and(|parameters| {
        matches!(
            parameters,
            Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. }
        )
    })
}

/// RuboCop's `literal_receiver?`: a literal, or a parenthesized single
/// literal (`(1..5).freeze`).
fn is_literal_receiver(receiver: &Node<'_>) -> bool {
    if let Some(parentheses) = receiver.as_parentheses_node() {
        return parentheses
            .body()
            .and_then(|body| single_statement(body.as_statements_node()))
            .is_some_and(|statement| is_literal(&statement));
    }
    is_literal(receiver)
}

/// The branch bodies of an `if`/`unless`, through `elsif` chains, as
/// RuboCop's `IfNode#branches` sees them: a body of several statements is a
/// `begin` (never a constant), and empty bodies are dropped.
fn collect_branches<'pr>(node: &Node<'pr>, branches: &mut Vec<Node<'pr>>) {
    let (body, subsequent) = if let Some(if_node) = node.as_if_node() {
        (if_node.statements(), if_node.subsequent())
    } else if let Some(unless) = node.as_unless_node() {
        (
            unless.statements(),
            unless.else_clause().map(|clause| clause.as_node()),
        )
    } else {
        return;
    };
    branches.extend(single_statement(body));
    match subsequent {
        Some(Node::ElseNode { .. }) => {
            let else_node = subsequent.unwrap();
            let statements = else_node.as_else_node().unwrap().statements();
            branches.extend(single_statement(statements));
        }
        Some(elsif) => collect_branches(&elsif, branches),
        None => {}
    }
}

/// The only statement of `statements`, if it holds exactly one.
fn single_statement<'pr>(statements: Option<StatementsNode<'pr>>) -> Option<Node<'pr>> {
    let body = statements?.body();
    let mut iter = body.iter();
    match (iter.next(), iter.next()) {
        (Some(statement), None) => Some(statement),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/naming/constant_name_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::pipeline::lint_str;

    const MESSAGE: &str = "Use SCREAMING_SNAKE_CASE for constants.";

    fn found(source: &str) -> Vec<(u32, u32)> {
        let parsed = ruby_prism::parse(source.as_bytes());
        assert!(
            parsed.errors().next().is_none(),
            "fixture must parse: {source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/ConstantName")
            .map(|o| {
                assert_eq!(o.message, MESSAGE);
                (o.line, o.col)
            })
            .collect()
    }

    #[test]
    fn flags_rubocop_spec_offenses() {
        assert_eq!(found("TopCase = 5\n"), [(1, 1)]);
        assert_eq!(found("TopCase = (1..5).freeze\n"), [(1, 1)]);
        assert_eq!(found("TopCase = 5.freeze\n"), [(1, 1)]);
        assert_eq!(found("Nö = 'no'\n"), [(1, 1)]);
        assert_eq!(
            found("TopCase, Test2, TEST_3 = 5, 6, 7\n"),
            [(1, 1), (1, 10)]
        );
        assert_eq!(found("TOP_test = 5\n"), [(1, 1)]);
        assert_eq!(found("Bar = Foo = 4\n"), [(1, 7)]);
        assert_eq!(
            found("::AnythingGoes = 30\na::Bar_foo = 10\n"),
            [(1, 3), (2, 4)]
        );
        assert_eq!(
            found("TopCase = if true then 'foo' else 'bar' end\n"),
            [(1, 1)]
        );
    }

    #[test]
    fn accepts_rubocop_spec_no_offenses() {
        for source in [
            "TOP_TEST = 5\n",
            "TOP_TEST, TEST_2 = 5, 6\n",
            "TÖP_TEST = 5\n",
            "AnythingGoes = test\n",
            "AnythingGoes ||= test\n",
            "Invalid = Class.new(StandardError)\n",
            "Invalid ||= Class.new(StandardError)\n",
            "Investigation = Struct.new(:offenses, :errors)\n",
            "Investigation ||= Struct.new(:offenses, :errors)\n",
            "AnythingGoes = test do\n  do_something\nend\n",
            "Parser::CurrentRuby = Parser::Ruby21\n",
            "Bar = Foo = Qux\n",
            "CONST = if condition\n  foo\nelse\nend\n",
            "Investigation = if true then Foo else Bar end\n",
            "Investigation = if true then \"foo\" else Bar end\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn flags_assignments_without_a_judgeable_value() {
        assert_eq!(found("Grault += 1\n"), [(1, 1)]);
        assert_eq!(found("Garply &&= 1\n"), [(1, 1)]);
        assert_eq!(found("Foo::BarBaz += 1\n"), [(1, 6)]);
        assert_eq!(found("Foo::BarBaz &&= Qux\n"), [(1, 6)]);
        assert_eq!(found("Foo::BarBaz, Qux = 1, 2\n"), [(1, 6), (1, 14)]);
        assert_eq!(found("a, (BarBaz, b) = 1, [2, 3]\n"), [(1, 5)]);
        assert_eq!(found("begin\nrescue => FooBar\nend\n"), [(2, 11)]);
        assert_eq!(found("for BazQux in x; end\n"), [(1, 5)]);
    }

    #[test]
    fn flags_values_that_cannot_be_a_class() {
        for source in [
            "Foo = bar { _1 }\n",
            "Foo = bar { it }\n",
            "Foo = -> { _1 }\n",
            "Foo = bar&.baz\n",
            "Foo = 1 + 2\n",
            "Foo = :sym.to_proc\n",
            "Foo = -1.abs\n",
            "Foo = (\"x\").freeze\n",
            "Foo = [x].freeze\n",
            "Foo = { a: 1 }.freeze\n",
            "Foo = \"#{x}\".freeze\n",
            "Foo = <<~X.freeze\n  hi\nX\n",
            "Foo = (a ? Bar : 1)\n",
            "Foo = __FILE__\n",
            "Foo = defined?(x)\n",
            "Foo = x.y ||= 1\n",
            "Foo = BAR ||= 1\n",
            "Foo = /(?<y>.)/ =~ x\n",
            "Foo = case x when 1 then Bar end\n",
            "Foo = if a then 1 elsif b then 2 else 3 end\n",
            "Foo = if a\n  x\n  Bar\nelse\n  1\nend\n",
            "Foo = super\n",
            "Foo = super(1)\n",
            "Foo = super { _1 }\n",
            "Foo = super { it }\n",
            "Foo = super(&b)\n",
            "Foo = super(1) { _1 }\n",
            "Foo = a && Bar\n",
        ] {
            assert_eq!(found(source).len(), 1, "should flag {source:?}");
        }
    }

    #[test]
    fn accepts_values_that_may_be_a_class() {
        for source in [
            "Foo = bar { |x| x }\n",
            "Foo = -> { 1 }\n",
            "Foo = lambda do end\n",
            "Foo = a&.b { 1 }\n",
            "Foo = Foo::Bar.baz do end\n",
            "Foo = a + b\n",
            "Foo = !x\n",
            "Foo = not(x)\n",
            "Foo = a[1]\n",
            "Foo = a.(1)\n",
            "Foo = foo(&:bar)\n",
            "Foo = (x).freeze\n",
            "Foo = (\"a\"; \"b\").freeze\n",
            "Foo = x =~ /(?<y>.)/\n",
            "Foo = __ENCODING__\n",
            "Foo = ::Bar\n",
            "Foo = Bar unless x\n",
            "Foo = x ? Bar : 1\n",
            "Foo = if a then 1 elsif b then Bar end\n",
            "Foo = if a then else Bar end\n",
            "Foo ||= Bar\n",
            "Foo::Bar ||= baz\n",
            "Foo = Bar::BAZ = 1\n",
            "Foo = super do |x| end\n",
            "Foo = super do end\n",
            "Foo = super { |x| x }\n",
            "Foo = super(1) do end\n",
            "class A < B\n  define_method(:build) do\n    Built = super() { |x| x }\n  end\nend\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "FooBar\n",
            "Foo::BarBaz.new\n",
            "foo_bar = 1\n",
            "@fooBar = 1\n",
            "$FooBar = 1\n",
            "def FooBar; end\n",
            "class FooBar; end\n",
            "case x\nin FooBar then 1\nend\n",
        ] {
            assert!(found(source).is_empty(), "should not flag {source:?}");
        }
    }
}
