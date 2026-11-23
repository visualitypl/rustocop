//! `Style/SwapValues` — detect temporary-variable swaps; never remove the temporary.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct SwapValues;
impl Cop for SwapValues {
    fn name(&self) -> &'static str {
        "Style/SwapValues"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(
            node,
            Node::StatementsNode { .. }
                | Node::ArrayNode { .. }
                | Node::ArgumentsNode { .. }
                | Node::CallNode { .. }
                | Node::IfNode { .. }
                | Node::UnlessNode { .. }
                | Node::WhenNode { .. }
        )
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // RuboCop checks immediate right siblings, not a search for later writes.
        // Visit each container once: even a long assignment sequence is linear.
        let children: Vec<_> = if let Some(n) = node.as_statements_node() {
            n.body().iter().collect()
        } else if let Some(n) = node.as_array_node() {
            n.elements().iter().collect()
        } else if let Some(n) = node.as_arguments_node() {
            if cx.parent().is_some_and(|p| p.as_call_node().is_some()) {
                return;
            }
            n.arguments().iter().collect()
        } else if let Some(n) = node.as_call_node() {
            // The receiver and argument list are siblings in the reference
            // AST; its method-name scalar is a barrier after the receiver.
            ast::call_arguments(&n)
        } else if let Some(n) = node.as_if_node() {
            if let (Some(a), Some(b)) = (
                single(n.statements()),
                n.subsequent()
                    .and_then(|e| e.as_else_node())
                    .and_then(|e| single(e.statements())),
            ) {
                check(
                    self,
                    [&n.predicate(), &a, &b],
                    cx.corrections_requested(),
                    file,
                    out,
                );
            }
            return;
        } else if let Some(n) = node.as_unless_node() {
            if let (Some(a), Some(b)) = (
                single(n.statements()),
                n.else_clause().and_then(|e| single(e.statements())),
            ) {
                // RuboCop normalizes unless to if, reversing the body slots.
                check(
                    self,
                    [&n.predicate(), &b, &a],
                    cx.corrections_requested(),
                    file,
                    out,
                );
            }
            return;
        } else if let Some(n) = node.as_when_node() {
            let mut children: Vec<_> = n.conditions().iter().collect();
            if let Some(body) = single(n.statements()) {
                children.push(body);
            }
            children
        } else {
            return;
        };
        for triple in children.windows(3) {
            #[cfg(test)]
            TRIPLES.with(|n| n.set(n.get() + 1));
            let [tmp, x, y] = triple else { unreachable!() };
            check(self, [tmp, x, y], cx.corrections_requested(), file, out);
        }
    }
}
fn single(body: Option<ruby_prism::StatementsNode<'_>>) -> Option<Node<'_>> {
    let body = body?;
    (body.body().len() == 1).then(|| body.body().first().unwrap())
}
fn check(
    cop: &SwapValues,
    [tmp, x, y]: [&Node<'_>; 3],
    fix: bool,
    file: &SourceFile,
    out: &mut Vec<Offense>,
) {
    let (Some((tmp_name, tmp_value)), Some((x_name, x_value)), Some((y_name, y_value))) =
        (assignment(tmp), assignment(x), assignment(y))
    else {
        return;
    };
    if tmp_value.location().as_slice() != x_name
        || x_value.location().as_slice() != y_name
        || y_value.location().as_slice() != tmp_name
    {
        return;
    }
    // Source encodings need not be UTF-8. Only render names after comparing
    // their original bytes, so replacement characters cannot affect detection.
    let x_name = String::from_utf8_lossy(&x_name);
    let y_name = String::from_utf8_lossy(&y_name);
    let x_line = file.line_of(x.location().start_offset());
    let y_line = file.line_of(y.location().start_offset());
    // The three statements' whole lines (without the last line break).
    let correction = fix.then(|| {
        let start = file.line_start(file.line_of(tmp.location().start_offset()));
        let last = file.line_of(y.location().end_offset().saturating_sub(1));
        let mut end = if (last as usize) < file.line_count() {
            file.line_start(last + 1)
        } else {
            file.bytes.len()
        };
        while end > start && matches!(file.bytes[end - 1], b'\n' | b'\r') {
            end -= 1;
        }
        crate::offense::Correction {
            range: start..end,
            replacement: format!("{x_name}, {y_name} = {y_name}, {x_name}"),
        }
    });
    super::negated_if::report(
        cop,
        tmp,
        file,
        format!(
            "Replace this and assignments at lines {x_line} and {y_line} with `{x_name}, {y_name} = {y_name}, {x_name}`."
        ),
        correction,
        out,
    );
}
fn assignment<'pr>(node: &Node<'pr>) -> Option<(Vec<u8>, Node<'pr>)> {
    macro_rules! variable {
        ($as:ident) => {{
            let n = node.$as()?;
            (n.name().as_slice().to_vec(), n.value())
        }};
    }
    Some(match node {
        Node::LocalVariableWriteNode { .. } => variable!(as_local_variable_write_node),
        Node::InstanceVariableWriteNode { .. } => variable!(as_instance_variable_write_node),
        Node::ClassVariableWriteNode { .. } => variable!(as_class_variable_write_node),
        Node::GlobalVariableWriteNode { .. } => variable!(as_global_variable_write_node),
        Node::ConstantWriteNode { .. } => variable!(as_constant_write_node),
        Node::ConstantPathWriteNode { .. } => {
            let n = node.as_constant_path_write_node()?;
            (constant_path_name(&n.target().as_node()), n.value())
        }
        _ => return None,
    })
}
// Like RuboCop's const_name plus absolute prefix, but retain identifier bytes.
// Normalize separators rather than copying whitespace from the target source;
// a non-constant namespace renders empty (self::Foo -> ::Foo).
fn constant_path_name(node: &Node<'_>) -> Vec<u8> {
    if let Some(n) = node.as_constant_read_node() {
        n.name().as_slice().to_vec()
    } else if let Some(n) = node.as_constant_path_node() {
        let mut name = n.parent().map_or_else(Vec::new, |p| constant_path_name(&p));
        name.extend_from_slice(b"::");
        if let Some(short) = n.name() {
            name.extend_from_slice(short.as_slice());
        }
        name
    } else {
        Vec::new()
    }
}
#[cfg(test)]
thread_local! { static TRIPLES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/swap_values_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    #[test]
    fn all_temporary_kinds_nested_bodies_and_same_line_siblings() {
        for tmp in ["tmp", "@tmp", "@@tmp", "$tmp", "Tmp", "::Tmp", "Foo::Tmp"] {
            for (x, y) in [("x", "y"), ("@x", "$y"), ("::X", "Foo::Y")] {
                let source = format!("begin; {tmp} = {x}; {x} = {y}; {y} = {tmp}; end");
                assert_eq!(
                    crate::parser::parse(source.as_bytes()).errors().count(),
                    0,
                    "{source}"
                );
                let found = correction_offenses(&SwapValues, &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (1, 8));
                assert_eq!(
                    found[0].message,
                    format!(
                        "Replace this and assignments at lines 1 and 1 with `{x}, {y} = {y}, {x}`."
                    )
                );
                assert!(found[0].correction.is_some());
            }
        }
        for source in [
            "begin; tmp=x; x=y; y=tmp; end",
            "items.each { tmp=x; x=y; y=tmp }",
            "begin; work; rescue; tmp=x; x=y; y=tmp; ensure; done; end",
            "[tmp=x, x=y, y=tmp]",
            "consume(tmp=x, x=y, y=tmp)",
            "def f; yield(tmp=x, x=y, y=tmp); end",
            "if tmp=x; x=y; else; y=tmp; end",
            "case value; when tmp=x, x=y, y=tmp; work; end",
        ] {
            assert_eq!(
                correction_offenses(&SwapValues, source).len(),
                1,
                "{source}"
            );
        }
    }
    #[test]
    fn unless_siblings_follow_normalized_true_false_branch_order() {
        assert!(correction_offenses(&SwapValues, "unless tmp=x; x=y; else; y=tmp; end").is_empty());
        let found = correction_offenses(&SwapValues, "unless tmp=x; y=tmp; else; x=y; end");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 8));
    }
    #[test]
    fn near_misses_shorthand_multiple_writes_and_sibling_barriers() {
        for source in [
            "(tmp=x) ? x=y : y=tmp",
            "tmp=x; x=y; y=other",
            "tmp=x; barrier; x=y; y=tmp",
            "tmp=x; x=y; barrier; y=tmp",
            "tmp=x; x||=y; y=tmp",
            "tmp||=x; x=y; y=tmp",
            "tmp=x; x,y=y,tmp",
            "obj.tmp=x; x=y; y=obj.tmp",
            "tmp=obj.x; obj.x=y; y=tmp",
            "tmp=x; x=y; obj.y=tmp",
            "tmp=(x); x=y; y=tmp",
            "tmp=x; if ok; x=y; end; y=tmp",
            "x,y=y,x",
            "def (foo = Object.new).do_something; end",
        ] {
            assert!(
                !lint_str(source).iter().any(|o| o.cop == SwapValues.name()),
                "{source}"
            );
        }
    }
    #[test]
    fn sibling_scanning_is_linear() {
        let source = "tmp=x; x=y; y=tmp; barrier\n".repeat(1000);
        TRIPLES.with(|n| n.set(0));
        let found = correction_offenses(&SwapValues, &source);
        assert_eq!(found.len(), 1000);
        assert!(found.iter().all(|o| o.correction.is_some()));
        TRIPLES.with(|n| assert_eq!(n.get(), 3998));
    }
    #[test]
    fn every_pinned_call_has_exact_diagnostics_and_a_correction() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/swap_values_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.len(), 11);
        for c in cases {
            let source = c["source"].as_str().unwrap();
            let found: Vec<_> = lint_str(source)
                .into_iter()
                .filter(|o| o.cop == SwapValues.name())
                .collect();
            assert_eq!(
                serde_json::json!(found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>()),
                c["positions"],
                "{source}"
            );
            assert_eq!(
                serde_json::json!(found.iter().map(|o| &o.message).collect::<Vec<_>>()),
                c["messages"],
                "{source}"
            );
            assert!(
                correction_offenses(&SwapValues, source)
                    .iter()
                    .all(|o| o.correction.is_some())
            );
        }
        assert!(!SwapValues.safe_fixes());
    }
}
