//! `Style/PartitionInsteadOfDoubleSelect` — adjacent complementary filters.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{BlockNode, CallNode, Node};
pub struct PartitionInsteadOfDoubleSelect;
impl Cop for PartitionInsteadOfDoubleSelect {
    fn name(&self) -> &'static str {
        "Style/PartitionInsteadOfDoubleSelect"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_statements_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Each statement list is visited once by main dispatch. No candidate
        // scans its earlier siblings or walks outward from the ProgramNode.
        let Some(statements) = node.as_statements_node() else {
            return;
        };
        // A plain explicit `begin` is a RuboCop `kwbegin`, not `begin`.
        // Rescue/ensure introduce their own implicit sequence around this body.
        if cx
            .parent()
            .and_then(|n| n.as_begin_node())
            .is_some_and(|b| {
                b.begin_keyword_loc().is_some()
                    && b.rescue_clause().is_none()
                    && b.ensure_clause().is_none()
            })
        {
            return;
        }
        let items: Vec<Node<'pr>> = statements.body().iter().collect();
        let mut previous = None;
        for (index, container) in items.iter().enumerate() {
            let current = candidate(container);
            if let (Some(first), Some(second)) = (previous, current.as_ref())
                && matching(&first, second, file)
            {
                let correction = if cx.corrections_requested() {
                    correction(&items[index - 1], container, &first, second, file)
                } else {
                    None
                };
                let (line, col) = file.line_col(container.location().start_offset());
                out.push(Offense {
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!(
                        "Use `partition` instead of consecutive `{}` and `{}` calls.",
                        String::from_utf8_lossy(first.name().as_slice()),
                        String::from_utf8_lossy(second.name().as_slice())
                    ),
                    line,
                    col,
                    end_line: file.last_line_of(container.location()),
                    correction,
                });
            }
            previous = current;
        }
    }
}
fn is_select(call: &CallNode<'_>) -> bool {
    call.name().as_slice() != b"reject"
}

/// Upstream's `autocorrect`, for two local variable assignments: the first
/// becomes `select_var, reject_var = recv.partition ...`, the second line goes.
fn correction(
    sibling_container: &Node<'_>,
    container: &Node<'_>,
    sibling: &CallNode<'_>,
    node: &CallNode<'_>,
    file: &SourceFile,
) -> Option<Correction> {
    let first = sibling_container.as_local_variable_write_node()?;
    let second = container.as_local_variable_write_node()?;
    let (first_var, second_var) = (
        String::from_utf8_lossy(first.name().as_slice()),
        String::from_utf8_lossy(second.name().as_slice()),
    );
    let (select_var, reject_var, partition) = if is_select(sibling) != is_select(node) {
        if is_select(sibling) {
            (first_var, second_var, sibling)
        } else {
            (second_var, first_var, node)
        }
    } else {
        fn body<'pr>(c: &CallNode<'pr>) -> Option<Node<'pr>> {
            c.block()?.as_block_node()?.body()
        }
        let node_negated = negated(body(node), body(sibling), file);
        let partition = if node_negated { sibling } else { node };
        if is_select(node) != node_negated {
            (second_var, first_var, partition)
        } else {
            (first_var, second_var, partition)
        }
    };
    // build_partition_call: the call's source with `partition` as selector.
    let selector = partition.message_loc()?;
    let start = partition.location().start_offset();
    let end = partition.location().end_offset();
    let call = format!(
        "{}partition{}",
        String::from_utf8_lossy(&file.bytes[start..selector.start_offset()]),
        String::from_utf8_lossy(&file.bytes[selector.end_offset()..end])
    );
    // range_by_whole_lines(include_final_newline: true)
    let lines_start = file.line_start(file.line_of(container.location().start_offset()));
    let lines_end = file.line_start(file.last_line_of(container.location()) + 1);
    Correction::from_edits(
        &file.bytes,
        vec![
            (
                sibling_container.location().start_offset()
                    ..sibling_container.location().end_offset(),
                format!("{select_var}, {reject_var} = {call}"),
            ),
            (lines_start..lines_end, String::new()),
        ],
    )
}

fn candidate<'pr>(container: &Node<'pr>) -> Option<CallNode<'pr>> {
    let value = assignment_value(container);
    let call = value.as_ref().unwrap_or(container).as_call_node()?;
    (matches!(
        call.name().as_slice(),
        b"select" | b"filter" | b"find_all" | b"reject"
    ) && call.block().is_some())
    .then_some(call)
}
fn assignment_value<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    macro_rules! values { ($($as:ident),* $(,)?) => { $(if let Some(n) = node.$as() { return Some(n.value()); })* }; }
    values!(
        as_local_variable_write_node,
        as_instance_variable_write_node,
        as_class_variable_write_node,
        as_global_variable_write_node,
        as_constant_write_node,
        as_constant_path_write_node,
        as_multi_write_node,
        as_local_variable_and_write_node,
        as_local_variable_or_write_node,
        as_local_variable_operator_write_node,
        as_instance_variable_and_write_node,
        as_instance_variable_or_write_node,
        as_instance_variable_operator_write_node,
        as_class_variable_and_write_node,
        as_class_variable_or_write_node,
        as_class_variable_operator_write_node,
        as_global_variable_and_write_node,
        as_global_variable_or_write_node,
        as_global_variable_operator_write_node,
        as_constant_and_write_node,
        as_constant_or_write_node,
        as_constant_operator_write_node,
        as_constant_path_and_write_node,
        as_constant_path_or_write_node,
        as_constant_path_operator_write_node,
        as_call_and_write_node,
        as_call_or_write_node,
        as_call_operator_write_node,
        as_index_and_write_node,
        as_index_or_write_node,
        as_index_operator_write_node
    );
    None
}
fn same_node(a: &Node<'_>, b: &Node<'_>, file: &SourceFile) -> bool {
    ast::fingerprint_with_source_lines(a, file) == ast::fingerprint_with_source_lines(b, file)
}
fn same_optional_node(a: Option<&Node<'_>>, b: Option<&Node<'_>>, file: &SourceFile) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_node(a, b, file),
        (None, None) => true,
        _ => false,
    }
}
fn matching(first: &CallNode<'_>, second: &CallNode<'_>, file: &SourceFile) -> bool {
    if !same_optional_node(first.receiver().as_ref(), second.receiver().as_ref(), file) {
        return false;
    }
    let a = first.name().as_slice();
    let b = second.name().as_slice();
    let complementary = (a == b"reject") != (b == b"reject");
    let left = first.block().unwrap();
    let right = second.block().unwrap();
    match (left.as_block_node(), right.as_block_node()) {
        (Some(l), Some(r)) => {
            if !same_parameters(&l, &r, file) {
                return false;
            }
            let lb = l.body();
            let rb = r.body();
            (complementary && same_optional_node(lb.as_ref(), rb.as_ref(), file))
                || (a == b
                    && (negated(l.body(), r.body(), file) || negated(r.body(), l.body(), file)))
        }
        (Some(block), None) => complementary && symbol_proc(&block, &right),
        (None, Some(block)) => complementary && symbol_proc(&block, &left),
        (None, None) => complementary && same_node(&left, &right, file),
    }
}
fn same_parameters(a: &BlockNode<'_>, b: &BlockNode<'_>, file: &SourceFile) -> bool {
    fn params<'pr>(b: &BlockNode<'pr>, file: &SourceFile) -> (u8, [Vec<ast::Fingerprint<'pr>>; 8]) {
        let mut parts = std::array::from_fn(|_| vec![]);
        let Some(n) = b.parameters() else {
            return (0, parts);
        };
        if n.as_numbered_parameters_node().is_some() {
            return (1, parts);
        }
        if n.as_it_parameters_node().is_some() {
            return (2, parts);
        }
        let block = n.as_block_parameters_node().unwrap();
        parts[0] = block
            .locals()
            .iter()
            .map(|n| ast::fingerprint_with_source_lines(&n, file))
            .collect();
        if let Some(p) = block.parameters() {
            for (i, list) in [p.requireds(), p.optionals(), p.posts(), p.keywords()]
                .into_iter()
                .enumerate()
            {
                parts[i + 1] = list
                    .iter()
                    .map(|n| ast::fingerprint_with_source_lines(&n, file))
                    .collect();
            }
            // Parser's args omit Prism's implicit rest for `|x,|`.
            for (i, n) in [
                p.rest().filter(|r| r.as_implicit_rest_node().is_none()),
                p.keyword_rest(),
                p.block().map(|b| b.as_node()),
            ]
            .into_iter()
            .enumerate()
            {
                parts[i + 5] = n
                    .iter()
                    .map(|n| ast::fingerprint_with_source_lines(n, file))
                    .collect();
            }
        }
        (0, parts)
    }
    params(a, file) == params(b, file)
}
fn single_body(node: Option<Node<'_>>) -> Option<Node<'_>> {
    let n = node?;
    if let Some(s) = n.as_statements_node() {
        return (s.body().len() == 1).then(|| s.body().iter().next().unwrap());
    }
    Some(n)
}
fn negated(left: Option<Node<'_>>, right: Option<Node<'_>>, file: &SourceFile) -> bool {
    let Some(call) = single_body(left).and_then(|n| n.as_call_node()) else {
        return false;
    };
    !call.is_safe_navigation()
        && call.name().as_slice() == b"!"
        && same_optional_node(call.receiver().as_ref(), single_body(right).as_ref(), file)
}
fn symbol_proc(block: &BlockNode<'_>, pass: &Node<'_>) -> bool {
    let Some(symbol) = pass
        .as_block_argument_node()
        .and_then(|b| b.expression())
        .and_then(|n| n.as_symbol_node())
    else {
        return false;
    };
    let Some(params) = block
        .parameters()
        .and_then(|n| n.as_block_parameters_node())
    else {
        return false;
    };
    if !params.locals().is_empty() {
        return false;
    }
    let Some(p) = params.parameters() else {
        return false;
    };
    if p.requireds().len() != 1
        || !p.optionals().is_empty()
        || p.rest()
            .is_some_and(|r| r.as_implicit_rest_node().is_none())
        || !p.posts().is_empty()
        || !p.keywords().is_empty()
        || p.keyword_rest().is_some()
        || p.block().is_some()
    {
        return false;
    }
    let Some(name) = p
        .requireds()
        .iter()
        .next()
        .and_then(|n| n.as_required_parameter_node())
        .map(|n| n.name())
    else {
        return false;
    };
    let Some(call) = single_body(block.body()).and_then(|n| n.as_call_node()) else {
        return false;
    };
    !call.is_safe_navigation()
        && call.name().as_slice() == symbol.unescaped()
        && ast::call_arguments(&call).is_empty()
        && call.block().is_none()
        && call
            .receiver()
            .and_then(|n| n.as_local_variable_read_node())
            .is_some_and(|n| n.name().as_slice() == name.as_slice())
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop partition_instead_of_double_select_spec.rb
    //! (MIT, (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/PartitionInsteadOfDoubleSelect")
            .collect()
    }
    #[test]
    fn complementary_methods_blocks_and_passes_exact_container_location() {
        for first in ["select", "filter", "find_all", "reject"] {
            let second = if first == "reject" {
                "select"
            } else {
                "reject"
            };
            for (a, b) in [
                (" { |x| x > 0 }", " { |x| x > 0 }"),
                (" { _1 > 0 }", " { _1 > 0 }"),
                (" { it > 0 }", " { it > 0 }"),
                ("(&:positive?)", "(&:positive?)"),
                ("(&:positive?)", " { |x| x.positive? }"),
                (" { |x| x.positive? }", "(&:positive?)"),
                ("(&predicate)", "(&predicate)"),
            ] {
                for prefix in [
                    "", "a = ", "@a = ", "@@a = ", "$a = ", "A = ", "a,b = ", "a ||= ",
                ] {
                    let s = format!(
                        "def example\n  {prefix}factory.lazy&.{first}{a}\n  {prefix}factory.lazy&.{second}{b}\nend"
                    );
                    if prefix == "A = " {
                        continue;
                    } // dynamic constants are invalid in def
                    let found = offenses(&s);
                    assert_eq!(found.len(), 1, "{s}");
                    assert_eq!((found[0].line, found[0].col), (3, 3), "{s}");
                    assert_eq!(
                        found[0].message,
                        format!(
                            "Use `partition` instead of consecutive `{first}` and `{second}` calls."
                        )
                    );
                    assert!(found[0].correction.is_none());
                }
            }
        }
    }
    #[test]
    fn trailing_commas_do_not_change_upstream_parameter_matching() {
        for (a, b) in [
            ("{ |x,| x.ok? }", "{ |x| x.ok? }"),
            ("{ |x,| x.ok? }", "(&:ok?)"),
            ("{ |x,y,| x>y }", "{ |x,y| x>y }"),
        ] {
            assert_eq!(
                offenses(&format!("arr.select{a}\narr.reject{b}")).len(),
                1,
                "{a}: {b}"
            );
        }
        assert!(offenses("arr.select { |x,*rest| x.ok? }\narr.reject(&:ok?)").is_empty());
    }
    #[test]
    fn same_method_negation_in_both_orders() {
        for method in ["select", "filter", "find_all", "reject"] {
            for (a, b) in [("x.ok?", "!x.ok?"), ("!x.ok?", "x.ok?")] {
                let s = format!("arr.{method} {{ |x| {a} }}\narr.{method} {{ |x| {b} }}");
                assert_eq!(offenses(&s).len(), 1, "{s}");
            }
        }
    }
    #[test]
    fn excludes_different_receivers_predicates_parameters_types_and_separated_calls() {
        for (a, b) in [
            ("arr.select { |x| x>0 }", "arr.reject { |y| y>0 }"),
            ("arr.select { |x| x>0 }", "arr.reject { _1>0 }"),
            ("arr.select { _1>0 }", "arr.reject { it>0 }"),
            ("arr.select { |x| x>0 }", "other.reject { |x| x>0 }"),
            ("arr.select(&:positive?)", "arr.reject(&:negative?)"),
            ("arr.select(&:positive?)", "arr.reject { |x| x&.positive? }"),
            ("arr.select(&:positive?)", "arr.reject { _1.positive? }"),
            ("arr.select { |x| x>0 }", "arr.select { |x| x>0 }"),
            ("arr.select { |x| x.ok? }", "arr.filter { |x| !x.ok? }"),
            ("arr.reject { |x| x.ok? }", "arr.reject { |x| !x.other? }"),
            ("arr.select { |x| x.ok? }", "arr.reject { |x; y| x.ok? }"),
            ("consume(arr.select(&:ok?))", "consume(arr.reject(&:ok?))"),
            ("obj.a = arr.select(&:ok?)", "obj.b = arr.reject(&:ok?)"),
        ] {
            assert!(offenses(&format!("{a}\n{b}")).is_empty(), "{a}\n{b}");
        }
        for s in [
            "arr.select(&:ok?)\nintervening\narr.reject(&:ok?)",
            "if cond\narr.select(&:ok?)\nelse\narr.reject(&:ok?)\nend",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn assignment_containers_and_ignored_call_arguments_follow_the_pin() {
        for prefix in [
            "A = ",
            "Other::A = ",
            "A ||= ",
            "obj.x ||= ",
            "obj[0] += ",
            "a,b = ",
        ] {
            let s = format!(
                "{prefix}xs.select(1) {{ |x| x.ok? }}\n{prefix}xs.reject(2) {{ |x| x.ok? }}"
            );
            let found = offenses(&s);
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col), (2, 1));
        }
        assert_eq!(offenses("arr.select {}\narr.reject { || }").len(), 1);
        assert_eq!(
            offenses("arr.select { |x| next x }\narr.reject { |x| next x }").len(),
            1
        );
        assert!(offenses("arr.select(&:ok?)\narr.reject { |x| x.ok?(1) }").is_empty());
    }
    #[test]
    fn explicit_begin_is_not_an_implicit_statement_sequence() {
        let pair = "  a = xs.select(&:ok?)\n  b = xs.reject(&:ok?)\n";
        assert!(offenses(&format!("begin\n{pair}end")).is_empty());
        // A nested implicit sequence or rescue/ensure body still maps to
        // RuboCop's `begin`, even inside an explicit `kwbegin`.
        for (opening, closing, line) in [
            ("def example\n", "end", 3),
            ("(\n", ")", 3),
            ("begin\nif condition\n", "end\nend", 4),
            ("begin\n", "rescue\n  recover\nend", 3),
            ("begin\n", "ensure\n  cleanup\nend", 3),
            ("begin\nwork\nrescue\n", "end", 5),
            ("begin\nwork\nensure\n", "end", 5),
        ] {
            let source = format!("{opening}{pair}{closing}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, 3), "{source}");
        }
    }
    #[test]
    fn source_line_equality_uses_physical_lines_not_offsets() {
        for (first, second) in [
            (
                "xs.select { |x| x == __LINE__ }",
                "xs.reject { |x| x == __LINE__ }",
            ),
            (
                "xs.select { |x| x.ok?(__LINE__) }",
                "xs.select { |x| !x.ok?(__LINE__) }",
            ),
            (
                "xs.select { |x = __LINE__| x }",
                "xs.reject { |x = __LINE__| x }",
            ),
            ("xs[__LINE__].select(&:ok?)", "xs[__LINE__].reject(&:ok?)"),
            ("xs.select(&pred(__LINE__))", "xs.reject(&pred(__LINE__))"),
        ] {
            let source = format!("{first}; {second}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, first.len() as u32 + 3));
            assert_eq!(
                found[0].message,
                format!(
                    "Use `partition` instead of consecutive `select` and `{}` calls.",
                    if second.starts_with("xs.select {") {
                        "select"
                    } else {
                        "reject"
                    }
                )
            );
            for newline in ["\n", "\r\n"] {
                assert!(offenses(&format!("{first}{newline}{second}")).is_empty());
                let found = offenses(&format!("{newline}{source}"));
                assert_eq!(found.len(), 1);
                assert_eq!(found[0].line, 2);
            }
        }
        // Supplying source lines must not also erase grouping differences.
        assert!(
            offenses("xs.select { |x| x == __LINE__ }; xs.reject { |x| (x == __LINE__) }")
                .is_empty()
        );
    }
    #[test]
    fn every_adjacent_pair_and_multiline_container_end() {
        let s = "def example\n  a = arr.select do |x|\n    capture << x; x.ok?\n  end\n  b = arr.reject do |x|\n    capture << x; x.ok?\n  end\n  c = arr.select do |x|\n    capture << x; x.ok?\n  end\nend";
        let found = offenses(s);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (5, 3, 7));
        assert_eq!((found[1].line, found[1].col, found[1].end_line), (8, 3, 10));
    }
}
