//! `Lint/ConstantReassignment` — same-file, ordered lexical constant definitions.

use crate::{
    cops::{
        Cop, NodeContext, ast,
        walk::{NodeVisitor, walk},
    },
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
use std::collections::HashSet;

pub struct ConstantReassignment;
impl Cop for ConstantReassignment {
    fn name(&self) -> &'static str {
        "Lint/ConstantReassignment"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Sequential facts need a single ordered pass, not a root rescan for
        // each assignment. All state belongs to this file/investigation.
        if node.as_program_node().is_none() {
            return;
        }
        walk(
            &mut Definitions {
                cop: self,
                file,
                out,
                stack: Vec::new(),
                names: HashSet::new(),
            },
            node,
        );
    }
}

/// Byte segments avoid conflating source spellings or dynamic namespace values.
type ConstantName = Vec<Vec<u8>>;

struct Definitions<'a, 'pr> {
    cop: &'a ConstantReassignment,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
    stack: Vec<Node<'pr>>,
    // The pin keys by the joined name: a literal remove_const argument may
    // itself contain `::`, even though Ruby would reject that at runtime.
    names: HashSet<Vec<u8>>,
}

/// Constant paths preserve lexical spelling; no constant lookup/type flow.
/// `self` contributes no segment, while a leading `::` resets nesting.
fn path(node: &Node<'_>, parts: &mut ConstantName) -> Option<bool> {
    if let Some(read) = node.as_constant_read_node() {
        parts.push(read.name().as_slice().to_vec());
        Some(false)
    } else if let Some(constant) = node.as_constant_path_node() {
        let absolute = match constant.parent() {
            Some(parent) => path(&parent, parts),
            None => Some(true),
        };
        // Keyword identifiers count their constant segments even on a
        // dynamic receiver. Assignments, unlike identifiers, require Some.
        parts.push(constant.name()?.as_slice().to_vec());
        absolute
    } else if node.as_self_node().is_some() {
        Some(false)
    } else {
        None
    }
}

fn identifier<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    node.as_class_node()
        .map(|n| n.constant_path())
        .or_else(|| node.as_module_node().map(|n| n.constant_path()))
}

/// Prism-only grouping edges that do not add a RuboCop ancestor.
fn grouping(node: &Node<'_>) -> bool {
    matches!(
        node,
        Node::ProgramNode { .. } | Node::StatementsNode { .. } | Node::ParenthesesNode { .. }
    )
}

impl Definitions<'_, '_> {
    fn namespaces(&self) -> ConstantName {
        let mut result = Vec::new();
        for node in &self.stack {
            if let Some(id) = identifier(node) {
                let mut segments = Vec::new();
                if path(&id, &mut segments) == Some(true) {
                    result.clear();
                }
                result.extend(segments);
            }
        }
        result
    }

    fn simple(&self) -> bool {
        for ancestor in self.stack.iter().rev() {
            if identifier(ancestor).is_some() {
                return true;
            }
            if grouping(ancestor)
                || matches!(
                    ancestor,
                    Node::ArgumentsNode { .. } | Node::EmbeddedStatementsNode { .. }
                )
                || ast::is_literal(ancestor)
                || matches!(
                    ancestor,
                    Node::ConstantWriteNode { .. }
                        | Node::ConstantPathWriteNode { .. }
                        | Node::MultiWriteNode { .. }
                        | Node::MultiTargetNode { .. }
                )
                || ancestor.as_call_node().is_some_and(|call| {
                    !call.is_safe_navigation()
                        // Only literal blocks add RuboCop's block ancestor;
                        // a forwarded block argument leaves the send intact.
                        && call.block().is_none_or(|block| block.as_block_node().is_none())
                        && call.name().as_slice() == b"freeze"
                })
            {
                continue;
            }
            // Explicit begin/end is kwbegin, not begin; implicit rescue/ensure
            // wrappers also introduce a disallowed ancestor for their body.
            return false;
        }
        true
    }

    fn assignment(&self, node: &Node<'_>) -> Option<(Vec<u8>, ConstantName)> {
        let mut display = Vec::new();
        let mut absolute_namespace = None;
        let absolute = if let Some(write) = node.as_constant_write_node() {
            display.push(write.name().as_slice().to_vec());
            false
        } else if let Some(write) = node.as_constant_path_write_node() {
            let target = write.target();
            let absolute = path(&target.as_node(), &mut display)?;
            if absolute {
                absolute_namespace = target.parent().map(|p| p.location().as_slice().to_vec());
            }
            absolute
        } else if let Some(target) = node.as_constant_target_node() {
            display.push(target.name().as_slice().to_vec());
            false
        } else {
            let target = node.as_constant_path_target_node()?;
            let absolute = match target.parent() {
                Some(parent) => {
                    let absolute = path(&parent, &mut display)?;
                    if absolute {
                        absolute_namespace = Some(parent.location().as_slice().to_vec());
                    }
                    absolute
                }
                None => true,
            };
            display.push(target.name()?.as_slice().to_vec());
            absolute
        };
        let key = if absolute {
            // Upstream deliberately uses namespace.source for absolute writes,
            // unlike its normalized relative/keyword namespace segments. Keep
            // internal spacing (even though Ruby lookup would ignore it).
            let namespace = absolute_namespace.unwrap_or_default();
            let mut key = namespace.strip_prefix(b"::").unwrap_or(&namespace).to_vec();
            if !key.is_empty() {
                key.extend_from_slice(b"::");
            }
            key.extend_from_slice(display.last()?);
            key
        } else {
            let mut key = self.namespaces();
            key.extend(display.iter().cloned());
            key.join(b"::".as_slice())
        };
        Some((key, display))
    }

    fn inspect(&mut self, node: &Node<'_>) {
        if let Some(id) = identifier(node) {
            if self
                .stack
                .iter()
                .all(|n| grouping(n) || identifier(n).is_some())
            {
                let mut segments = Vec::new();
                let absolute = path(&id, &mut segments).unwrap_or(false);
                let mut key = if absolute {
                    Vec::new()
                } else {
                    self.namespaces()
                };
                key.extend(segments);
                self.names.insert(key.join(b"::".as_slice()));
            }
        } else if let Some(call) = node.as_call_node() {
            // Removal is deliberately not conditional-flow analysis: the pin
            // consumes every matching send, even in a branch or block.
            if call.is_safe_navigation()
                || call.name().as_slice() != b"remove_const"
                || call.receiver().is_some_and(|r| r.as_self_node().is_none())
            {
                return;
            }
            let args = ast::call_arguments(&call);
            let [arg] = args.as_slice() else {
                return;
            };
            let name = if let Some(s) = arg.as_symbol_node() {
                s.unescaped().to_vec()
            } else if let Some(s) = arg.as_string_node() {
                s.unescaped().to_vec()
            } else {
                return;
            };
            let mut key = self.namespaces();
            if key.is_empty() {
                return;
            }
            key.push(name);
            self.names.remove(&key.join(b"::".as_slice()));
        } else if matches!(
            node,
            Node::ConstantWriteNode { .. }
                | Node::ConstantPathWriteNode { .. }
                | Node::ConstantTargetNode { .. }
                | Node::ConstantPathTargetNode { .. }
        ) && self.simple()
        {
            let Some((key, display)) = self.assignment(node) else {
                return;
            };
            if !self.names.insert(key) {
                let loc = node.location();
                let (line, col) = self.file.line_col(loc.start_offset());
                let display = display
                    .iter()
                    .map(|s| String::from_utf8_lossy(s))
                    .collect::<Vec<_>>()
                    .join("::");
                self.out.push(Offense {
                    cop: self.cop.name(),
                    severity: self.cop.severity(),
                    line,
                    col,
                    end_line: self.file.line_of(loc.end_offset().saturating_sub(1)),
                    message: format!("Constant `{display}` is already assigned in this namespace."),
                    correction: None,
                });
            }
        }
    }
}

impl<'pr> NodeVisitor<'pr> for Definitions<'_, 'pr> {
    fn enter(&mut self, node: Node<'pr>) {
        self.inspect(&node);
        self.stack.push(node);
    }
    fn leave(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's constant_reassignment_spec.rb (MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ConstantReassignment")
            .collect()
    }

    #[test]
    fn reports_exact_reassignment_and_multiple_assignment_locations() {
        for (source, line, col, name) in [
            ("FOO = :bar\nFOO = :baz\n", 2, 1, "FOO"),
            ("class A\n self::FOO = 1\n self::FOO = 2\nend", 3, 2, "FOO"),
            ("module A::B\n X = 1\nend\n::A::B::X = 2", 4, 1, "A::B::X"),
            ("class A; end\nA = 1", 2, 1, "A"),
            ("module A; end\nA = 1", 2, 1, "A"),
            ("FOO = 1\nFOO, BAR = 2, 3", 2, 1, "FOO"),
            ("module A; X = 1; end\nmodule A; X = 2; end", 2, 11, "X"),
            (
                "class A\n ALL = [FOO = 1, FOO = 2].freeze\nend",
                2,
                18,
                "FOO",
            ),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("Constant `{name}` is already assigned in this namespace.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn freeze_block_arguments_do_not_introduce_a_literal_block_scope() {
        for argument in ["&block", "&nil", "&:method"] {
            let source = format!("X = 1\nvalue.freeze(X = 2, {argument})\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 14), "{source}");
            assert_eq!(
                found[0].message,
                "Constant `X` is already assigned in this namespace."
            );
            assert!(found[0].correction.is_none());
        }
        for source in [
            "X = 1\nvalue.freeze(X = 2) {}\n",
            "X = 1\nvalue.freeze(X = 2) do; end\n",
            "X = 1\nvalue&.freeze(X = 2, &block)\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn removal_is_an_ordered_syntactic_event_not_constant_lookup_or_flow() {
        for (source, expected) in [
            ("X = 1; remove_const :X; X = 2", 1),
            (
                "module A; B::X = 1; remove_const :\"B::X\"; B::X = 2; end",
                0,
            ),
            ("module A; X = 1; remove_const :X if false; X = 2; end", 0),
            ("module A; X = 1; Other.remove_const :X; X = 2; end", 1),
            ("module A; X = 1; remove_const :X; X ||= 2; X = 3; end", 0),
            (
                "module A; X = 1; remove_const :X; class X; end; X = 2; end",
                1,
            ),
            (
                "module A; X = 1; remove_const :X; end; module A; X = 2; X = 3; end",
                1,
            ),
        ] {
            assert_eq!(offenses(source).len(), expected, "{source}");
        }
    }

    #[test]
    fn keyword_definition_ownership_is_narrower_than_assignment_ownership() {
        for (source, expected) in [
            ("class A; X = 1; X = 2; end if condition", 1),
            ("if condition; class A; end; end; A = 1", 0),
            ("task { class A; X = 1; X = 2; end }; A = 1", 1),
            (
                "module object::A; X = 1; end; module other::B; X = 2; end",
                0,
            ),
            (
                "module object::A; X = 1; end; module other::A; X = 2; end",
                1,
            ),
        ] {
            assert_eq!(offenses(source).len(), expected, "{source}");
        }
        // State belongs to an investigation, not the stateless cop instance.
        assert!(offenses("X = 1").is_empty());
        assert!(offenses("X = 2").is_empty());
    }

    #[test]
    fn ignores_non_simple_assignments_and_distinct_namespaces() {
        for source in [
            "X = 1; X ||= 2",
            "X = 1; X = 2 if condition",
            "if condition; X = 1; else; X = 2; end",
            "X = 1; task { X = 2 }",
            "X = 1; -> { X = 2 }",
            "begin; X = 1; rescue; X = 2; end",
            "object::X = 1; object::X = 2",
            "module A; X = 1; end; module B; X = 2; end",
            "module A::B; X = 1; end; module C::B; X = 2; end",
            "module A; X = 1; module ::B; X = 2; end; end",
            "class A; X = 1; self.remove_const :X; X = 2; end",
            "class A; X = 1; remove_const 'X'; X = 2; end",
            "if condition; class A; end; end; A = 1",
            "A = Class.new; class A; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
