//! `Naming/MemoizedInstanceVariableName`: fixed disallowed leading underscores.
//! Unsafe upstream instance-variable renaming is deliberately detection-only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};

pub struct MemoizedInstanceVariableName;
impl Cop for MemoizedInstanceVariableName {
    fn name(&self) -> &'static str {
        "Naming/MemoizedInstanceVariableName"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_defined_node().is_some() {
            check_defined(self, node, cx, file, out);
            return;
        }
        let Some(write) = node.as_instance_variable_or_write_node() else {
            return;
        };
        let Some((method, body)) = definition(cx) else {
            return;
        };
        let Some(suggestion) = suggestion(&method, write.name().as_slice()) else {
            return;
        };
        let body = unwrap_statements(body);
        if equivalent(&body, node, file)
            || last_child(&body).is_some_and(|last| equivalent(&last, node, file))
        {
            report(self, write.name_loc(), &method, &suggestion, file, out);
        }
    }
}

/// The reference retains Symbol versus String method names: only Symbols
/// participate in its initializer exemption (including regular definitions).
struct MethodName {
    bytes: Vec<u8>,
    is_symbol: bool,
}

/// The nearest matching definition, not the nearest arbitrary block. In Prism
/// a dynamic definition's block is a child of its call, not its parent.
fn definition<'pr>(cx: &NodeContext<'_, 'pr>) -> Option<(MethodName, Node<'pr>)> {
    let mut ancestors = cx.ancestors().peekable();
    while let Some(ancestor) = ancestors.next() {
        if let Some(def) = ancestor.as_def_node() {
            return Some((
                MethodName {
                    bytes: def.name().as_slice().to_vec(),
                    is_symbol: true,
                },
                def.body()?,
            ));
        }
        if let Some(block) = ancestor.as_block_node()
            // RuboCop's matcher is `block`, not numblock/itblock. These can
            // still be ordinary children of an enclosing regular definition.
            && block.parameters().is_none_or(|p| p.as_numbered_parameters_node().is_none() && p.as_it_parameters_node().is_none())
            && let Some(call) = ancestors.peek().and_then(|n| n.as_call_node())
            && !call.is_safe_navigation()
            && matches!(
                call.name().as_slice(),
                b"define_method" | b"define_singleton_method"
            )
        {
            let Some(arguments) = call.arguments() else {
                continue;
            };
            if arguments.arguments().len() == 1 {
                let name = arguments.arguments().first().unwrap();
                let name = if let Some(symbol) = name.as_symbol_node() {
                    MethodName {
                        bytes: symbol.unescaped().to_vec(),
                        is_symbol: true,
                    }
                } else if let Some(string) = name.as_string_node() {
                    MethodName {
                        bytes: string.unescaped().to_vec(),
                        is_symbol: false,
                    }
                } else {
                    continue;
                };
                return Some((name, block.body()?));
            }
        }
    }
    None
}

/// Match only the pinned `(begin (if (defined? (ivar)) (return (ivar)) nil)
/// ... (ivasgn ...))`. This is a syntactic pattern, not a return-flow proof.
fn check_defined(
    cop: &MemoizedInstanceVariableName,
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let Some((method, body)) = definition(cx) else {
        return;
    };
    let body = unwrap_statements(body);
    let body = if let Some(paren) = body.as_parentheses_node() {
        paren.body()
    } else {
        Some(body)
    };
    let Some(body) = body else { return };
    let Some(statements) = body.as_statements_node() else {
        return;
    };
    if statements.body().len() < 2 {
        return;
    }
    let first = statements.body().iter().next().unwrap();
    let (predicate, consequent) = if let Some(guard) = first.as_if_node() {
        if guard
            .subsequent()
            .is_some_and(|n| n.as_else_node().is_none_or(|n| n.statements().is_some()))
        {
            return;
        }
        (guard.predicate(), guard.statements())
    } else if let Some(guard) = first.as_unless_node() {
        // Parser's if node swaps unless's branches. Its false branch must be
        // absent, and the return must occupy the else (true) branch.
        if guard.statements().is_some() {
            return;
        }
        (
            guard.predicate(),
            guard.else_clause().and_then(|n| n.statements()),
        )
    } else {
        return;
    };
    // Only the first guard needs to emit: other defined? callbacks would add
    // the same three locations, which RuboCop's offense set deduplicates.
    if predicate.location().start_offset() != node.location().start_offset() {
        return;
    }
    let Some(defined) = predicate.as_defined_node() else {
        return;
    };
    let Some(variable) = defined.value().as_instance_variable_read_node() else {
        return;
    };
    let Some(consequent) = consequent.map(|s| unwrap_statements(s.as_node())) else {
        return;
    };
    let Some(returned) = consequent.as_return_node() else {
        return;
    };
    let Some(args) = returned.arguments() else {
        return;
    };
    if args.arguments().len() != 1 {
        return;
    }
    let value = args.arguments().iter().next().unwrap();
    let Some(value) = value.as_instance_variable_read_node() else {
        return;
    };
    let last = statements.body().last().unwrap();
    let Some(write) = last.as_instance_variable_write_node() else {
        return;
    };
    if value.name().as_slice() != variable.name().as_slice()
        || write.name().as_slice() != variable.name().as_slice()
    {
        return;
    }
    let Some(suggestion) = suggestion(&method, write.name().as_slice()) else {
        return;
    };
    for loc in [variable.location(), value.location(), write.name_loc()] {
        report(cop, loc, &method, &suggestion, file, out);
    }
}

fn unwrap_statements(node: Node<'_>) -> Node<'_> {
    if let Some(statements) = node.as_statements_node()
        && statements.body().len() == 1
    {
        return statements.body().iter().next().unwrap();
    }
    node
}

/// RuboCop's actual `body.children.last`, not a recursive tail-expression
/// search. Typed Prism lists disappear; grouping/kwbegin remain real nodes.
fn last_child<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    macro_rules! value {
        ($($cast:ident),* $(,)?) => { $(if let Some(n) = node.$cast() { return Some(n.value()); })* };
    }
    macro_rules! body {
        ($($cast:ident),* $(,)?) => { $(if let Some(n) = node.$cast() { return n.statements().map(|s| unwrap_statements(s.as_node())); })* };
    }
    if let Some(n) = node.as_statements_node() {
        return n.body().last();
    }
    if let Some(n) = node.as_parentheses_node() {
        return n
            .body()
            .and_then(|b| b.as_statements_node().and_then(|s| s.body().last()));
    }
    if let Some(n) = node.as_begin_node() {
        // An explicit begin adds a kwbegin wrapper. An implicit method rescue
        // or ensure is itself the body node, whose last child is else/ensure.
        if n.rescue_clause().is_some() || n.ensure_clause().is_some() {
            if n.begin_keyword_loc().is_some() {
                return None;
            }
            if let Some(ensure) = n.ensure_clause() {
                return ensure.statements().map(|s| unwrap_statements(s.as_node()));
            }
            return n
                .else_clause()
                .and_then(|n| n.statements())
                .map(|s| unwrap_statements(s.as_node()));
        }
        return n.statements().and_then(|s| s.body().last());
    }
    if let Some(n) = node.as_call_node() {
        if let Some(block) = n.block() {
            return block_last_child(block);
        }
        return n.arguments().and_then(|a| a.arguments().last());
    }
    if let Some(n) = node.as_super_node() {
        if let Some(block) = n.block() {
            return block_last_child(block);
        }
        return n.arguments().and_then(|a| a.arguments().last());
    }
    if let Some(n) = node.as_forwarding_super_node() {
        return n
            .block()
            .and_then(|block| block_last_child(block.as_node()));
    }
    if let Some(n) = node.as_lambda_node() {
        return n.body().map(unwrap_statements);
    }
    if let Some(n) = node.as_class_node() {
        return n.body().map(unwrap_statements);
    }
    if let Some(n) = node.as_module_node() {
        return n.body().map(unwrap_statements);
    }
    if let Some(n) = node.as_singleton_class_node() {
        return n.body().map(unwrap_statements);
    }
    if let Some(n) = node.as_array_node() {
        return n.elements().last();
    }
    if let Some(n) = node.as_if_node() {
        return n.subsequent().map(|n| {
            n.as_else_node()
                .and_then(|n| n.statements())
                .map(|s| unwrap_statements(s.as_node()))
                .unwrap_or(n)
        });
    }
    // Ruby unless reverses the if branches. The last child is its main body.
    body!(as_unless_node, as_while_node, as_until_node, as_for_node);
    if let Some(n) = node.as_case_node() {
        return n
            .else_clause()
            .and_then(|n| n.statements())
            .map(|s| unwrap_statements(s.as_node()));
    }
    if let Some(n) = node.as_case_match_node() {
        return n
            .else_clause()
            .and_then(|n| n.statements())
            .map(|s| unwrap_statements(s.as_node()));
    }
    macro_rules! arguments {
        ($($cast:ident),* $(,)?) => { $(if let Some(n) = node.$cast() { return n.arguments().and_then(|a| a.arguments().last()); })* };
    }
    arguments!(as_return_node, as_break_node, as_next_node, as_yield_node);
    value!(
        as_defined_node,
        as_local_variable_write_node,
        as_instance_variable_write_node,
        as_class_variable_write_node,
        as_global_variable_write_node,
        as_constant_write_node,
        as_constant_path_write_node,
        as_local_variable_or_write_node,
        as_instance_variable_or_write_node,
        as_class_variable_or_write_node,
        as_global_variable_or_write_node,
        as_constant_or_write_node,
        as_constant_path_or_write_node,
        as_local_variable_and_write_node,
        as_instance_variable_and_write_node,
        as_class_variable_and_write_node,
        as_global_variable_and_write_node,
        as_constant_and_write_node,
        as_constant_path_and_write_node,
        as_local_variable_operator_write_node,
        as_instance_variable_operator_write_node,
        as_class_variable_operator_write_node,
        as_global_variable_operator_write_node,
        as_constant_operator_write_node,
        as_constant_path_operator_write_node,
        as_call_or_write_node,
        as_call_and_write_node,
        as_call_operator_write_node,
        as_index_or_write_node,
        as_index_and_write_node,
        as_index_operator_write_node,
        as_multi_write_node,
        as_assoc_node
    );
    if let Some(n) = node.as_and_node() {
        return Some(n.right());
    }
    if let Some(n) = node.as_or_node() {
        return Some(n.right());
    }
    if let Some(n) = node.as_range_node() {
        return n.right();
    }
    if let Some(n) = node.as_flip_flop_node() {
        return n.right();
    }
    if let Some(n) = node.as_splat_node() {
        return n.expression();
    }
    None
}

fn block_last_child(block: Node<'_>) -> Option<Node<'_>> {
    if let Some(block) = block.as_block_node() {
        block.body().map(unwrap_statements)
    } else {
        Some(block)
    }
}

/// The reference compares structural equality, so an identical earlier write
/// also matches the final write. Retain each relevant fingerprint once, lazily,
/// rather than rewalking a large final RHS for every sibling candidate.
#[derive(Default)]
pub(crate) struct Fingerprints(
    std::collections::HashMap<(usize, usize), ast::Fingerprint<'static>>,
);
fn equivalent(a: &Node<'_>, b: &Node<'_>, file: &SourceFile) -> bool {
    let Some(a_write) = a.as_instance_variable_or_write_node() else {
        return false;
    };
    let Some(b_write) = b.as_instance_variable_or_write_node() else {
        return false;
    };
    if a_write.name().as_slice() != b_write.name().as_slice() {
        return false;
    }
    let key = |n: &Node<'_>| (n.location().start_offset(), n.location().end_offset());
    if key(a) == key(b) {
        return true;
    }
    let mut cache = file.memoized_name_fingerprints().lock().unwrap();
    for node in [a, b] {
        cache
            .0
            .entry(key(node))
            .or_insert_with(|| ast::fingerprint_for_detection(node, file).into_owned());
    }
    cache.0[&key(a)] == cache.0[&key(b)]
}

fn suggestion(method: &MethodName, variable: &[u8]) -> Option<String> {
    if method.is_symbol
        && matches!(
            method.bytes.as_slice(),
            b"initialize" | b"initialize_clone" | b"initialize_copy" | b"initialize_dup"
        )
    {
        return None;
    }
    let name: Vec<_> = method
        .bytes
        .iter()
        .copied()
        .filter(|b| !b"!?=".contains(b))
        .collect();
    if !name
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        || !name.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return None;
    }
    let variable = variable.strip_prefix(b"@").unwrap_or(variable);
    if variable == name || variable == name.strip_prefix(b"_").unwrap_or(&name) {
        return None;
    }
    Some(String::from_utf8(name).unwrap())
}

fn report(
    cop: &MemoizedInstanceVariableName,
    loc: Location<'_>,
    method: &MethodName,
    suggestion: &str,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let (line, col) = file.line_col(loc.start_offset());
    out.push(Offense {
        correction: None,
        cop: cop.name(),
        severity: cop.severity(),
        message: format!(
            "Memoized variable `{}` does not match method name `{}`. Use `@{suggestion}` instead.",
            String::from_utf8_lossy(loc.as_slice()),
            String::from_utf8_lossy(&method.bytes)
        ),
        line,
        col,
        end_line: file.line_of(loc.end_offset().saturating_sub(1)),
    });
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop naming/memoized_instance_variable_name_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Naming/MemoizedInstanceVariableName")
            .collect()
    }
    fn assert_review_finding(finding: &str) {
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/ns07_review.json")).unwrap();
        for row in rows.iter().filter(|r| r["finding"] == finding) {
            let found = offenses(row["source"].as_str().unwrap());
            assert_eq!(
                serde_json::json!(found.iter().map(|o| [o.line, o.col]).collect::<Vec<_>>()),
                row["positions"],
                "{row}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                row["messages"],
                "{row}"
            );
            assert!(found.iter().all(|o| o.correction.is_none()), "{row}");
        }
    }

    #[test]
    fn review_f1_super_blocks_are_the_last_direct_child() {
        assert_review_finding("F1");
    }

    #[test]
    fn review_f2_duplicate_assignments_use_detection_equality() {
        assert_review_finding("F2");
    }

    #[test]
    fn review_f3_unless_guards_reverse_branch_orientation() {
        assert_review_finding("F3");
    }

    #[test]
    fn review_f4_only_symbol_named_initializers_are_exempt() {
        assert_review_finding("F4");
    }

    #[test]
    fn accepts_nonmemoization_operators_initializers_and_matching_names() {
        for source in [
            "@bad ||= 1",
            "def foo; @bad = 1; end",
            "def foo; @bad &&= 1; end",
            "def foo; @bad ||= 1; helper; end",
            "def foo; @foo ||= 1; end",
            "def _foo; @foo ||= 1; end",
            "def _foo; @_foo ||= 1; end",
            "def []; @bad ||= 1; end",
            "def initialize; @bad ||= 1; end",
            "# @bad ||= 1\n'@bad ||= 1'",
            "def foo; return @bad if defined?(@bad); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        assert_eq!(offenses("def foo; @_foo ||= 1; end").len(), 1);
    }

    #[test]
    fn structural_tail_cache_is_lazy_and_computed_once_per_relevant_node() {
        let source = "def foo; @bad ||= 1; end\n".repeat(2000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parse = crate::parser::parse(&file.bytes);
        let mut found = Vec::new();
        crate::cops::run_cops(&[&MemoizedInstanceVariableName], &parse, &file, &mut found);
        assert_eq!(found.len(), 2000);
        assert!(
            file.memoized_name_fingerprints()
                .lock()
                .unwrap()
                .0
                .is_empty()
        );

        let source = format!(
            "def foo\n{}@bad ||= [{}]\nend\n",
            "@bad ||= 1\n".repeat(2000),
            "1,".repeat(2000)
        );
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parse = crate::parser::parse(&file.bytes);
        found.clear();
        crate::cops::run_cops(&[&MemoizedInstanceVariableName], &parse, &file, &mut found);
        assert_eq!(found.len(), 1);
        assert_eq!(
            file.memoized_name_fingerprints().lock().unwrap().0.len(),
            2001
        );
        found.clear();
        crate::cops::run_cops(&[&MemoizedInstanceVariableName], &parse, &file, &mut found);
        assert_eq!(found.len(), 1);
        assert_eq!(
            file.memoized_name_fingerprints().lock().unwrap().0.len(),
            2001
        );
    }

    #[test]
    fn flags_all_three_defined_memoization_identifiers() {
        let found =
            offenses("def x\n  return @my_var if defined?(@my_var)\n  @my_var = false\nend\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(2, 10), (2, 30), (3, 3)]
        );
        assert!(found.iter().all(|o| o.message
            == "Memoized variable `@my_var` does not match method name `x`. Use `@x` instead."
            && o.correction.is_none()));
    }
    #[test]
    fn flags_mismatched_tail_or_assignment_at_the_variable() {
        let found = offenses("def x\n  @my_var ||= :foo\nend\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 3));
        assert_eq!(
            found[0].message,
            "Memoized variable `@my_var` does not match method name `x`. Use `@x` instead."
        );
        assert!(found[0].correction.is_none());
    }
}
