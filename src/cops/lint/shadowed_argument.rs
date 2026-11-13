//! `Lint/ShadowedArgument` — arguments overwritten before their first use.

use crate::{
    cops::{
        Cop, NodeContext,
        variables::{AssignmentKind, Declaration, ParameterKind, ReferenceKind, ScopeAnalysis},
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::{collections::HashMap, mem::Discriminant};

/// Detection-only argument shadowing at the fixed upstream defaults.
pub struct ShadowedArgument;
impl Cop for ShadowedArgument {
    fn name(&self) -> &'static str {
        "Lint/ShadowedArgument"
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_program_node().is_some()
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
        let analysis = ScopeAnalysis::for_shadowed_arguments(&program, file);
        let syntax = Syntax::of(node, file, &analysis);
        for scope in analysis.scopes_in_exit_order() {
            let boundary = syntax.indices[&key(&analysis.scope(scope).node)];
            for id in analysis.scope(scope).variables() {
                let argument = analysis.variable(id);
                if !argument.is_parameter()
                    || argument.declaration == Declaration::Parameter(ParameterKind::BlockLocal)
                    || !argument.is_referenced()
                {
                    continue;
                }
                let mut known = true;
                for id in &argument.assignments {
                    let assignment = analysis.assignment(*id);
                    let index = syntax.indices[&key(&assignment.node)];
                    let meta = syntax.meta(index, assignment.kind);
                    if assignment.kind == AssignmentKind::Operator {
                        known = false;
                        continue;
                    }
                    if syntax.uses(meta, argument.name, file) {
                        continue;
                    }
                    if syntax.conditional(meta, boundary) {
                        known = false;
                        continue;
                    }
                    let pos = assignment.node.location().start_offset();
                    let used = argument
                        .references
                        .iter()
                        .map(|r| analysis.reference(*r))
                        .any(|r| {
                            !(r.is_explicit() && r.assignment_owned)
                                && syntax.reference_pos(&r.range) <= pos
                        });
                    if !used {
                        let range = if known {
                            let loc = assignment.node.location();
                            if matches!(
                                assignment.kind,
                                AssignmentKind::Write | AssignmentKind::NamedCapture
                            ) {
                                loc.start_offset()..loc.end_offset()
                            } else {
                                assignment.name_range.clone()
                            }
                        } else {
                            argument.declaration_range.clone()
                        };
                        let (line, col) =
                            super::array_literal_in_regexp::reference_line_col(range.start, file);
                        // Preserve binary identifier bytes in printable diagnostics,
                        // as other source-projected cops do, rather than U+FFFD.
                        let name: String = file
                            .match_pattern_source(argument.name)
                            .utf8_chunks()
                            .map(|chunk| {
                                let mut text = chunk.valid().to_owned();
                                for byte in chunk.invalid() {
                                    text.push_str(&format!("\\x{byte:02X}"));
                                }
                                text
                            })
                            .collect();
                        out.push(Offense {
                            correction: None,
                            cop: self.name(),
                            severity: self.severity(),
                            message: format!(
                                "Argument `{name}` was shadowed by a local variable before it was used."
                            ),
                            line,
                            col,
                            end_line: file.line_of(range.end.saturating_sub(1).max(range.start)),
                        });
                    }
                    break;
                }
            }
        }
    }
}

type Key<'pr> = (usize, usize, Discriminant<Node<'pr>>);
fn key<'pr>(node: &Node<'pr>) -> Key<'pr> {
    let loc = node.location();
    (
        loc.start_offset(),
        loc.end_offset(),
        std::mem::discriminant(node),
    )
}
struct Entry<'pr> {
    node: Node<'pr>,
    parent: Option<usize>,
    end: usize,
}
/// One typed-child-complete syntax index, independent of physical containment
/// (heredoc bodies can lie outside all their ancestors' byte ranges).
struct Syntax<'pr> {
    entries: Vec<Entry<'pr>>,
    indices: HashMap<Key<'pr>, usize>,
    reads: HashMap<Vec<u8>, Vec<usize>>,
    reference_positions: HashMap<(usize, usize), usize>,
}
impl<'pr> Syntax<'pr> {
    fn of(root: &Node<'pr>, file: &SourceFile, analysis: &ScopeAnalysis<'pr>) -> Self {
        let mut projected = HashMap::new();
        for scope in analysis.scopes_in_exit_order() {
            for id in analysis.scope(scope).variables() {
                let variable = analysis.variable(id);
                for id in &variable.references {
                    let reference = analysis.reference(*id);
                    if reference.kind == ReferenceKind::Read {
                        projected
                            .insert((reference.range.start, reference.range.end), variable.name);
                    }
                }
            }
        }
        struct Builder<'a, 'pr> {
            syntax: Syntax<'pr>,
            stack: Vec<usize>,
            file: &'a SourceFile,
            projected: HashMap<(usize, usize), &'pr [u8]>,
        }
        impl<'pr> NodeVisitor<'pr> for Builder<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                let index = self.syntax.entries.len();
                let name = node
                    .as_local_variable_read_node()
                    .map(|n| n.name().as_slice())
                    .or_else(|| {
                        node.as_call_node().and_then(|_| {
                            self.projected
                                .get(&(
                                    node.location().start_offset(),
                                    node.location().end_offset(),
                                ))
                                .copied()
                        })
                    });
                if let Some(name) = name {
                    self.syntax
                        .reads
                        .entry(self.file.match_pattern_source(name).into_owned())
                        .or_default()
                        .push(index);
                }
                if let Some(parent) = self.stack.last().map(|i| &self.syntax.entries[*i].node)
                    && parent.as_multi_write_node().is_some()
                {
                    self.syntax.reference_positions.insert(
                        (node.location().start_offset(), node.location().end_offset()),
                        parent.location().start_offset(),
                    );
                }
                self.syntax.indices.insert(key(&node), index);
                self.syntax.entries.push(Entry {
                    node,
                    parent: self.stack.last().copied(),
                    end: 0,
                });
                self.stack.push(index);
            }
            fn leave(&mut self) {
                let index = self.stack.pop().unwrap();
                self.syntax.entries[index].end = self.syntax.entries.len();
            }
        }
        let mut builder = Builder {
            syntax: Self {
                entries: Vec::new(),
                indices: HashMap::new(),
                reads: HashMap::new(),
                reference_positions: HashMap::new(),
            },
            stack: Vec::new(),
            file,
            projected,
        };
        walk(&mut builder, root);
        builder.syntax
    }
    fn uses(&self, index: usize, name: &[u8], file: &SourceFile) -> bool {
        self.reads
            .get(file.match_pattern_source(name).as_ref())
            .is_some_and(|reads| {
                reads
                    .get(reads.partition_point(|p| *p < index))
                    .is_some_and(|p| *p < self.entries[index].end)
            })
    }
    fn meta(&self, index: usize, kind: AssignmentKind) -> usize {
        if !matches!(kind, AssignmentKind::Multiple | AssignmentKind::For) {
            return index;
        }
        let rest = self.entries[index]
            .parent
            .filter(|p| self.entries[*p].node.as_splat_node().is_some());
        let mut current = index;
        while let Some(parent) = self.entries[current].parent {
            let node = &self.entries[parent].node;
            if node.as_multi_write_node().is_some() {
                return parent;
            }
            if node.as_for_node().is_some() {
                // Upstream selects masgn, then an immediate splat, then for.
                // A splat's meta-assignment does not include loop-body reads.
                let direct = self.entries[index].parent.is_some_and(|p| {
                    p == parent
                        || (self.entries[p].node.as_multi_target_node().is_some()
                            && self.entries[p].parent == Some(parent))
                });
                return rest.unwrap_or(if direct { parent } else { index });
            }
            if !matches!(node, Node::MultiTargetNode { .. } | Node::SplatNode { .. }) {
                break;
            }
            current = parent;
        }
        index
    }
    fn reference_pos(&self, range: &std::ops::Range<usize>) -> usize {
        // RuboCop compares a direct masgn child at the masgn's source position,
        // not its evaluation position or the position of a nested descendant.
        self.reference_positions
            .get(&(range.start, range.end))
            .copied()
            .unwrap_or(range.start)
    }
    fn conditional(&self, mut child: usize, boundary: usize) -> bool {
        while let Some(index) = self.entries[child].parent {
            if index == boundary {
                return false;
            }
            let node = &self.entries[index].node;
            if matches!(
                node,
                Node::IfNode { .. }
                    | Node::UnlessNode { .. }
                    | Node::CaseNode { .. }
                    | Node::CaseMatchNode { .. }
                    | Node::RescueModifierNode { .. }
            ) || ordinary_block(node)
                || node
                    .as_call_node()
                    .is_some_and(|n| n.block().is_some_and(|b| ordinary_block(&b)))
                || node.as_while_node().is_some_and(|n| !n.is_begin_modifier())
                || node.as_until_node().is_some_and(|n| !n.is_begin_modifier())
                || node.as_begin_node().is_some_and(|n| {
                    n.rescue_clause().is_some()
                        && self.entries[child].node.as_ensure_node().is_none()
                })
            {
                return true;
            }
            child = index;
        }
        false
    }
}

/// Prism shares block/lambda node types for ordinary, numbered and `it`
/// parameters. Only ordinary blocks are conditional in the pinned AST, including
/// the synthetic block ancestor above the call's receiver and arguments.
fn ordinary_block(node: &Node<'_>) -> bool {
    let parameters = if let Some(block) = node.as_block_node() {
        block.parameters()
    } else if let Some(lambda) = node.as_lambda_node() {
        lambda.parameters()
    } else {
        return false;
    };
    !matches!(
        parameters,
        Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
    )
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's shadowed_argument_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ShadowedArgument")
            .collect()
    }
    #[test]
    fn reports_argument_overwrite_at_assignment() {
        for header in [
            "def f(foo)",
            "def f(foo = 1)",
            "def f(foo:)",
            "def f(*foo)",
            "def f(**foo)",
            "def f(&foo)",
            "xs.each do |foo|",
        ] {
            let source = format!("{header}\n  foo = 42\n  puts foo\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                "Argument `foo` was shadowed by a local variable before it was used."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn reports_uncertain_earlier_assignments_at_declaration() {
        for first in [
            "foo=43 if flag",
            "proc { foo=43 }",
            "consume(foo=43) {}",
            "begin; nil; rescue; foo=43; end",
        ] {
            let source = format!("def f(foo); {first}; foo=42; puts foo; end");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 7), "{source}");
        }
    }
    #[test]
    fn implicit_parameter_blocks_preserve_assignment_and_call_locations() {
        for parameter in ["_1", "it"] {
            for wrapper in [
                format!("proc {{ {parameter}; foo=43 }}"),
                format!("-> {{ {parameter}; foo=43 }}"),
                format!("consume(foo=43) {{ {parameter} }}"),
                format!("object&.consume(foo=43) {{ {parameter} }}"),
            ] {
                for later in ["", "foo=42; "] {
                    let source = format!("def f(foo); {wrapper}; {later}puts foo; end");
                    let found = offenses(&source);
                    assert_eq!(found.len(), 1, "{source}");
                    assert_eq!(
                        (found[0].line, found[0].col as usize),
                        (1, source.find("foo=43").unwrap() + 1),
                        "{source}"
                    );
                }
            }
            for body in ["puts foo; foo=43", "foo=foo+1", "foo=43 if flag"] {
                let source = format!("def f(foo); proc {{ {parameter}; {body} }}; puts foo; end");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn splatted_for_targets_do_not_include_body_reads_in_meta_assignment() {
        let source = "def f(foo)\n  for *foo in xs\n    puts foo\n  end\nend";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 8, 2));
        assert_eq!(
            found[0].message,
            "Argument `foo` was shadowed by a local variable before it was used."
        );
        // A nested mlhs target is not a direct for assignment upstream either.
        let nested = "def f(foo); for ((foo, bar), *baz) in xs; puts foo; end; end";
        assert_eq!(offenses(nested)[0].col, 19);
        for source in [
            "def f(foo); puts foo; for *foo in xs; puts foo; end; end",
            "def f(foo); for *foo in xs; nil; end; end",
            "def f(foo); for foo in xs; puts foo; end; end",
            "def f(foo); *foo, bar=foo; puts foo; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn excludes_assignment_owned_references_not_rhs_argument_reads() {
        assert_eq!(
            offenses("def f(foo); proc { foo=43; puts foo }; foo=42; puts foo; end")[0].col,
            7
        );
        assert!(
            offenses("def f(foo); proc { puts foo; foo=43 }; foo=42; puts foo; end").is_empty()
        );
        assert!(offenses("def f(*foo); *foo, last=foo; puts foo; end").is_empty());
        assert_eq!(
            offenses("def f(*foo); *foo, last=[42,42]; puts foo; end")[0].col,
            15
        );
    }
    #[test]
    fn default_implicit_references_count_without_the_opt_in_exemption() {
        for body in [
            "foo=42; super",
            "foo=super; foo",
            "foo=42; binding",
            "foo=42; 1.binding",
        ] {
            assert_eq!(
                offenses(&format!("def f(foo); {body}; end")).len(),
                1,
                "{body}"
            );
        }
        for body in [
            "foo=42; send(:binding)",
            "foo=42; object&.binding",
            "binding; foo=42; foo",
            "super; foo=42; foo",
        ] {
            assert!(
                offenses(&format!("def f(foo); {body}; end")).is_empty(),
                "{body}"
            );
        }
        assert!(offenses("xs.each { |foo| foo=42; super }").is_empty());
    }
    #[test]
    fn ensure_and_posttest_loops_are_not_rescue_or_pretest_conditionals() {
        for first in [
            "begin; nil; rescue; nil; ensure; foo=43; end",
            "begin; foo=43; end while flag",
        ] {
            let source = format!("def f(foo); {first}; foo=42; foo; end");
            let found = offenses(&source);
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].col as usize, source.find("foo=43").unwrap() + 1);
        }
    }
    #[test]
    fn utf8_mac_projection_is_local_and_preserves_original_parse_bindings() {
        for (param, write, read) in [
            ("é", "e\u{301}", "é"),
            ("e\u{301}", "é", "e\u{301}"),
            ("é", "é", "e\u{301}"),
        ] {
            let source =
                format!("# coding: UTF8-MAC\ndef f({param}); {write}=42; puts {read}; end\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 11));
            assert!(found[0].message.contains("`é`"));
            if param != write || param != read {
                assert!(offenses(&source.replace("UTF8-MAC", "UTF-8")).is_empty());
            }
        }
    }
    #[test]
    fn reports_full_multiline_assignment_and_declaration_ranges() {
        let found = offenses("def f(foo)\n foo = (\n 42\n ); puts foo\nend");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 2, 4));
        let found = offenses("def f(foo=(\n1\n)); foo=43 if flag; foo=42; puts foo; end");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 7, 3));
    }
    #[test]
    fn ignores_used_unused_and_block_local_arguments() {
        for source in [
            "def f(foo); foo = foo + 1; puts foo; end",
            "def f(foo); puts foo; foo = 42; puts foo; end",
            "def f(foo); foo = 42; end",
            "xs.each { |i; foo| foo = i; puts foo }",
            "def f(foo); foo = 42 if flag; puts foo; end",
            "def f(foo); proc { foo = 42 }; puts foo; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
