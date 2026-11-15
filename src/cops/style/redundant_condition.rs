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

#[cfg(test)]
thread_local! {
    static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Equality does not imply repeatability: a second send can mutate state, raise,
// or return something else. In particular a custom `?` need not return Boolean.
fn stable_read(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::LocalVariableReadNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::SelfNode { .. }
            | Node::TrueNode { .. }
            | Node::FalseNode { .. }
            | Node::NilNode { .. }
    )
}

fn text<'pr>(node: &Node<'pr>) -> Option<&'pr str> {
    std::str::from_utf8(node.location().as_slice()).ok()
}

// Parentheses make fallback control flow, rescue, low-precedence operators and
// command calls unambiguous without reparsing or moving their effects.
fn fallback_source(node: &Node<'_>) -> Option<String> {
    let source = text(node)?;
    let unwrapped = stable_read(node)
        || matches!(
            node,
            Node::ParenthesesNode { .. }
                | Node::BeginNode { .. }
                | Node::IntegerNode { .. }
                | Node::FloatNode { .. }
                | Node::StringNode { .. }
                | Node::SymbolNode { .. }
                | Node::ArrayNode { .. }
                | Node::HashNode { .. }
                | Node::ConstantReadNode { .. }
                | Node::ConstantPathNode { .. }
        )
        || node.as_call_node().is_some_and(|n| {
            !n.message_loc().is_some_and(|l| l.as_slice() == b"not")
                && n.block().is_none()
                && (n.opening_loc().is_some() || n.arguments().is_none())
        });
    Some(
        if node
            .as_array_node()
            .is_some_and(|n| n.opening_loc().is_none())
        {
            format!("[{source}]")
        } else if node.as_keyword_hash_node().is_some() {
            format!("{{ {source} }}")
        } else if unwrapped {
            source.to_owned()
        } else {
            format!("({source})")
        },
    )
}

// Ruby reparses regexp/range values as implicit matches/flip-flops in
// predicates, even through parentheses, begin and boolean operators. Scan only
// the bounded fallback; conservatively include literals nested in value wrappers.
fn condition_sensitive_value(node: &Node<'_>) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    struct Sensitive(bool);
    impl<'pr> NodeVisitor<'pr> for Sensitive {
        fn enter(&mut self, node: Node<'pr>) {
            self.0 |= matches!(
                node,
                Node::RegularExpressionNode { .. }
                    | Node::InterpolatedRegularExpressionNode { .. }
                    | Node::RangeNode { .. }
            );
        }
    }
    let mut visitor = Sensitive(false);
    walk(&mut visitor, node);
    visitor.0
}

fn in_predicate(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let within = |predicate: Node<'_>| {
        predicate.location().start_offset() <= node.location().start_offset()
            && node.location().end_offset() <= predicate.location().end_offset()
    };
    cx.ancestors().any(|ancestor| {
        let predicate = if let Some(n) = ancestor.as_if_node() {
            Some(n.predicate())
        } else if let Some(n) = ancestor.as_unless_node() {
            Some(n.predicate())
        } else if let Some(n) = ancestor.as_while_node() {
            Some(n.predicate())
        } else {
            ancestor.as_until_node().map(|n| n.predicate())
        };
        predicate.is_some_and(within)
            || ancestor
                .as_when_node()
                .is_some_and(|n| n.conditions().iter().any(within))
            // Both `!` and keyword `not` have the selector `!` in Prism.
            || ancestor
                .as_call_node()
                .is_some_and(|n| n.name().as_slice() == b"!")
    })
}

fn correction(
    node: &Node<'_>,
    branches: &Branches<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = node.location();
    // Bounded correction-only source reconstruction. Detection remains uncapped.
    if loc.as_slice().len() > 32 * 1024
        || !stable_read(&branches.condition)
        || cx.ancestors().any(|n| n.as_defined_node().is_some())
    {
        return None;
    }
    std::str::from_utf8(loc.as_slice()).ok()?;
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + loc.as_slice().len()));
    let root = cx.ancestors().next_back()?;
    if super::correction::contains_comment(file, loc.start_offset()..loc.end_offset())
        || super::correction::touches_directive(file, loc.start_offset()..loc.end_offset())
        || file.contains_heredoc(root, node)
        || file.heredoc_pending_at(root, loc.start_offset())
        || file.class_method_bindings(root).observes_sources
    {
        return None;
    }
    let truthy = branches.truthy.as_ref()?;
    let guard = text(&branches.condition)?;
    let mut replacement = if ast::same_node(&branches.condition, truthy) {
        match &branches.falsy {
            Some(falsy) => {
                if in_predicate(node, cx) && condition_sensitive_value(falsy) {
                    return None;
                }
                format!("{guard} || {}", fallback_source(falsy)?)
            }
            // A bare replacement would return false, whereas an if without an
            // else returns nil on that path. Retain that observable distinction.
            None => format!("{guard} || nil"),
        }
    } else if let (Some((_, yes)), Some((_, no))) = (
        assignment(truthy),
        branches.falsy.as_ref().and_then(assignment),
    ) {
        if !ast::same_node(&branches.condition, &yes) {
            return None;
        }
        let prefix = |branch: &Node<'_>, value: &Node<'_>| {
            std::str::from_utf8(
                &file.bytes[branch.location().start_offset()..value.location().start_offset()],
            )
            .ok()
        };
        let yes_prefix = prefix(truthy, &yes)?;
        let no_prefix = prefix(branches.falsy.as_ref()?, &no)?;
        // Do not move a namespace lookup (or conflate same short constant names).
        if yes_prefix != no_prefix || truthy.as_constant_path_write_node().is_some() {
            return None;
        }
        format!("{yes_prefix}{guard} || {}", fallback_source(&no)?)
    } else if branches.same_methods() {
        let (yes, arg) = method_argument(truthy)?;
        let (_, no) = method_argument(branches.falsy.as_ref()?)?;
        if !ast::same_node(&branches.condition, &arg)
            || yes.receiver().is_some_and(|r| !stable_read(&r))
            // Keyword-to-positional hash conversion is not a safe Ruby 3 rewrite.
            || no.as_keyword_hash_node().is_some()
        {
            return None;
        }
        let source = text(truthy)?;
        let start = arg.location().start_offset() - truthy.location().start_offset();
        let end = arg.location().end_offset() - truthy.location().start_offset();
        let inner = format!("{guard} || {}", fallback_source(&no)?);
        let binary = yes.opening_loc().is_none()
            && yes.call_operator_loc().is_none()
            && yes.receiver().is_some()
            && yes.equal_loc().is_none();
        format!(
            "{}{}{}",
            &source[..start],
            if binary { format!("({inner})") } else { inner },
            &source[end..]
        )
    } else {
        return None;
    };
    // Replacing a conditional with || (or an extracted assignment) must not
    // capture a surrounding operator/receiver, modifier, range or condition.
    let operator_argument = cx.parent().is_some_and(|p| p.as_arguments_node().is_some())
        && cx
            .ancestors()
            .nth(1)
            .and_then(|p| p.as_call_node())
            .is_some_and(|c| {
                c.receiver().is_some()
                    && c.opening_loc().is_none()
                    && c.call_operator_loc().is_none()
                    && c.equal_loc().is_none()
            });
    // An extracted command call can swallow the next argument/array element,
    // or steal an enclosing block. Group the entire call, not only its argument.
    let command_argument = truthy
        .as_call_node()
        .is_some_and(|n| branches.same_methods() && n.opening_loc().is_none())
        && cx
            .parent()
            .is_some_and(|p| matches!(p, Node::ArgumentsNode { .. } | Node::ArrayNode { .. }));
    if operator_argument
        || command_argument
        || !cx.parent().is_some_and(|p| {
            matches!(
                p,
                Node::StatementsNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::ArgumentsNode { .. }
                    | Node::ArrayNode { .. }
                    | Node::LocalVariableWriteNode { .. }
                    | Node::InstanceVariableWriteNode { .. }
                    | Node::ClassVariableWriteNode { .. }
                    | Node::GlobalVariableWriteNode { .. }
                    | Node::ConstantWriteNode { .. }
            )
        })
    {
        replacement = format!("({replacement})");
    }
    let edit = Correction {
        range: loc.start_offset()..loc.end_offset(),
        replacement,
    };
    (super::correction::preserves_line_values(file, &edit)
        && super::correction::preserves_data_positions(file, cx, &edit))
    .then_some(edit)
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

    #[test]
    fn stable_reads_have_real_corrections_and_preserve_branch_results() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            ("@b ? @b : c\n", "@b || c\n"),
            ("if @b\n  @b\nelse\n  c\nend\n", "@b || c\n"),
            ("b = false; if b then b else c end\n", "b = false; b || c\n"),
            ("unless @b\n  c\nelse\n  @b\nend\n", "@b || c\n"),
            ("if @b then @b end\n", "@b || nil\n"),
            ("if @b then @v = @b else @v = c end\n", "@v = @b || c\n"),
            (
                "if @b then consume(@b) else consume(c) end\n",
                "consume(@b || c)\n",
            ),
            (
                "if @b then self.value = @b else self.value = c end\n",
                "self.value = @b || c\n",
            ),
        ] {
            assert_correction(&super::RedundantCondition, before, after);
        }
    }

    #[test]
    fn all_pinned_calls_and_corrections_are_accounted_for() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_condition_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 115);
        let mut counts = (0, 0, 0);
        for case in cases {
            let source = case["source"].as_str().unwrap();
            let invalid = source.contains("else next 1")
                || source.contains("else break 1")
                || source.contains("else yield 1")
                || source.contains("else yield(1)");
            if invalid {
                assert!(ruby_prism::parse(source.as_bytes()).errors().count() > 0);
                counts.2 += 1;
                continue;
            }
            let found = offenses(source);
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                case["positions"],
                "{case}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                case["messages"],
                "{case}"
            );
            // Only the instance-variable example repeats a provably stable
            // read. All other correction examples repeat sends, or infer a
            // Boolean from a custom predicate name. Neither is a safety proof.
            let expected = if source == "@b ? @b : c\n" {
                counts.1 += 1;
                case["correction"].as_str().unwrap()
            } else {
                source
            };
            if case["correction"].is_string() {
                counts.0 += 1;
            }
            if found.is_empty() {
                assert!(
                    crate::cops::testing::correction_offenses(&super::RedundantCondition, source)
                        .is_empty()
                );
            } else {
                crate::cops::testing::assert_correction(
                    &super::RedundantCondition,
                    source,
                    expected,
                );
            }
        }
        assert_eq!(counts, (63, 1, 4)); // 67 upstream correction calls, four invalid.
    }

    #[test]
    fn effects_nonboolean_predicates_namespaces_and_dynamic_receivers_are_withheld() {
        use crate::cops::testing::assert_correction;
        for source in [
            "if value then value else fallback end\n",
            "if foo? then true else fallback end\n",
            "foo? ? true : fallback\n",
            "if value then value end\n",
            "if @b then A::V = @b else B::V = c end\n",
            "if @b then A::V = @b else A::V = c end\n",
            "if @b then receiver.consume(@b) else receiver.consume(c) end\n",
            "if @b then consume(@b) else consume(key: c) end\n",
            "defined?(@b ? @b : c)\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
            assert_correction(&super::RedundantCondition, source, source);
        }
    }

    #[test]
    fn fallback_control_flow_precedence_and_branch_effects_remain_grouped() {
        use crate::cops::testing::assert_correction;
        for (fallback, expected) in [
            ("raise 'x'", "(raise 'x')"),
            ("consume x, y, key: :value", "(consume x, y, key: :value)"),
            ("c + d", "(c + d)"),
            ("(c || d)", "(c || d)"),
            ("('x' if $VERBOSE)", "('x' if $VERBOSE)"),
            ("c while d", "(c while d)"),
            ("1..2", "(1..2)"),
            ("b rescue c", "(b rescue c)"),
            ("b and c", "(b and c)"),
            ("not c", "(not c)"),
            ("(c = work)", "(c = work)"),
            ("work(1) do\n  effect\nend", "(work(1) do\n  effect\nend)"),
            (
                "begin; work; rescue; recover; ensure; cleanup; end",
                "begin; work; rescue; recover; ensure; cleanup; end",
            ),
        ] {
            assert_correction(
                &super::RedundantCondition,
                &format!("if @b\n  @b\nelse\n  {fallback}\nend\n"),
                &format!("@b || {expected}\n"),
            );
        }
        for (keyword, expected) in [
            ("return 1", "(return 1)"),
            ("yield 1", "(yield 1)"),
            ("yield(1)", "(yield(1))"),
            ("super 1", "(super 1)"),
        ] {
            assert_correction(
                &super::RedundantCondition,
                &format!("def example\n  x = if @b then @b else {keyword} end\nend\n"),
                &format!("def example\n  x = @b || {expected}\nend\n"),
            );
        }
        for keyword in ["next 1", "break 1"] {
            assert_correction(
                &super::RedundantCondition,
                &format!("loop do\n  x = if @b then @b else {keyword} end\nend\n"),
                &format!("loop do\n  x = @b || ({keyword})\nend\n"),
            );
        }
    }

    #[test]
    fn method_assignment_operator_and_enclosing_expression_grouping() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "if @b then @stack << @b else @stack << :new end\n",
                "@stack << (@b || :new)\n",
            ),
            (
                "if @b then @stack.-(@b) else @stack.-(1..2) end\n",
                "@stack.-(@b || (1..2))\n",
            ),
            ("if @b then bar @b else bar({}) end\n", "bar @b || {}\n"),
            (
                "if @b then @v = @b else @v = 1, 2 end\n",
                "@v = @b || [1, 2]\n",
            ),
            ("ary << if @b then @b else c end\n", "ary << (@b || c)\n"),
            ("(if @b then @b else c end).work\n", "(@b || c).work\n"),
            (
                "work if (if @b then @b else c end)\n",
                "work if (@b || c)\n",
            ),
            (
                "if guard\n  if @b then @b else c end\nelsif other\n  work\nend\n",
                "if guard\n  @b || c\nelsif other\n  work\nend\n",
            ),
        ] {
            assert_correction(&super::RedundantCondition, before, after);
        }
    }

    #[test]
    fn regexp_and_range_fallbacks_stay_correctable_in_value_contexts() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "value = if @b then @b else /yes/ end\n",
                "value = @b || (/yes/)\n",
            ),
            (
                "if ready\n  if @b then @b else 1..2 end\nend\n",
                "if ready\n  @b || (1..2)\nend\n",
            ),
            (
                "if (if @b then @v = @b else @v = /yes/ end); work; end\n",
                "if (@v = @b || (/yes/)); work; end\n",
            ),
            (
                "if (if @b then use(@b) else use(1..2) end); work; end\n",
                "if (use(@b || (1..2))); work; end\n",
            ),
        ] {
            assert_correction(&super::RedundantCondition, before, after);
        }
    }

    #[test]
    fn correction_source_work_is_lazy_and_bounded() {
        use crate::cops::testing::correction_offenses;
        let source = "@b ? @b : c\n".repeat(256);
        super::REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(offenses(&source).len(), 256);
        super::REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        assert_eq!(
            correction_offenses(&super::RedundantCondition, &source)
                .iter()
                .filter(|o| o.correction.is_some())
                .count(),
            256
        );
        super::REWRITE_BYTES.with(|n| assert!(n.get() <= source.len()));
        let source = format!(
            "if @b then @b else consume('{}') end\n",
            "x".repeat(33 * 1024)
        );
        super::REWRITE_BYTES.with(|n| n.set(0));
        assert_correction_hold(&source);
        super::REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
    }

    fn assert_correction_hold(source: &str) {
        assert_eq!(offenses(source).len(), 1);
        crate::cops::testing::assert_correction(&super::RedundantCondition, source, source);
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
