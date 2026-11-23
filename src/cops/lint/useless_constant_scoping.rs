//! `Lint/UselessConstantScoping` — private method visibility is not constant privacy.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashMap;

pub struct UselessConstantScoping;
impl Cop for UselessConstantScoping {
    fn name(&self) -> &'static str {
        "Lint/UselessConstantScoping"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Process each actual sibling sequence once, not once per assignment.
        // Prism singleton Statements/Else wrappers are absent in RuboCop's AST;
        // only the supported parent edges below may join those singleton nodes.
        if let Some(statements) = node.as_statements_node() {
            if statements.body().len() > 1 {
                self.check_siblings(statements.body().iter(), node, cx, file, out);
            }
        } else if let Some(arguments) = node.as_arguments_node() {
            self.check_siblings(arguments.arguments().iter(), node, cx, file, out);
        } else {
            let siblings = if let Some(class) = node.as_class_node() {
                [class.superclass(), singleton_statement(class.body()), None]
            } else if let Some(class) = node.as_singleton_class_node() {
                [
                    Some(class.expression()),
                    singleton_statement(class.body()),
                    None,
                ]
            } else if let Some(branch) = node.as_if_node() {
                [
                    Some(branch.predicate()),
                    singleton_statement(branch.statements().map(|s| s.as_node())),
                    singleton_statement(
                        branch
                            .subsequent()
                            .and_then(|n| n.as_else_node())
                            .and_then(|n| n.statements())
                            .map(|s| s.as_node()),
                    ),
                ]
            } else if let Some(branch) = node.as_unless_node() {
                // RuboCop represents unless as if(predicate, else_body, body).
                [
                    Some(branch.predicate()),
                    singleton_statement(
                        branch
                            .else_clause()
                            .and_then(|n| n.statements())
                            .map(|s| s.as_node()),
                    ),
                    singleton_statement(branch.statements().map(|s| s.as_node())),
                ]
            } else if let Some(block) = node.as_block_node() {
                [
                    cx.parent()
                        .and_then(|n| n.as_call_node())
                        .map(|c| c.as_node()),
                    singleton_statement(block.body()),
                    None,
                ]
            } else {
                return;
            };
            self.check_siblings(siblings.into_iter().flatten(), node, cx, file, out);
        }
    }
}

impl UselessConstantScoping {
    fn check_siblings<'pr>(
        &self,
        siblings: impl Iterator<Item = Node<'pr>>,
        parent: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Index only direct literal privatizations. Keeping their last sibling
        // index makes a later matching call an O(1) lookup per assignment.
        let mut privatizations = HashMap::new();
        let mut candidates = Vec::new();
        let ancestors: Vec<_> = std::iter::once(parent).chain(cx.ancestors()).collect();
        let mut private = false;
        // Non-conditional sequences share one macro-scope path. Cache that
        // answer locally, including structural branch comparisons, so large
        // predicates cannot re-scan the same subtree per modifier.
        let conditional = parent.as_if_node().is_some() || parent.as_unless_node().is_some();
        let mut scope = None;
        for (index, node) in siblings.enumerate() {
            if let Some(call) = node.as_call_node().filter(|c| {
                c.receiver().is_none()
                    && !c.is_safe_navigation()
                    && c.name().as_slice() == b"private_constant"
                    // Only a literal block wraps the upstream send. A forwarded
                    // block remains an argument, not a privatized literal name.
                    && !c.block().is_some_and(|b| b.as_block_node().is_some())
            }) {
                for arg in ast::call_arguments(&call) {
                    if let Some(name) = private_constant_name(&arg, file) {
                        privatizations.insert(name, index);
                    }
                }
            }
            if bare_modifier(&node)
                && (parent.as_block_node().is_some()
                    || node.as_call_node().is_some_and(|c| c.block().is_none()))
            {
                let allowed = if conditional {
                    macro_scope(&node, &ancestors, cx, file)
                } else {
                    *scope.get_or_insert_with(|| macro_scope(&node, &ancestors, cx, file))
                };
                if allowed {
                    private = node.as_call_node().unwrap().name().as_slice() == b"private";
                }
            }
            if !private {
                continue;
            }
            let (name, value) = if let Some(write) = node.as_constant_write_node() {
                (write.name().as_slice(), write.value())
            } else if let Some(write) = node.as_constant_path_write_node() {
                let target = write.target();
                let Some(name) = target.name() else { continue };
                (name.as_slice(), write.value())
            } else {
                continue;
            };
            if !class_definition(&value) {
                candidates.push((index, file.match_pattern_source(name).into_owned(), node));
            }
        }
        for (index, name, node) in candidates {
            if privatizations
                .get(&name)
                .is_some_and(|later| *later > index)
            {
                continue;
            }
            let start = node.location().start_offset();
            let end = node.location().end_offset();
            let (line, col) = file.line_col(start);
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                line,
                col,
                end_line: file.line_of(end - 1),
                message: "Useless `private` access modifier for constant scope.".into(),
                correction: None,
            });
        }
    }
}

fn private_constant_name(node: &Node<'_>, file: &SourceFile) -> Option<Vec<u8>> {
    if let Some(symbol) = node.as_symbol_node() {
        if symbol.closing_loc().is_none() {
            return Some(file.match_pattern_source(symbol.unescaped()).into_owned());
        }
        let raw = symbol.value_loc()?;
        let single = symbol
            .opening_loc()
            .is_some_and(|loc| loc.as_slice() == b":'" || loc.as_slice().starts_with(b"%s"));
        return Some(
            ast::require_literal::parts(raw.as_slice(), symbol.unescaped(), single, file)
                .into_iter()
                .flatten()
                .collect(),
        );
    }
    let mut name = Vec::new();
    append_static_string(node, &mut name, file)?;
    Some(name)
}

fn append_static_string(node: &Node<'_>, value: &mut Vec<u8>, file: &SourceFile) -> Option<()> {
    if let Some(string) = node.as_string_node() {
        let single = string
            .opening_loc()
            .is_some_and(|loc| loc.as_slice() == b"'" || loc.as_slice().starts_with(b"%q"));
        // Reuse main-parse literal alignment: transcode/compose source glyphs
        // before escapes, never normalize the already decoded escape payload.
        value.extend(
            ast::require_literal::parts(
                string.content_loc().as_slice(),
                string.unescaped(),
                single,
                file,
            )
            .into_iter()
            .flatten(),
        );
    } else {
        // Prism represents adjacent literals as an interpolated string even
        // without interpolation. Match upstream DstrNode#value for these names,
        // but never evaluate embedded expressions or treat their source as a name.
        for part in node.as_interpolated_string_node()?.parts().iter() {
            append_static_string(&part, value, file)?;
        }
    }
    Some(())
}

fn singleton_statement(body: Option<Node<'_>>) -> Option<Node<'_>> {
    let statements = body?.as_statements_node()?;
    if statements.body().len() == 1 {
        statements.body().iter().next()
    } else {
        None
    }
}

fn bare_modifier(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.receiver().is_none()
            && ast::call_arguments(&call).is_empty()
            && matches!(
                call.name().as_slice(),
                b"private" | b"public" | b"protected" | b"module_function"
            )
    })
}

fn macro_scope<'pr>(
    node: &Node<'pr>,
    ancestors: &[&Node<'pr>],
    cx: &NodeContext<'_, 'pr>,
    file: &SourceFile,
) -> bool {
    macro_scope_with(
        node,
        ancestors,
        |parent, child, predicate, body, subsequent| {
            // Upstream classifies locals AFTER source-buffer composition. Text-only
            // equality cannot reconcile a raw vcall with a composed local read (or
            // distinguish equal vcalls on opposite sides of a declaration).
            // Reuse the existing VariableForce projection over this conditional,
            // with its actual enclosing stack, at most once per branch comparison.
            // Ordinary encodings and the other macro consumers remain unchanged.
            let locals = std::cell::OnceCell::new();
            conditional_macro_child_with(child, predicate, body, subsequent, |a, b| {
                let spans = locals.get_or_init(|| {
                    if !file.match_pattern_composes_identifiers() {
                        return None;
                    }
                    // `parent` is either the sequence itself or an original entry
                    // in cx's ancestor slice. Keep precisely its enclosing scopes.
                    let index = cx
                        .ancestors
                        .iter()
                        .position(|n| std::ptr::eq(n, parent))
                        .unwrap_or(cx.ancestors.len());
                    let context = NodeContext {
                        ancestors: &cx.ancestors[..index],
                        ..*cx
                    };
                    super::non_atomic_file_operation::projected_local_spans(parent, &context, file)
                });
                ast::fingerprint_for_file_operation(a, file, spans.as_ref()).source_buffer_scope_eq(
                    &ast::fingerprint_for_file_operation(b, file, spans.as_ref()),
                    file,
                )
            })
        },
    )
}

/// Same pinned macro predicate for method modifiers. Cache structural branch
/// equality by the real conditional edge, not separately for every definition
/// in a potentially large begin predicate. Existing consumers keep their keys.
pub(super) fn method_modifier_scope(
    node: &Node<'_>,
    ancestors: &[&Node<'_>],
    file: &SourceFile,
) -> bool {
    macro_scope_with(
        node,
        ancestors,
        |parent, child, predicate, body, subsequent| {
            let key = (
                parent.location().start_offset(),
                child.location().start_offset(),
                child.location().end_offset(),
            );
            *METHOD_DEFINITION_MACRO_BRANCHES
                .get(file, Default::default)
                .lock()
                .unwrap()
                .entry(key)
                .or_insert_with(|| {
                    conditional_macro_child_with(child, predicate, body, subsequent, |a, b| {
                        ast::fingerprint_for_conditional_scope(a, file).source_buffer_scope_eq(
                            &ast::fingerprint_for_conditional_scope(b, file),
                            file,
                        )
                    })
                })
        },
    )
}

fn macro_scope_with<'pr>(
    node: &Node<'pr>,
    ancestors: &[&Node<'pr>],
    mut conditional: impl FnMut(
        &Node<'pr>,
        &Node<'pr>,
        Node<'pr>,
        Option<Node<'pr>>,
        Option<Node<'pr>>,
    ) -> bool,
) -> bool {
    let mut child = node;
    let mut skip_call = false;
    for (index, parent) in ancestors.iter().enumerate() {
        if class_definition(parent)
            || (parent.as_block_node().is_some()
                && ancestors
                    .get(index + 1)
                    .is_some_and(|n| class_definition(n)))
        {
            return true;
        }
        match parent {
            Node::ProgramNode { .. }
            | Node::ClassNode { .. }
            | Node::ModuleNode { .. }
            | Node::SingletonClassNode { .. } => return true,
            Node::StatementsNode { .. }
            | Node::ArgumentsNode { .. }
            | Node::ParenthesesNode { .. }
            | Node::ElseNode { .. } => {}
            Node::BlockNode { .. } => skip_call = true,
            Node::LambdaNode { .. } => {}
            Node::CallNode { .. } if skip_call => skip_call = false,
            Node::IfNode { .. } => {
                let branch = parent.as_if_node().unwrap();
                if !conditional(
                    parent,
                    child,
                    branch.predicate(),
                    branch.statements().map(|s| s.as_node()),
                    branch.subsequent(),
                ) {
                    return false;
                }
            }
            Node::UnlessNode { .. } => {
                let branch = parent.as_unless_node().unwrap();
                if !conditional(
                    parent,
                    child,
                    branch.predicate(),
                    branch.statements().map(|s| s.as_node()),
                    branch.else_clause().map(|s| s.as_node()),
                ) {
                    return false;
                }
            }
            Node::BeginNode { .. }
                if parent.as_begin_node().is_some_and(|b| {
                    b.rescue_clause().is_none() && b.ensure_clause().is_none()
                }) => {}
            _ => return false,
        }
        if !parent
            .as_statements_node()
            .is_some_and(|s| s.body().len() == 1)
            && parent.as_else_node().is_none()
        {
            child = parent;
        }
    }
    true
}

pub(super) fn conditional_macro_child(
    child: &Node<'_>,
    predicate: Node<'_>,
    body: Option<Node<'_>>,
    subsequent: Option<Node<'_>>,
) -> bool {
    conditional_macro_child_with(child, predicate, body, subsequent, ast::same_node)
}

fn conditional_macro_child_with(
    child: &Node<'_>,
    predicate: Node<'_>,
    body: Option<Node<'_>>,
    subsequent: Option<Node<'_>>,
    same: impl Fn(&Node<'_>, &Node<'_>) -> bool,
) -> bool {
    let start = child.location().start_offset();
    if !(predicate.location().start_offset() <= start && start < predicate.location().end_offset())
    {
        return true;
    }
    // Upstream's branch membership uses structural node equality. A predicate
    // equal to a branch is therefore accepted too (e.g. `if private; X = 1;
    // else; private; end`). Do not mistake that for general predicate scope.
    [
        body,
        subsequent.map(|n| {
            n.as_else_node()
                .and_then(|e| e.statements())
                .map_or(n, |s| s.as_node())
        }),
    ]
    .into_iter()
    .flatten()
    .any(|branch| {
        let branch = if branch
            .as_statements_node()
            .is_some_and(|s| s.body().len() == 1)
        {
            singleton_statement(Some(branch)).unwrap()
        } else {
            branch
        };
        // Parenthesized sequences and multi-statement branches are both
        // begin nodes upstream; keep singleton parentheses as a begin too.
        let child_body = child.as_parentheses_node().and_then(|p| p.body());
        let branch_body = branch.as_parentheses_node().and_then(|p| p.body());
        same(
            child_body.as_ref().unwrap_or(child),
            branch_body.as_ref().unwrap_or(&branch),
        )
    })
}

fn class_definition(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        !call.is_safe_navigation()
            && call.receiver().is_some_and(|r| {
                (call.name().as_slice() == b"new"
                    && ["Class", "Module", "Struct"]
                        .iter()
                        .any(|name| ast::is_global_const(&r, &[*name])))
                    || (call.name().as_slice() == b"define" && ast::is_global_const(&r, &["Data"]))
            })
    })
}

/// Conditional-edge equality, only for non-bare method modifiers.
pub(crate) static METHOD_DEFINITION_MACRO_BRANCHES: crate::source::Fact<
    std::sync::Mutex<MacroBranches>,
> = crate::source::Fact::new();
type MacroBranches = std::collections::HashMap<(usize, usize, usize), bool>;

#[cfg(test)]
mod tests {
    //! RuboCop lint/useless_constant_scoping_spec.rb examples (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov); source-only reference replay.
    fn check_forwarded_private_constants(kind: &str, count: usize) {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/forwarded_private_constants.json"
        ))
        .unwrap();
        let mut checked = 0;
        for row in rows.iter().filter(|r| r["kind"] == kind) {
            let found = crate::pipeline::lint_str(row["source"].as_str().unwrap());
            let actual: Vec<_> = found
                .iter()
                .filter(|o| o.cop == "Lint/UselessConstantScoping")
                .map(|o| {
                    assert!(o.correction.is_none());
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            assert_eq!(actual, expected, "{row}");
            checked += 1;
        }
        assert_eq!(checked, count);
    }

    #[test]
    fn forwarded_blocks_preserve_literal_private_constant_arguments() {
        check_forwarded_private_constants("accepted", 12);
    }

    #[test]
    fn forwarded_blocks_do_not_privatize_other_names_or_bypass_call_boundaries() {
        check_forwarded_private_constants("boundary", 16);
    }

    #[test]
    fn concatenated_private_constant_names_match_unescaped_static_parts() {
        check_forwarded_private_constants("concat_accepted", 9);
    }

    #[test]
    fn concatenated_private_constant_names_preserve_call_and_dynamic_boundaries() {
        check_forwarded_private_constants("concat_boundary", 12);
    }

    #[test]
    fn unless_macro_scopes_distinguish_branches_and_predicates() {
        let cases: &[(&str, &[(u32, u32)])] = &[
            ("unless yes\n private\n X = 1\nend\n", &[(3, 2)]),
            (
                "unless yes\n public\n X = 1\nelse\n private\n Y = 2\nend\n",
                &[(6, 2)],
            ),
            ("unless (private; X = 1)\n yes\nend\n", &[]),
            ("unless private\n X = 1\nend\n", &[]),
            ("if private\n X = 1\nelse\n private\nend\n", &[(2, 2)]),
            ("unless private\n X = 1\nelse\n private\nend\n", &[(2, 2)]),
            (
                "if (private; X = 1); private; X = 1; end\n",
                &[(1, 14), (1, 31)],
            ),
            (
                "unless (private; X = 1); private; X = 1; end\n",
                &[(1, 18), (1, 35)],
            ),
            ("unless yes\n private\nelse\n X = 1\nend\n", &[]),
            // RuboCop normalizes unless to if(predicate, else_body, body).
            ("unless yes\n X = 1\nelse\n private\nend\n", &[(2, 2)]),
            ("unless yes\n X = 1\nelse\n private_constant :X\nend\n", &[]),
            (
                "unless yes\n private\n X = 1\n private_constant :X\nend\n",
                &[],
            ),
            (
                "unless yes\n private\n X = 1\nelse\n Y = 2\nend\n",
                &[(3, 2)],
            ),
            ("private\nunless yes\n X = 1\nend\n", &[]),
        ];
        for (source, expected) in cases {
            let found = crate::pipeline::lint_str_with(&[&super::UselessConstantScoping], source);
            let positions: Vec<_> = found
                .iter()
                .map(|o| {
                    assert_eq!(
                        o.message,
                        "Useless `private` access modifier for constant scope."
                    );
                    assert!(o.correction.is_none());
                    (o.line, o.col)
                })
                .collect();
            assert_eq!(&positions, expected, "{source}");
        }
    }

    #[test]
    fn utf8_mac_macro_membership_uses_reference_buffer_but_reports_raw_positions() {
        // Exact witness: the reference buffer composes the two
        // selector spellings, making the predicate structurally equal to body.
        let source = "# coding: UTF8-MAC\nclass C; if (private; e\u{301}; X = 1); (private; é; X = 1); end; end\n";
        let found = crate::pipeline::lint_str_with(&[&super::UselessConstantScoping], source);
        let actual: Vec<_> = found
            .iter()
            .map(|o| {
                assert!(o.correction.is_none());
                assert_eq!(
                    o.message,
                    "Useless `private` access modifier for constant scope."
                );
                (o.line, o.col, o.end_line)
            })
            .collect();
        // Keep inherited original-source scalar columns, not converted columns.
        assert_eq!(actual, [(2, 27, 2), (2, 48, 2)]);
    }

    #[test]
    fn private_constant_names_project_source_characters_not_decoded_escape_results() {
        for (argument, expected) in [
            (":É", 0),
            ("\"É\"", 0),
            ("\"É\" \"\"", 0),
            ("\"E\\u0301\"", 1),
            ("\"\\u0045\\u0301\"", 1),
        ] {
            let source = format!(
                "# coding: UTF8-MAC\nclass C; private; E\u{301} = 1; private_constant {argument}, &block; end\n"
            );
            let found = crate::pipeline::lint_str_with(&[&super::UselessConstantScoping], &source);
            assert_eq!(found.len(), expected, "{source}");
            for offense in found {
                assert!(offense.correction.is_none());
                assert_eq!((offense.line, offense.col), (2, 19));
            }
        }
    }

    #[test]
    fn projected_local_macro_membership_keeps_raw_positions() {
        for (keyword, col) in [("if", 27), ("unless", 31)] {
            let source = format!(
                "# coding: UTF8-MAC\nclass C\n  {keyword} (private; é = 1; e\u{301}; X = 1)\n    (private; é = 1; é; X = 1)\n  end\nend\n"
            );
            let found = crate::pipeline::lint_str_with(&[&super::UselessConstantScoping], &source);
            let positions: Vec<_> = found.iter().map(|o| (o.line, o.col)).collect();
            assert_eq!(positions, [(3, col), (4, 25)], "{source}");
            assert!(found.iter().all(|o| o.correction.is_none()));
        }
    }

    #[test]
    fn singleton_class_receiver_is_a_sibling_of_its_singleton_body() {
        for receiver in ["private", "private()"] {
            let source = format!("class << {receiver}\n  X = 1\nend\n");
            let found = crate::pipeline::lint_str_with(&[&super::UselessConstantScoping], &source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn binding_and_singleton_boundaries() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/constant_scoping_boundaries.json"
        ))
        .unwrap();
        for row in rows {
            let bytes: Vec<u8> = crate::fixture::value(&row["bytes"]);
            let source = std::str::from_utf8(&bytes).unwrap();
            let found = crate::pipeline::lint_str_with(&[&super::UselessConstantScoping], source);
            let actual: Vec<_> = found
                .iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            let expected: Vec<(u32, u32, u32, String)> =
                crate::fixture::value(&row["raw_findings"]);
            assert_eq!(actual, expected, "{source}");
        }
    }

    #[test]
    fn constant_heavy_sibling_scaling() {
        use std::time::{Duration, Instant};
        fn measure(count: usize) -> Duration {
            let mut source = String::from("class Foo\n private\n");
            for i in 0..count {
                use std::fmt::Write;
                writeln!(source, " C{i} = 1").unwrap();
            }
            source.push_str("end\n");
            let start = Instant::now();
            let found = crate::pipeline::lint_str_with(&[&super::UselessConstantScoping], &source);
            assert_eq!(found.len(), count);
            start.elapsed()
        }
        let small = measure(1_000);
        let large = measure(8_000);
        // Eight times the input should not approach the old 64x runtime.
        // A broad ratio and fixed slack allow scheduler noise and debug builds.
        assert!(
            large < small * 24 + Duration::from_millis(50),
            "small={small:?}, large={large:?}"
        );
    }

    #[test]
    fn pinned_specs_sources_and_sibling_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/constant_scoping.json"
        ))
        .unwrap();
        for row in rows
            .iter()
            .filter(|r| r["cop"] == "Lint/UselessConstantScoping")
        {
            let found: Vec<_> = crate::pipeline::lint_str(row["source"].as_str().unwrap())
                .into_iter()
                .filter(|o| o.cop == "Lint/UselessConstantScoping")
                .collect();
            let expected: Vec<(u32, u32, u32, String)> = crate::fixture::value(&row["findings"]);
            let mut actual: Vec<_> = found
                .iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    (o.line, o.col, o.end_line, o.message.clone())
                })
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "{row}");
        }
    }
}
