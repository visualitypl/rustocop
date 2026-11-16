//! `Style/RedundantCondition` — fixed-policy detection and conservative safe corrections.

use ruby_prism::{Node, StatementsNode};

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};

#[cfg(test)]
thread_local! {
    static MATCHED_SOURCE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct RedundantCondition;

impl Cop for RedundantCondition {
    fn name(&self) -> &'static str {
        "Style/RedundantCondition"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(node, Node::IfNode { .. } | Node::UnlessNode { .. })
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !matches!(node, Node::IfNode { .. } | Node::UnlessNode { .. }) {
            return;
        }
        // Main dispatch is preorder: every enclosing conditional has already
        // been checked. Reuse those matches via NodeContext, never rescan a
        // wide ancestor condition for each of its many redundant descendants.
        let mut matches = file
            .redundant_conditions()
            .lock()
            .expect("match cache poisoned");
        if cx.ancestors().any(|ancestor| {
            matches!(ancestor, Node::IfNode { .. } | Node::UnlessNode { .. })
                && matches.contains(&(
                    ancestor.location().start_offset(),
                    ancestor.location().end_offset(),
                ))
        }) {
            return;
        }
        let Some(branches) = redundant(node, file) else {
            return;
        };
        matches.insert((node.location().start_offset(), node.location().end_offset()));
        drop(matches);
        let ternary = node.as_if_node().filter(|n| n.if_keyword_loc().is_none());
        let (start, end) = ternary.filter(|_| !branches.same_methods()).map_or_else(
            || (node.location().start_offset(), node.location().end_offset()),
            |n| {
                (
                    n.then_keyword_loc().unwrap().start_offset(),
                    n.subsequent()
                        .unwrap()
                        .as_else_node()
                        .unwrap()
                        .else_keyword_loc()
                        .end_offset(),
                )
            },
        );
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: if branches.falsy.is_none() {
                "This condition is not needed."
            } else {
                "Use double pipes `||` instead."
            }
            .into(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            correction: cx
                .corrections_requested()
                .then(|| correction(node, &branches, cx, file))
                .flatten(),
        });
    }
}

struct Branches<'pr> {
    condition: Node<'pr>,
    truthy: Option<Node<'pr>>,
    falsy: Option<Node<'pr>>,
}

fn text<'pr>(node: &Node<'pr>) -> Option<&'pr str> {
    std::str::from_utf8(node.location().as_slice()).ok()
}

/// RuboCop's corrector: `a ? a : b` becomes `a || b`, a conditional
/// without `else` its branch, and any other one `a || b` built from the
/// branches (`make_ternary_form`).
fn correction(
    node: &Node<'_>,
    branches: &Branches<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    let ternary = node.as_if_node().filter(|n| n.if_keyword_loc().is_none());
    let falsy_start = |n: &ruby_prism::IfNode<'_>| {
        n.subsequent()?
            .as_else_node()?
            .statements()
            .map(|s| s.location().start_offset())
    };
    // RuboCop's `contains_comments?` on each descendant: the lines from the
    // condition up to the `end`, or to a ternary's else branch.
    let end = match &ternary {
        Some(n) => falsy_start(n)?,
        None => node
            .as_if_node()
            .and_then(|n| n.end_keyword_loc())
            .or_else(|| node.as_unless_node().and_then(|n| n.end_keyword_loc()))?
            .start_offset(),
    };
    let lines = file.line_of(branches.condition.location().start_offset())..file.line_of(end);
    if file
        .comments()
        .iter()
        .any(|c| lines.contains(&file.line_of(c.start)))
    {
        return None;
    }
    let same_methods = branches.same_methods();
    if let Some(n) = ternary.as_ref().filter(|_| !same_methods) {
        let question = n.then_keyword_loc()?;
        let colon = n.subsequent()?.as_else_node()?.else_keyword_loc();
        let mut edits = vec![Correction {
            range: question.start_offset()..colon.end_offset(),
            replacement: "||".into(),
        }];
        if let Some(falsy) = branches
            .falsy
            .as_ref()
            .filter(|f| f.as_range_node().is_some())
        {
            let f = falsy.location();
            edits.push(Correction {
                range: f.start_offset()..f.start_offset(),
                replacement: "(".into(),
            });
            edits.push(Correction {
                range: f.end_offset()..f.end_offset(),
                replacement: ")".into(),
            });
        }
        return super::correction::combine(file, edits);
    }
    let truthy = branches.truthy.as_ref()?;
    let Some(falsy) = branches.falsy.as_ref() else {
        return Some(Correction {
            range: loc.start_offset()..loc.end_offset(),
            replacement: text(truthy)?.to_owned(),
        });
    };
    let parenthesized = |n: &Node<'_>| {
        n.as_call_node()
            .and_then(|c| c.opening_loc())
            .is_some_and(|o| o.as_slice() == b"(")
    };
    let binary = same_methods
        && truthy.as_call_node().is_some_and(|c| {
            c.receiver().is_some()
                && c.call_operator_loc().is_none()
                && super::super::layout::multiline_expression::operator_method(c.name().as_slice())
        });
    let if_source = if same_methods && parenthesized(truthy) {
        text(truthy)?.strip_suffix(')')?.to_owned()
    } else if binary {
        let call = truthy.as_call_node()?;
        format!(
            "{} {} ({}",
            text(&call.receiver()?)?,
            std::str::from_utf8(call.name().as_slice()).ok()?,
            text(&method_argument(truthy)?.1)?
        )
    } else if truthy.as_true_node().is_some() {
        let condition = branches.condition.as_call_node()?;
        let arguments = ast::call_arguments(&condition);
        if arguments.is_empty() || parenthesized(&branches.condition) {
            text(&branches.condition)?.to_owned()
        } else {
            let c = branches.condition.location();
            let selector = condition.message_loc()?.end_offset();
            let first = arguments[0].location().start_offset();
            format!(
                "{}({})",
                std::str::from_utf8(&file.bytes[c.start_offset()..selector]).ok()?,
                std::str::from_utf8(&file.bytes[first..c.end_offset()]).ok()?
            )
        }
    } else {
        text(truthy)?.to_owned()
    };
    let wrapped = |n: &Node<'_>| -> Option<String> {
        let source = text(n)?;
        Some(if require_parentheses(n) {
            format!("({source})")
        } else if n.as_keyword_hash_node().is_some() {
            format!("{{ {source} }}")
        } else {
            source.to_owned()
        })
    };
    let else_source = if binary {
        format!("{})", text(&method_argument(falsy)?.1)?)
    } else if same_methods {
        wrapped(&method_argument(falsy)?.1)?
    } else if require_parentheses(falsy) {
        format!("({})", text(falsy)?)
    } else if let Some(call) = falsy.as_call_node().filter(|c| {
        let name = c.name().as_slice();
        !c.is_safe_navigation()
            && !ast::call_arguments(c).is_empty()
            && !parenthesized(falsy)
            && !super::super::layout::multiline_expression::operator_method(name)
            && !(name.ends_with(b"=") && !matches!(name, b"==" | b"!=" | b"===" | b"<=" | b">="))
    }) {
        let arguments = ast::call_arguments(&call)
            .iter()
            .map(text)
            .collect::<Option<Vec<_>>>()?
            .join(", ");
        format!(
            "{}({arguments})",
            std::str::from_utf8(call.name().as_slice()).ok()?
        )
    } else if let (Some((name, _)), Some((other, value))) = (assignment(truthy), assignment(falsy))
        && name == other
    {
        wrapped(&value)?
    } else {
        text(falsy)?.to_owned()
    };
    let mut replacement = format!("{if_source} || {else_source}");
    if same_methods && parenthesized(truthy) {
        replacement.push(')');
    }
    let parent_send = cx.parent().is_some_and(|p| {
        p.as_call_node().is_some()
            || (p.as_arguments_node().is_some()
                && cx
                    .ancestors()
                    .nth(1)
                    .is_some_and(|g| g.as_call_node().is_some()))
    });
    if parent_send {
        replacement = format!("({replacement})");
    }
    Some(Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement,
    })
}

/// RuboCop's `require_parentheses?`: what binds looser than `||`.
fn require_parentheses(node: &Node<'_>) -> bool {
    match node {
        Node::IfNode { .. } => node
            .as_if_node()
            .is_some_and(|n| n.if_keyword_loc().is_some() && n.end_keyword_loc().is_none()),
        Node::UnlessNode { .. } => node
            .as_unless_node()
            .is_some_and(|n| n.end_keyword_loc().is_none()),
        Node::WhileNode { .. } => node
            .as_while_node()
            .is_some_and(|n| n.closing_loc().is_none()),
        Node::UntilNode { .. } => node
            .as_until_node()
            .is_some_and(|n| n.closing_loc().is_none()),
        Node::RangeNode { .. } | Node::RescueModifierNode { .. } => true,
        Node::AndNode { .. } => node
            .as_and_node()
            .is_some_and(|n| n.operator_loc().as_slice() == b"and"),
        Node::OrNode { .. } => node
            .as_or_node()
            .is_some_and(|n| n.operator_loc().as_slice() == b"or"),
        Node::CallNode { .. } => node
            .as_call_node()
            .is_some_and(|c| c.message_loc().is_some_and(|m| m.as_slice() == b"not")),
        Node::ReturnNode { .. } | Node::NextNode { .. } | Node::BreakNode { .. } => {
            let arguments = match node {
                Node::ReturnNode { .. } => node.as_return_node().unwrap().arguments(),
                Node::NextNode { .. } => node.as_next_node().unwrap().arguments(),
                _ => node.as_break_node().unwrap().arguments(),
            };
            arguments.is_some_and(|a| {
                let a = a.arguments();
                !(a.len() == 1 && a.iter().next().unwrap().as_parentheses_node().is_some())
            })
        }
        Node::YieldNode { .. } => node
            .as_yield_node()
            .is_some_and(|n| n.arguments().is_some() && n.lparen_loc().is_none()),
        Node::SuperNode { .. } => node
            .as_super_node()
            .is_some_and(|n| n.arguments().is_some() && n.lparen_loc().is_none()),
        _ => false,
    }
}

// Statements lists are transparent in RuboCop unless they contain multiple
// expressions, in which case they are a begin node (not just the final value).
fn body(statements: Option<StatementsNode<'_>>) -> Option<Node<'_>> {
    let statements = statements?;
    match statements.body().len() {
        0 => None,
        1 => statements.body().iter().next(),
        _ => Some(statements.as_node()),
    }
}

fn redundant<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Branches<'pr>> {
    #[cfg(test)]
    MATCHED_SOURCE_BYTES.with(|bytes| bytes.set(bytes.get() + node.location().as_slice().len()));
    let branches = if let Some(n) = node.as_if_node() {
        if n.if_keyword_loc().is_some_and(|k| k.as_slice() == b"elsif")
            || (n.if_keyword_loc().is_some() && n.end_keyword_loc().is_none())
        {
            return None;
        }
        let falsy = match n.subsequent() {
            None => None,
            Some(subsequent) => body(subsequent.as_else_node()?.statements()),
        };
        Branches {
            condition: n.predicate(),
            truthy: body(n.statements()),
            falsy,
        }
    } else {
        let n = node.as_unless_node()?;
        n.end_keyword_loc()?;
        // Parser/RuboCop stores unless branches in truth-value order.
        Branches {
            condition: n.predicate(),
            truthy: body(n.else_clause().and_then(|n| n.statements())),
            falsy: body(n.statements()),
        }
    };
    if let Some(falsy) = &branches.falsy {
        if matches!(falsy, Node::IfNode { .. } | Node::UnlessNode { .. })
            || falsy.as_call_node().is_some_and(|n| {
                n.name().as_slice() == b"[]="
                    && !n.is_safe_navigation()
                    && !n.block().is_some_and(|b| b.as_block_node().is_some())
            })
        {
            return None;
        }
        // Typed RuboCop nodes (including explicit keyword begin) may span
        // lines. Only generic statement/grouping nodes have this restriction.
        let ternary = node
            .as_if_node()
            .is_some_and(|n| n.if_keyword_loc().is_none());
        if !ternary
            && (matches!(
                falsy,
                Node::StatementsNode { .. } | Node::ParenthesesNode { .. }
            ) || falsy
                .as_begin_node()
                .is_some_and(|n| n.begin_keyword_loc().is_none()))
            && file.line_of(falsy.location().start_offset()) != file.last_line_of(falsy.location())
        {
            return None;
        }
    }
    let truthy = branches.truthy.as_ref()?;
    let synonymous = ast::same_node(&branches.condition, truthy)
        || (node.as_unless_node().is_none()
            && truthy.as_true_node().is_some()
            && branches
                .falsy
                .as_ref()
                .is_some_and(|n| n.as_true_node().is_none())
            && branches.condition.as_call_node().is_some_and(|n| {
                !n.block().is_some_and(|b| b.as_block_node().is_some())
                    && n.name().as_slice().ends_with(b"?")
                    && !matches!(n.name().as_slice(), b"infinite?" | b"nonzero?")
            }))
        || branches.falsy.as_ref().is_some_and(|falsy| {
            if let (Some((name, value)), Some((other, _))) = (assignment(truthy), assignment(falsy))
                && name == other
                && ast::same_node(&branches.condition, &value)
            {
                return true;
            }
            branches.same_methods()
                && ast::same_node(&branches.condition, &method_argument(truthy).unwrap().1)
        });
    synonymous.then_some(branches)
}

impl Branches<'_> {
    fn same_methods(&self) -> bool {
        let Some((yes, _)) = self.truthy.as_ref().and_then(method_argument) else {
            return false;
        };
        let Some((no, _)) = self.falsy.as_ref().and_then(method_argument) else {
            return false;
        };
        yes.name().as_slice() == no.name().as_slice()
            && ast::same_optional_node(yes.receiver().as_ref(), no.receiver().as_ref())
    }
}

fn assignment<'pr>(node: &Node<'pr>) -> Option<(&'pr [u8], Node<'pr>)> {
    macro_rules! variable {
        ($($access:ident),* $(,)?) => {$(
            if let Some(n) = node.$access() { return Some((n.name().as_slice(), n.value())); }
        )*};
    }
    variable! { as_local_variable_write_node, as_instance_variable_write_node,
    as_class_variable_write_node, as_global_variable_write_node, as_constant_write_node }
    let n = node.as_constant_path_write_node()?;
    // Pinned RuboCop compares the constant's short name, not its namespace.
    Some((n.target().name()?.as_slice(), n.value()))
}

fn method_argument<'pr>(node: &Node<'pr>) -> Option<(ruby_prism::CallNode<'pr>, Node<'pr>)> {
    let call = node.as_call_node()?;
    if call.is_safe_navigation()
        || call.name().as_slice() == b"[]"
        || call.block().is_some_and(|n| n.as_block_node().is_some())
    {
        return None;
    }
    let mut args = ast::call_arguments(&call);
    if args.len() != 1 {
        return None;
    }
    let argument = args.pop().unwrap();
    if matches!(
        argument,
        Node::SplatNode { .. }
            | Node::BlockArgumentNode { .. }
            | Node::ForwardingArgumentsNode { .. }
    ) {
        return None;
    }
    let first = argument
        .as_hash_node()
        .and_then(|n| n.elements().iter().next())
        .or_else(|| {
            argument
                .as_keyword_hash_node()
                .and_then(|n| n.elements().iter().next())
        });
    if first.is_some_and(|n| n.as_assoc_splat_node().is_some()) {
        return None;
    }
    Some((call, argument))
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/redundant_condition_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    #[test]
    fn corrects_as_rubocop() {
        for (before, after) in [
            ("b ? b : c\n", "b || c\n"),
            ("b ? b : 1..2\n", "b || (1..2)\n"),
            (
                "if b\n  b\nelse\n  raise 'foo'\nend\n",
                "b || raise('foo')\n",
            ),
            ("if b\n  b\nend\n", "b\n"),
            ("if a.zero?\n  true\nelse\n  a\nend\n", "a.zero? || a\n"),
            ("foo(if b\n  b\nelse\n  c\nend)\n", "foo((b || c))\n"),
        ] {
            crate::cops::testing::assert_correction(&super::RedundantCondition, before, after);
        }
    }

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantCondition")
            .collect()
    }

    fn assert_offense(source: &str, line: u32, col: u32, end_line: u32, message: &str) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}: {found:?}");
        assert_eq!(
            (found[0].line, found[0].col, found[0].end_line),
            (line, col, end_line),
            "{source}"
        );
        assert_eq!(found[0].message, message, "{source}");
        assert!(found[0].correction.is_none());
    }

    const PIPES: &str = "Use double pipes `||` instead.";
    const UNNEEDED: &str = "This condition is not needed.";

    #[test]
    fn repeated_conditions_report_full_normal_or_question_to_colon_ternary_range() {
        for (source, line, col, end_line, message) in [
            ("if b\n  b\nelse\n  c\nend\n", 1, 1, 5, PIPES),
            (
                "def example\n  if do_something\n    do_something\n  end\nend\n",
                2,
                3,
                4,
                UNNEEDED,
            ),
            ("a = b(x) ? b(x) : c\n", 1, 10, 1, PIPES),
            ("b ?\n  b\n: c\n", 1, 3, 3, PIPES),
            ("unless b\n  y(x, z)\nelse\n  b\nend\n", 1, 1, 5, PIPES),
            ("a = b[:x] ? b[:x] : b[:y]\n", 1, 11, 1, PIPES),
        ] {
            assert_offense(source, line, col, end_line, message);
        }
    }

    #[test]
    fn predicate_true_branches_respect_fixed_allowed_methods() {
        for condition in [
            "a.zero?",
            "a&.zero?",
            "foo? arg",
            "foo?(arg)",
            "obj&.foo? arg",
            "nil?",
            "self.empty?",
        ] {
            assert_offense(
                &format!("if {condition}\n  true\nelse\n  a\nend"),
                1,
                1,
                5,
                PIPES,
            );
        }
        assert_offense("something = a.nil? ? true : a", 1, 20, 1, PIPES);
        for source in [
            "a.nonzero? ? true : false",
            "a.infinite? ? true : false",
            "if a.nonzero?\n  true\nelse\n  false\nend",
            "unless a.zero?\n  a\nelse\n  true\nend",
            "variable = work; variable ? true : a",
            "@variable ? true : a",
            "@@variable ? true : a",
            "$variable ? true : a",
            "CONST ? true : a",
            "a[:key] ? true : a",
            "a.zero? ? true : true",
            "a.zero? ? 'true' : a",
            "a.zero? ? false : a",
            "a.zero? ? 1 : a",
            "if a.zero?\n  true\nend",
            "if a.zero?\n  true\nelse\nend",
            "if a.zero?\nelse\n  a\nend",
            "a.zero? { work } ? true : a",
            "if !a[:key]\n  a\nelse\n  false\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn matches_plain_assignments_by_kind_and_name() {
        for target in [
            "value", "@value", "@@value", "$value", "CONST", "A::CONST", "::CONST",
        ] {
            assert_offense(
                &format!("if foo\n  {target} = foo\nelse\n  {target} = 'bar'\nend"),
                1,
                1,
                5,
                PIPES,
            );
        }
        for source in [
            "if foo.nil?\n  @value = foo\nelse\n  @value = 'bar'\nend",
            "if foo\n  @foo = foo\nelse\n  @baz = 'quux'\nend",
            "if foo\n  value = foo\nelse\n  @value = 'bar'\nend",
            "if foo\n  value ||= foo\nelse\n  value ||= 'bar'\nend",
            "if foo\n  a, b = foo\nelse\n  a, b = other\nend",
            "if foo\n  value = foo\nend",
            "if value\n  value = value\nelse\n  value = other\nend", // first `value` is a call, RHS is a local
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_offense(
            "value = work\nif value\n  value = value\nelse\n  value = other\nend",
            2,
            1,
            6,
            PIPES,
        );
    }

    #[test]
    fn matches_single_argument_methods_and_operators_with_equal_receivers() {
        for (yes, no) in [
            ("test.bar = foo", "test.bar = 'baz'"),
            ("@value - foo", "@value - 'bar'"),
            ("@value.-(foo)", "@value.-('bar')"),
            ("bar foo", "bar 1..2"),
            ("bar(foo)", "bar({})"),
            ("X.find(foo)", "X.find(y)"),
            ("foo(foo).bar(foo)", "foo(foo).bar(quux)"),
            ("bar foo", "bar baz => quux"),
        ] {
            assert_offense(
                &format!("if foo\n  {yes}\nelse\n  {no}\nend"),
                1,
                1,
                5,
                PIPES,
            );
        }
        for source in [
            "foo ? bar(foo) : bar(quux)",
            "foo ? foo(foo).bar(foo) : foo(foo).bar(quux)",
            "foo ? X.find(foo) : X.find(y)",
        ] {
            assert_offense(source, 1, 1, 1, PIPES);
        }
        for op in ["<<", "==", "<", "=~"] {
            assert_offense(
                &format!("if foo\n  stack {op} foo\nelse\n  stack {op} :new\nend"),
                1,
                1,
                5,
                PIPES,
            );
        }
        for fallback in [
            "bar(*baz)",
            "bar(**baz)",
            "bar(&baz)",
            "bar()",
            "bar(1, 2)",
            "bar(foo) { work }",
            "other(foo)",
            "obj.bar(foo)",
            "bar&.call(foo)",
        ] {
            let source = format!("if foo\n  bar(foo)\nelse\n  {fallback}\nend");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for source in [
            "if foo\n  bar[foo]\nelse\n  bar[1]\nend",
            "if foo\n  X.find(foo)\nelse\n  Y.find(y)\nend",
            "if foo\n  obj&.bar(foo)\nelse\n  obj&.bar(other)\nend",
            "def work(foo, *); if foo; bar(foo); else; bar(*); end; end",
            "def work(foo, **); if foo; bar(foo); else; bar(**); end; end",
            "def work(foo, &); if foo; bar(foo); else; bar(&); end; end",
            "def work(foo, ...); if foo; bar(foo); else; bar(...); end; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn outermost_redundant_ancestor_suppresses_descendants_not_siblings() {
        assert_offense(
            "if foo.bar\n  x = foo.bar\nelse\n  x = if foo.bar\n    foo.bar\n  else\n    baz\n  end\nend",
            1,
            1,
            9,
            PIPES,
        );
        assert_offense(
            "if foo\n  foo\nelse\n  consume { bar ? bar : baz }\nend",
            1,
            1,
            5,
            PIPES,
        );
        assert_offense("if guard\n  if foo\n    foo\n  end\nend", 2, 3, 4, UNNEEDED);
        assert_offense(
            "if a\n  b\nelsif c\n  if d\n    d\n  end\nend",
            4,
            3,
            6,
            UNNEEDED,
        );
        let found = offenses("foo ? foo : bar\nbaz ? baz : quux");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 5), (2, 5)]
        );
    }

    #[test]
    fn comments_fallback_precedence_and_multiline_typed_nodes_keep_detection() {
        for fallback in [
            "raise 'foo'",
            "do_something foo, bar, key: :value",
            "c + d",
            "(c || d)",
            "('foo' if $VERBOSE)",
            "(('foo' if $VERBOSE))",
            "c while d",
            "1..2",
            "b rescue c",
            "b and c",
            "y(x,\n    z)",
            "begin; work; rescue; recover; ensure; cleanup; end",
            "begin\n    work\n    recover\n  end",
            "begin\n    work\n  rescue\n    recover\n  end",
            "begin\n    work\n  ensure\n    cleanup\n  end",
            "begin\n    work\n  rescue\n    recover\n  ensure\n    cleanup\n  end",
        ] {
            let source =
                format!("if b\n  # before\n  b # after\nelse\n  # fallback\n  {fallback}\nend");
            assert_offense(&source, 1, 1, source.lines().count() as u32, PIPES);
        }
        for source in [
            "if foo\n  work\n  foo\nelse\n  other\nend",
            "if foo\n  foo\nelse\n  (\n    other\n  )\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn valid_control_flow_branch_values_and_begin_rescue_ensure_contexts() {
        for keyword in ["return 1", "yield 1", "yield(1)", "super 1", "not c"] {
            assert_offense(
                &format!("def example\n  x = if b then b else {keyword} end\nend"),
                2,
                7,
                2,
                PIPES,
            );
        }
        for keyword in ["next 1", "break 1"] {
            assert_offense(
                &format!("loop do\n  x = if b then b else {keyword} end\nend"),
                2,
                7,
                2,
                PIPES,
            );
        }
        assert_offense(
            "begin\n  if foo\n    foo\n  else\n    other\n  end\nrescue\n  recover\nensure\n  cleanup\nend",
            2,
            3,
            6,
            PIPES,
        );
        for source in [
            "value = work; value if value",
            "value = work; value unless value",
            "value = work if value",
            "value = work unless value",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_offense(
            "def example(value)\n  return value ? value : fallback\nend",
            2,
            16,
            2,
            PIPES,
        );
    }

    #[test]
    fn structural_matching_distinguishes_scope_blocks_and_line_values() {
        assert_offense("if foo(1)\n  foo(0x1)\nelse\n  other\nend", 1, 1, 5, PIPES);
        assert_offense("if A::B\n  A::B\nelse\n  other\nend", 1, 1, 5, PIPES);
        for source in [
            "if A::B\n  B\nelse\n  other\nend",
            "if foo\n  self.foo\nelse\n  other\nend",
            "if foo\n  foo { work }\nelse\n  other\nend",
            "if __LINE__\n  __LINE__\nelse\n  other\nend",
            "if foo\n  (foo)\nelse\n  other\nend",
            "if foo\n  return foo\nelse\n  other\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // Like pinned RuboCop, constant assignments compare short names.
        assert_offense(
            "if foo\n  A::VALUE = foo\nelse\n  B::VALUE = bar\nend",
            1,
            1,
            5,
            PIPES,
        );
    }

    #[test]
    fn shorthand_keyword_values_match_explicit_values_in_all_comparison_paths() {
        for (condition, repeated) in [("foo(x:)", "foo(x: x)"), ("foo(x: x)", "foo(x:)")] {
            for (predicate, truthy, falsy) in [
                (condition, repeated.to_string(), "other".to_string()),
                (
                    condition,
                    format!("@v = {repeated}"),
                    "@v = other".to_string(),
                ),
                (
                    condition,
                    format!("bar({repeated})"),
                    "bar(other)".to_string(),
                ),
                (
                    "guard",
                    format!("{condition}.bar(guard)"),
                    format!("{repeated}.bar(other)"),
                ),
            ] {
                assert_offense(
                    &format!("x = 1\nif {predicate}\n  {truthy}\nelse\n  {falsy}\nend"),
                    2,
                    1,
                    6,
                    PIPES,
                );
            }
        }
        for source in [
            "x = 1; y = 2\nif foo(x:)\n  foo(x: y)\nelse\n  other\nend",
            "if foo(x:)\n  x = 1\n  foo(x: x)\nelse\n  other\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn cached_matches_remain_correct_when_dispatch_reuses_the_source_file() {
        let source = "foo ? foo : bar\n";
        let result = ruby_prism::parse(source.as_bytes());
        let file = crate::source::SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        for _ in 0..2 {
            let mut found = Vec::new();
            assert!(
                crate::cops::run_cops(&[&super::RedundantCondition], &result, &file, &mut found)
                    .is_empty()
            );
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), (1, 5));
        }
    }

    #[test]
    fn matching_work_is_bounded_and_does_not_rescan_wide_ancestors() {
        let condition = format!("foo({})", vec!["arg"; 128].join(", "));
        for count in [1, 128] {
            let descendants = "bar ? bar : baz\n".repeat(count);
            let source =
                format!("if {condition}\n{condition}\nelse\nconsume {{\n{descendants}}}\nend");
            super::MATCHED_SOURCE_BYTES.with(|bytes| bytes.set(0));
            assert_offense(&source, 1, 1, (count + 6) as u32, PIPES);
            super::MATCHED_SOURCE_BYTES.with(|bytes| assert_eq!(bytes.get(), source.len()));
        }
        let source = "foo ? foo : bar\n".repeat(128);
        super::MATCHED_SOURCE_BYTES.with(|bytes| bytes.set(0));
        assert_eq!(offenses(&source).len(), 128);
        super::MATCHED_SOURCE_BYTES.with(|bytes| assert!(bytes.get() <= source.len()));
    }

    #[test]
    fn different_conditions_modifiers_and_elsif_are_not_redundant() {
        for source in [
            "if a\n  b\nelse\n  c\nend",
            "b ? d : c",
            "bar if bar",
            "bar unless bar",
            "if a\n  a\nelsif cond\n  d\nend",
            "if a\n  b\nelsif d\n  d\nelse\n  c\nend",
            "if a\n  a\nelse\n  b ? c : d\nend",
            "unless a\n  b\nelse\n  c\nend",
            "if b\n  b\nelse\n  c\n  d\nend",
            "unless b\n  c\n  d\nelse\n  b\nend",
            "if b\n  work\n  b\nelse\n  c\nend",
            "if @cache[key]\n  @cache[key]\nelse\n  @cache[key] = heavy_load[key]\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
