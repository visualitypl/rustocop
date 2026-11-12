//! `Style/DoubleNegation` — fixed `allowed_in_returns`, unsafe detection-only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct DoubleNegation;
impl Cop for DoubleNegation {
    fn name(&self) -> &'static str {
        "Style/DoubleNegation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(outer) = argumentless_negation(node) else {
            return;
        };
        let Some(inner_node) = outer.receiver() else {
            return;
        };
        // A literal block wraps its send upstream: it may wrap the outer
        // send, but cannot stand in for the inner send of the matcher.
        let Some(inner) = argumentless_negation(&inner_node).filter(|n| n.block().is_none()) else {
            return;
        };
        let Some(selector) = outer.message_loc().filter(|loc| loc.as_slice() == b"!") else {
            return;
        };
        if (outer.block().is_none()
            && cx
                .parent()
                .and_then(negation)
                .and_then(|n| n.receiver())
                .is_some_and(|n| n.location().start_offset() == node.location().start_offset()))
            || inner
                .receiver()
                .as_ref()
                .and_then(negation)
                .is_some_and(|n| n.block().is_none_or(|b| b.as_block_node().is_none()))
        {
            return;
        }
        if allowed_in_returns(node, cx, file) {
            return;
        }
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Avoid the use of double negation (`!!`).".into(),
            correction: None,
        });
    }
}

fn negation<'pr>(node: &Node<'pr>) -> Option<ruby_prism::CallNode<'pr>> {
    node.as_call_node()
        .filter(|n| n.name().as_slice() == b"!" && !n.is_safe_navigation())
}
fn argumentless_negation<'pr>(node: &Node<'pr>) -> Option<ruby_prism::CallNode<'pr>> {
    negation(node).filter(|n| {
        n.arguments().is_none_or(|a| a.arguments().is_empty())
            // Literal blocks are separate nodes upstream; &block is an
            // argument, so it prevents the argumentless send match.
            && n.block().is_none_or(|b| b.as_block_node().is_some())
    })
}

// Prism's argument/else wrappers and singleton statement lists have no
// corresponding parent in RuboCop. Explicit grouping remains a :begin.
fn semantic_parent(node: &Node<'_>) -> bool {
    !matches!(node, Node::ArgumentsNode { .. } | Node::ElseNode { .. })
        && !node
            .as_statements_node()
            .is_some_and(|n| n.body().len() == 1)
}
fn body_root(node: Node<'_>) -> Node<'_> {
    if let Some(n) = node.as_statements_node().filter(|n| n.body().len() == 1) {
        n.body().first().unwrap()
    } else {
        node
    }
}
fn conditional(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::IfNode { .. }
            | Node::UnlessNode { .. }
            | Node::CaseNode { .. }
            | Node::CaseMatchNode { .. }
    ) || node.as_while_node().is_some_and(|n| !n.is_begin_modifier())
        || node.as_until_node().is_some_and(|n| !n.is_begin_modifier())
}
fn enumerable(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::ArrayNode { .. }
            | Node::HashNode { .. }
            | Node::KeywordHashNode { .. }
            | Node::AssocNode { .. }
    )
}
fn allowed_in_returns(node: &Node<'_>, cx: &NodeContext<'_, '_>, file: &SourceFile) -> bool {
    // Prism folds this upstream parent block into the candidate's CallNode.
    let has_block_parent = node
        .as_call_node()
        .is_some_and(|n| n.block().is_some_and(|b| b.as_block_node().is_some()));
    if !has_block_parent
        && cx
            .ancestors()
            .find(|n| semantic_parent(n))
            .is_some_and(|n| n.as_return_node().is_some())
    {
        return true;
    }
    let mut last = None;
    for ancestor in cx.ancestors() {
        if let Some(n) = ancestor.as_def_node() {
            last = n.body().and_then(last_child);
            break;
        }
        if ancestor.as_call_node().is_some_and(|c| {
            !c.is_safe_navigation()
                && c.block().is_some_and(|b| b.as_block_node().is_some())
                && matches!(
                    c.name().as_slice(),
                    b"define_method" | b"define_singleton_method"
                )
        }) {
            // RuboCop finds the definition's send inside its block wrapper,
            // including from receiver/argument descendants. Its last child
            // is the method-name argument, not the body.
            let c = ancestor.as_call_node().unwrap();
            last = c
                .arguments()
                .and_then(|a| a.arguments().last())
                .or_else(|| c.receiver())
                .map(|n| (n, TailKind::Normal));
            break;
        }
    }
    let Some((last, kind)) = last else {
        return false;
    };
    if let Some(condition) = cx.ancestors().find(|n| conditional(n)) {
        let parent = cx
            .ancestors()
            .find(|n| semantic_parent(n) && !enumerable(n));
        if !has_block_parent
            && parent.is_some_and(|n| {
                n.as_statements_node().is_some() || n.as_parentheses_node().is_some()
            })
        {
            return file.line_of(node.location().start_offset())
                == file.last_line_of(parent.unwrap().location());
        }
        let last_line = match kind {
            TailKind::ExceptionBody { end, .. } => file.line_of(end.saturating_sub(1)),
            _ => file.last_line_of(last.location()),
        };
        last_line <= file.last_line_of(condition.location())
    } else {
        let first_line = match kind {
            TailKind::ExceptionBody { start, .. } => file.line_of(start),
            // RuboCop splits static interpolation parts at physical newlines;
            // the last part begins on its last nonempty physical line. No
            // candidate-local scan of a potentially huge literal is needed.
            TailKind::StringPart if last.as_string_node().is_some() => {
                file.line_of(last.location().end_offset().saturating_sub(1))
            }
            TailKind::RegexpOptions => file.line_of(last.location().end_offset().saturating_sub(1)),
            _ => file.line_of(last.location().start_offset()),
        };
        !matches!(kind, TailKind::ArrayChild)
            && !matches!(
                last,
                Node::AssocNode { .. } | Node::HashNode { .. } | Node::KeywordHashNode { .. }
            )
            && first_line <= file.line_of(node.location().start_offset())
    }
}

#[derive(Clone, Copy)]
enum TailKind {
    Normal,
    ArrayChild,
    StringPart,
    RegexpOptions,
    ExceptionBody { start: usize, end: usize },
}

// Return the pinned cop's immediate last child, not a recursive value-flow
// analysis. In particular arrays/hashes at a sole method-body root differ
// from collections inside a final conditional. List::last is O(1).
fn last_child(node: Node<'_>) -> Option<(Node<'_>, TailKind)> {
    let node = body_root(node);
    if let Some(n) = node.as_begin_node() {
        // Implicit method-body rescue/ensure wrappers are unwrapped through
        // the main body, not through rescue/else/ensure handler values.
        if n.begin_keyword_loc().is_none() {
            return n.statements().and_then(|s| last_child(s.as_node()));
        }
        // A keyword begin owns a synthetic rescue/ensure child. Unlike an
        // implicit method-body wrapper it is not recursively unwrapped.
        if n.rescue_clause().is_some() || n.ensure_clause().is_some() {
            let start = n
                .statements()
                .map_or(n.location().start_offset(), |s| s.location().start_offset());
            let end = if let Some(e) = n.ensure_clause() {
                e.statements()
                    .map_or(e.ensure_keyword_loc().end_offset(), |s| {
                        s.location().end_offset()
                    })
            } else if let Some(e) = n.else_clause() {
                e.statements()
                    .map_or(e.else_keyword_loc().end_offset(), |s| {
                        s.location().end_offset()
                    })
            } else {
                n.rescue_clause().unwrap().location().end_offset()
            };
            return Some((node, TailKind::ExceptionBody { start, end }));
        }
        return n
            .statements()
            .and_then(|s| s.body().last())
            .map(|n| (n, TailKind::Normal));
    }
    let child = match &node {
        Node::StatementsNode { .. } => node.as_statements_node().unwrap().body().last(),
        Node::ArrayNode { .. } => {
            return node
                .as_array_node()
                .unwrap()
                .elements()
                .last()
                .map(|n| (n, TailKind::ArrayChild));
        }
        Node::HashNode { .. } => node.as_hash_node().unwrap().elements().last(),
        Node::KeywordHashNode { .. } => node.as_keyword_hash_node().unwrap().elements().last(),
        Node::ParenthesesNode { .. } => node.as_parentheses_node().unwrap().body().and_then(|n| {
            n.as_statements_node()
                .and_then(|s| s.body().last())
                .or(Some(n))
        }),
        Node::CallNode { .. } => {
            let c = node.as_call_node().unwrap();
            if let Some(b) = c.block().and_then(|b| b.as_block_node()) {
                b.body().map(body_root).or_else(|| b.parameters())
            } else {
                c.block()
                    .or_else(|| c.arguments().and_then(|a| a.arguments().last()))
                    .or_else(|| c.receiver())
            }
        }
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            n.subsequent()
                .map(|s| {
                    s.as_else_node()
                        .and_then(|e| e.statements())
                        .map(|s| body_root(s.as_node()))
                        .unwrap_or(s)
                })
                .or_else(|| n.statements().map(|s| body_root(s.as_node())))
                .or_else(|| Some(n.predicate()))
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node().unwrap();
            n.statements()
                .map(|s| body_root(s.as_node()))
                .or_else(|| {
                    n.else_clause()
                        .and_then(|e| e.statements())
                        .map(|s| body_root(s.as_node()))
                })
                .or_else(|| Some(n.predicate()))
        }
        Node::CaseNode { .. } => {
            let n = node.as_case_node().unwrap();
            n.else_clause()
                .and_then(|e| e.statements())
                .map(|s| body_root(s.as_node()))
                .or_else(|| n.conditions().last())
                .or_else(|| n.predicate())
        }
        Node::CaseMatchNode { .. } => {
            let n = node.as_case_match_node().unwrap();
            n.else_clause()
                .and_then(|e| e.statements())
                .map(|s| body_root(s.as_node()))
                .or_else(|| n.conditions().last())
                .or_else(|| n.predicate())
        }
        Node::InterpolatedStringNode { .. } => {
            return node
                .as_interpolated_string_node()
                .unwrap()
                .parts()
                .last()
                .map(|n| (n, TailKind::StringPart));
        }
        Node::InterpolatedSymbolNode { .. } => {
            return node
                .as_interpolated_symbol_node()
                .unwrap()
                .parts()
                .last()
                .map(|n| (n, TailKind::StringPart));
        }
        Node::InterpolatedXStringNode { .. } => {
            return node
                .as_interpolated_x_string_node()
                .unwrap()
                .parts()
                .last()
                .map(|n| (n, TailKind::StringPart));
        }
        Node::InterpolatedRegularExpressionNode { .. }
        | Node::InterpolatedMatchLastLineNode { .. } => {
            return Some((node, TailKind::RegexpOptions));
        }

        Node::LambdaNode { .. } => {
            let n = node.as_lambda_node().unwrap();
            n.body().map(body_root).or_else(|| n.parameters())
        }
        Node::ForNode { .. } => {
            let n = node.as_for_node().unwrap();
            n.statements()
                .map(|s| body_root(s.as_node()))
                .or_else(|| Some(n.collection()))
        }
        Node::RescueModifierNode { .. } => {
            return last_child(node.as_rescue_modifier_node().unwrap().expression());
        }
        Node::MatchPredicateNode { .. } => Some(node.as_match_predicate_node().unwrap().pattern()),
        Node::MatchRequiredNode { .. } => Some(node.as_match_required_node().unwrap().pattern()),
        Node::UndefNode { .. } => node.as_undef_node().unwrap().names().last(),
        Node::RangeNode { .. } => {
            let n = node.as_range_node().unwrap();
            n.right().or_else(|| n.left())
        }
        Node::YieldNode { .. } => node
            .as_yield_node()
            .unwrap()
            .arguments()
            .and_then(|a| a.arguments().last()),
        Node::SuperNode { .. } => {
            let n = node.as_super_node().unwrap();
            if let Some(b) = n.block().and_then(|b| b.as_block_node()) {
                b.body().map(body_root).or_else(|| b.parameters())
            } else {
                n.block()
                    .or_else(|| n.arguments().and_then(|a| a.arguments().last()))
            }
        }
        Node::AndNode { .. } => Some(node.as_and_node().unwrap().right()),
        Node::OrNode { .. } => Some(node.as_or_node().unwrap().right()),
        Node::LocalVariableWriteNode { .. } => {
            Some(node.as_local_variable_write_node().unwrap().value())
        }
        Node::InstanceVariableWriteNode { .. } => {
            Some(node.as_instance_variable_write_node().unwrap().value())
        }
        Node::ClassVariableWriteNode { .. } => {
            Some(node.as_class_variable_write_node().unwrap().value())
        }
        Node::GlobalVariableWriteNode { .. } => {
            Some(node.as_global_variable_write_node().unwrap().value())
        }
        Node::LocalVariableAndWriteNode { .. } => {
            Some(node.as_local_variable_and_write_node().unwrap().value())
        }
        Node::LocalVariableOrWriteNode { .. } => {
            Some(node.as_local_variable_or_write_node().unwrap().value())
        }
        Node::LocalVariableOperatorWriteNode { .. } => Some(
            node.as_local_variable_operator_write_node()
                .unwrap()
                .value(),
        ),
        Node::InstanceVariableAndWriteNode { .. } => {
            Some(node.as_instance_variable_and_write_node().unwrap().value())
        }
        Node::InstanceVariableOrWriteNode { .. } => {
            Some(node.as_instance_variable_or_write_node().unwrap().value())
        }
        Node::InstanceVariableOperatorWriteNode { .. } => Some(
            node.as_instance_variable_operator_write_node()
                .unwrap()
                .value(),
        ),
        Node::ClassVariableAndWriteNode { .. } => {
            Some(node.as_class_variable_and_write_node().unwrap().value())
        }
        Node::ClassVariableOrWriteNode { .. } => {
            Some(node.as_class_variable_or_write_node().unwrap().value())
        }
        Node::ClassVariableOperatorWriteNode { .. } => Some(
            node.as_class_variable_operator_write_node()
                .unwrap()
                .value(),
        ),
        Node::GlobalVariableAndWriteNode { .. } => {
            Some(node.as_global_variable_and_write_node().unwrap().value())
        }
        Node::GlobalVariableOrWriteNode { .. } => {
            Some(node.as_global_variable_or_write_node().unwrap().value())
        }
        Node::GlobalVariableOperatorWriteNode { .. } => Some(
            node.as_global_variable_operator_write_node()
                .unwrap()
                .value(),
        ),
        Node::ConstantAndWriteNode { .. } => {
            Some(node.as_constant_and_write_node().unwrap().value())
        }
        Node::ConstantOrWriteNode { .. } => Some(node.as_constant_or_write_node().unwrap().value()),
        Node::ConstantOperatorWriteNode { .. } => {
            Some(node.as_constant_operator_write_node().unwrap().value())
        }
        Node::ConstantWriteNode { .. } => Some(node.as_constant_write_node().unwrap().value()),
        Node::ConstantPathWriteNode { .. } => {
            Some(node.as_constant_path_write_node().unwrap().value())
        }
        Node::ConstantPathAndWriteNode { .. } => {
            Some(node.as_constant_path_and_write_node().unwrap().value())
        }
        Node::ConstantPathOrWriteNode { .. } => {
            Some(node.as_constant_path_or_write_node().unwrap().value())
        }
        Node::ConstantPathOperatorWriteNode { .. } => {
            Some(node.as_constant_path_operator_write_node().unwrap().value())
        }
        Node::SingletonClassNode { .. } => {
            let n = node.as_singleton_class_node().unwrap();
            n.body().map(body_root).or_else(|| Some(n.expression()))
        }
        Node::MultiWriteNode { .. } => Some(node.as_multi_write_node().unwrap().value()),
        Node::CallAndWriteNode { .. } => Some(node.as_call_and_write_node().unwrap().value()),
        Node::CallOrWriteNode { .. } => Some(node.as_call_or_write_node().unwrap().value()),
        Node::CallOperatorWriteNode { .. } => {
            Some(node.as_call_operator_write_node().unwrap().value())
        }
        Node::IndexAndWriteNode { .. } => Some(node.as_index_and_write_node().unwrap().value()),
        Node::IndexOrWriteNode { .. } => Some(node.as_index_or_write_node().unwrap().value()),
        Node::IndexOperatorWriteNode { .. } => {
            Some(node.as_index_operator_write_node().unwrap().value())
        }
        Node::ReturnNode { .. } => node
            .as_return_node()
            .unwrap()
            .arguments()
            .and_then(|a| a.arguments().last()),
        _ => Some(node),
    };
    child.map(|n| (n, TailKind::Normal))
}

#[cfg(test)]
mod tests {
    //! Ported/adapted from RuboCop style/double_negation_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/DoubleNegation")
            .collect()
    }
    #[test]
    fn compound_variable_assignments_end_a_method_with_their_value() {
        for (source, col) in [
            ("def a\n  @a ||= { x: !!b }\nend\n", 15),
            ("def a\n  @a &&= { x: !!b }\nend\n", 15),
            ("def a\n  @@a ||= { x: !!b }\nend\n", 16),
            ("def a\n  $a ||= { x: !!b }\nend\n", 15),
            ("def a\n  a ||= { x: !!b }\nend\n", 14),
            ("def a\n  a += { x: !!b }\nend\n", 13),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, col), "{source}");
        }
        for source in [
            "def a\n  @a ||= !!b\nend\n",
            "def a\n  @a ||= [!!b]\nend\n",
            "def a\n  @a += !!b\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn reports_exact_outer_selector_and_never_corrects() {
        for (source, line, col) in [
            ("!!test.something", 1, 1),
            ("foo.!(!!test.something)", 1, 7),
            ("# comment\n  !!false\n", 2, 3),
            ("value.!.!", 1, 9),
            ("!!(not value)", 1, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line),
                "{source}"
            );
            assert_eq!(found[0].message, "Avoid the use of double negation (`!!`).");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn explicit_returns_are_direct_only_and_grouping_is_not_transparent() {
        for source in [
            "def f; return !!x; finish; end",
            "return !!x, !!y",
            "def f; return !!x if c; finish; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "def f\nreturn [!!x]\nfinish\nend",
            "def f\nreturn(!!x)\nfinish\nend",
            "def f\nreturn {a: !!x}\nfinish\nend",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn nontrivial_sole_body_roots_select_their_actual_last_children() {
        for (expr, col) in [
            ("consume(!!x,\n y) rescue z", 9),
            ("!!x in\n [y]", 1),
            ("!!x =>\n [y]", 1),
            ("super(!!x) {\n y\n}", 7),
            ("undef :\"#{!!x}\",\n :foo", 11),
        ] {
            let source = format!("def f\n{expr}\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, col), "{source}");
        }
        for expr in ["->(a = (!!x)) {}", "for x in !!y\nend"] {
            let source = format!("def f\n{expr}\nend");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn compound_assignment_targets_do_not_become_boolean_return_values() {
        for op in ["+=", "||=", "&&="] {
            let source = format!("def f\nobject[!!index] {op}\n value\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 8));
        }
        let found = offenses("def f\nobject[!!index], x =\n values\nend");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 8));
    }
    #[test]
    fn collection_and_short_circuit_returns_follow_pinned_ast_shapes() {
        for source in [
            "def f\nstart\n[!!x]\nend",
            "def f\nstart\n!!x &&\n y\nend",
            "def f\nif c\n[!!x]\nend\nend",
            "def f; !!x; finish; end",
            "def f\nstart\n!!x\nrescue\nhandler\nensure\ncleanup\nend",
            "def f\nbegin\n!!x\nfinish\nrescue\nhandler\nend\nend",
            "def f\nbegin\n!!x\nfinish\nensure\nhandler\nend\nend",
            "def f\nbegin\nstart\nrescue\nif c\n!!x\nend\nend\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for source in [
            "def f\n[!!x]\nend",
            "def f\n{a: !!x}\nend",
            "def f\n!!x &&\n y\nend",
            "def f\n!!x\nfinish\nend",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }
    #[test]
    fn multiline_tail_children_preserve_pinned_line_based_return_policy() {
        for (expression, col) in [
            ("\"#{!!x}\\nending\"", 4),
            ("\"#{!!x}\\n#{y}\"", 4),
            ("/#{!!x}\\nending/", 4),
            ("`#{!!x}\\nending`", 4),
            ("->(a = (!!x)) {\\n y\\n}", 9),
            ("yield(!!x,\\n y)", 7),
            ("super(!!x,\\n y)", 7),
            ("(!!x)..\\ny", 2),
            ("(!!x;\\n y)", 2),
            ("for x in !!y\\n !!z\\nend", 10),
        ] {
            let source = format!("def foo\n{}\nend", expression.replace("\\n", "\n"));
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, col), "{source}");
        }
        let found = offenses("def f\nbegin\n!!x\ny\nend\nend");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 1));
    }
    #[test]
    fn pinned_macro_returns_use_the_definition_call_not_its_body() {
        for source in [
            "define_method(:foo) { [!!x] }",
            "define_method(:foo) do\n!!x &&\n y\nend",
            "obj.define_singleton_method(:foo) { {a: !!x} }",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn post_condition_loops_are_not_upstream_conditionals() {
        for keyword in ["while", "until"] {
            let source = format!("def f\nbegin\n!!x\ny\nend {keyword} c\nend");
            assert!(offenses(&source).is_empty(), "{source}");
            let source = format!("def f\nbegin\ny\n!!x\nend {keyword} c\nfinish\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (4, 1), "{source}");

            // Pre-condition loops retain the conditional exemption instead.
            let source = format!("def f\n{keyword} c\n!!x\ny\nend\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (3, 1), "{source}");
            let source = format!("def f\n{keyword} c\ny\n!!x\nend\nfinish\nend");
            assert!(offenses(&source).is_empty(), "{source}");
        }
    }
    #[test]
    fn literal_blocks_separate_negation_receivers_but_not_their_own_send() {
        for (source, col) in [
            ("value.!.! { y }", 9),
            ("!!value.! { y }", 1),
            ("!value.!.! { y }", 10),
            ("value.!.! { y }.!", 9),
            ("value.! { y }.!.!", 17),
            ("value.!.! do y end", 9),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, col), "{source}");
            assert_eq!(found[0].message, "Avoid the use of double negation (`!!`).");
            assert!(found[0].correction.is_none());
        }
        for source in [
            "value.! { y }.!",
            "value.!.!(&b)",
            "!!value.!(&b)",
            "value.!(&b).!",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn negation_block_wrapper_remains_the_immediate_return_and_conditional_parent() {
        for (source, line) in [
            ("return value.!.! { y }", 1),
            ("def f\nreturn value.!.! { y }\nfinish\nend", 2),
            ("return value.!.! do\n y\nend", 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, 16), "{source}");
        }
        for source in [
            "def f\nif c\nvalue.!.! { y }\nfinish\nend\nend",
            "def f\nif c\nvalue.!.! do\n y\nend\nfinish\nend\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn constant_path_compounds_and_singleton_classes_select_the_tail_child() {
        for op in ["+=", "||=", "&&="] {
            let source = format!("def f\n(object(!!x))::CONST {op}\ny\nend");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 9), "{source}");
            let source = format!("def f\n(object(x))::CONST {op}\n!!y\nend");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        let source = "def f\nclass << object(!!x)\ny\nend\nend";
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!((found[0].line, found[0].col), (2, 17));
        for source in [
            "def f\nclass << object(x)\n!!y\nend\nend",
            "def f\nclass << object(!!x)\nend\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn macro_definition_exemption_covers_send_descendants_but_not_safe_navigation() {
        for name in ["define_method", "define_singleton_method"] {
            let source = format!("obj&.{name}(:f) {{ !!x }}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col),
                (1, (source.find("!!").unwrap() + 1) as u32),
                "{source}"
            );
            for source in [
                format!("{name}(:\"#{{!!x}}\") {{}}"),
                format!("obj.{name}(:\"#{{!!x}}\") {{}}"),
                format!("object(!!x).{name}(:f) {{}}"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
            for source in [
                format!("{name}(:\"#{{!!x}}\")"),
                format!("{name}(:\"#{{!!x}}\", &body)"),
            ] {
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (1, (source.find("!!").unwrap() + 1) as u32),
                    "{source}"
                );
            }
        }
    }
    #[test]
    fn ignores_other_negations_and_longer_receiver_chains() {
        for source in [
            "!test.something",
            "not not test.something",
            "foo(!!!test.something)",
            "foo(!!!!test.something)",
            "not !!test.something",
            "foo(!!not(test.something))",
            "!(!value)",
            "value&.!.!",
            "value.!(!other)",
            "!!value.!(arg)",
            "'!!value'",
            "# !!value\nvalue",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
