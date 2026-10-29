//! `Lint/SuperArgumentMismatch`: explicit super positional arity.
//! Requires an assigned index; fixed UseProjectIndex=false supplies none.
//! No public index mode, runtime inference, or source corrections.
use super::argument_mismatch::{Index, positional_arguments};
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct SuperArgumentMismatch;
impl Cop for SuperArgumentMismatch {
    fn name(&self) -> &'static str {
        "Lint/SuperArgumentMismatch"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // Forcing selection/config/directives does not provide an index.
        self.check_with_index(None, node, cx, file, out);
    }
}

impl SuperArgumentMismatch {
    /// Private explicit indexed input. Normal production never self-indexes.
    fn check_with_index<'pr>(
        &self,
        index: Option<&Index<'pr>>,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(index) = index else { return };
        // Bare super is ForwardingSuperNode, not SuperNode.
        let Some(call) = node.as_super_node() else {
            return;
        };
        let enclosing: Vec<_> = cx.ancestors().collect();
        let mut ancestors = enclosing.iter().copied();
        let def = loop {
            let Some(ancestor) = ancestors.next() else {
                return;
            };
            if let Some(def) = ancestor.as_def_node() {
                if def.receiver().is_some() {
                    return;
                }
                break def;
            }
            if ancestor.as_singleton_class_node().is_some() {
                return;
            }
            // RuboCop's block wraps its send too: arguments/receiver of an
            // eval call with a literal block have that same block ancestor.
            if ancestor.as_call_node().is_some_and(|c| {
                c.block().is_some_and(|b| b.as_block_node().is_some())
                    && context_changing(c.name().as_slice())
            }) {
                return;
            }
        };
        // The nearest class/module/singleton/block boundary above the def must
        // be a class; a def in an ordinary block is not directly in its body.
        let Some(class) = ancestors.clone().find(|n| {
            matches!(
                n,
                Node::ClassNode { .. }
                    | Node::ModuleNode { .. }
                    | Node::SingletonClassNode { .. }
                    | Node::BlockNode { .. }
                    | Node::LambdaNode { .. }
            ) || n
                .as_call_node()
                .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()))
        }) else {
            return;
        };
        if class.as_class_node().is_none() {
            return;
        }
        let mut nesting = Vec::new();
        for ancestor in ancestors {
            let (path, body) = if let Some(c) = ancestor.as_class_node() {
                (c.constant_path(), c.body())
            } else if let Some(m) = ancestor.as_module_node() {
                (m.constant_path(), m.body())
            } else {
                continue;
            };
            if body.is_some_and(|b| {
                def.location().start_offset() >= b.location().start_offset()
                    && def.location().end_offset() <= b.location().end_offset()
            }) {
                let Some(name) = ast::const_name(&path) else {
                    return;
                };
                nesting.push(name);
            }
        }
        nesting.reverse();
        let method = String::from_utf8_lossy(def.name().as_slice());
        let Some((shape, parent)) = index.super_shape(class, &nesting.join("::"), &method) else {
            return;
        };
        let Some(given) = positional_arguments(call.arguments(), shape.keywords) else {
            return;
        };
        if given >= shape.min && shape.max.is_none_or(|max| given <= max) {
            return;
        }
        let expected = match shape.max {
            None => format!("{}+", shape.min),
            Some(max) if max == shape.min => max.to_string(),
            Some(max) => format!("{}..{max}", shape.min),
        };
        let loc = call.keyword_loc();
        let (line, col) = file.line_col(loc.start_offset());
        out.push(Offense {
            cop: self.name(), severity: self.severity(), correction: None,
            line, col, end_line: line,
            message: format!("Wrong number of arguments to `super` (given {given}, expected {expected} for `{parent}#{method}`)."),
        });
    }
}
fn context_changing(name: &[u8]) -> bool {
    matches!(
        name,
        b"define_method"
            | b"define_singleton_method"
            | b"class_eval"
            | b"class_exec"
            | b"module_eval"
            | b"module_exec"
            | b"instance_eval"
            | b"instance_exec"
            | b"refine"
    )
}

#[cfg(test)]
mod tests {
    //! RuboCop super_argument_mismatch_spec.rb examples: MIT License,
    //! Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::cops::walk::{NodeVisitor, walk};
    use std::{cell::RefCell, collections::HashSet};
    fn offenses(source: &str) -> Vec<Offense> {
        let file = SourceFile::new("example.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        assert!(parsed.errors().next().is_none(), "{source}");
        let index = Index::supplied(&parsed.node());
        indexed_offenses(&file, &parsed.node(), Some(&index))
    }
    fn indexed_offenses<'pr>(
        file: &SourceFile,
        node: &Node<'pr>,
        index: Option<&Index<'pr>>,
    ) -> Vec<Offense> {
        struct Visitor<'a, 'pr> {
            index: Option<&'a Index<'pr>>,
            file: &'a SourceFile,
            stack: Vec<Node<'pr>>,
            out: Vec<Offense>,
            reports: RefCell<HashSet<(usize, usize)>>,
        }
        impl<'pr> NodeVisitor<'pr> for Visitor<'_, 'pr> {
            fn enter(&mut self, node: Node<'pr>) {
                let cx = NodeContext {
                    ancestors: &self.stack,
                    corrections_requested: false,
                    require_order_reports: &self.reports,
                };
                SuperArgumentMismatch.check_with_index(
                    self.index,
                    &node,
                    &cx,
                    self.file,
                    &mut self.out,
                );
                self.stack.push(node);
            }
            fn leave(&mut self) {
                self.stack.pop();
            }
        }
        let mut visitor = Visitor {
            index,
            file,
            stack: Vec::new(),
            out: Vec::new(),
            reports: RefCell::default(),
        };
        walk(&mut visitor, node);
        visitor.out
    }
    #[test]
    fn no_index_and_unrelated_declaration_are_not_targets() {
        let source =
            "class Base; def f(a,b); end; end\nclass Child < Base; def f(a); super(a); end; end\n";
        let file = SourceFile::new("child.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(&file.bytes);
        let other = source.as_bytes().to_vec();
        let other_parse = ruby_prism::parse(&other);
        let wrong_index = Index::supplied(&other_parse.node());
        assert!(indexed_offenses(&file, &parsed.node(), None).is_empty());
        assert!(indexed_offenses(&file, &parsed.node(), Some(&wrong_index)).is_empty());
        assert_eq!(offenses(source).len(), 1);
    }
    #[test]
    fn matching_bare_forwarding_and_context_lookalikes() {
        for call in [
            "super(a,b)",
            "super",
            "super(*args)",
            "super(**kw)",
            "class_eval { super(a) }",
        ] {
            assert!(offenses(&format!("class Base; def f(a,b); end; end\nclass Child < Base; def f(a,b,*args,**kw); {call}; end; end\n")).is_empty(), "{call}");
        }
    }
    #[test]
    fn deep_flat_ancestry_and_repeated_super_are_bounded() {
        let mut source = String::from("class C0; def f(a,b); end; end\n");
        for i in 1..2500 {
            source.push_str(&format!("class C{i} < C{}; end\n", i - 1));
        }
        source.push_str("class C2500 < C2499; def f(a);\n");
        for _ in 0..100 {
            source.push_str("super(a)\n");
        }
        source.push_str("end; end\n");
        assert_eq!(offenses(&source).len(), 100);
        source.push_str("class C0 < Unknown; end\n");
        assert!(offenses(&source).is_empty());
    }
    #[test]
    fn all_indexed_reference_projections_and_controls() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/super_argument_mismatch.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let found: Vec<_> = offenses(row["source"].as_str().unwrap())
                .into_iter()
                .map(|o| {
                    assert!(o.correction.is_none());
                    assert_eq!(o.severity, crate::offense::Severity::Error);
                    (o.line, o.col, o.end_line, o.message)
                })
                .collect();
            let expected: Vec<(u32, u32, u32, String)> =
                serde_json::from_value(row["findings"].clone()).unwrap();
            assert_eq!(found, expected, "{row}");
        }
    }
    #[test]
    fn explicit_self_mixins_select_the_actual_super_target() {
        for (method, owner) in [("include", "Child"), ("prepend", "Base")] {
            for (params, parent_params, mismatch) in [("a", "a,b", false), ("a,b", "a", true)] {
                let source = format!(
                    "module M; def f({params}); end; end\nclass Base; def f({parent_params}); end; end\nclass {owner}; self.{method} M; end\nclass Child < Base; def f(a); super(a); end; end\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), usize::from(mismatch), "{source}");
                if mismatch {
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 31, 4));
                    assert_eq!(
                        found[0].message,
                        "Wrong number of arguments to `super` (given 1, expected 2 for `M#f`)."
                    );
                    assert!(found[0].correction.is_none());
                }
                assert!(
                    crate::pipeline::lint_str(&source)
                        .iter()
                        .all(|o| o.cop != "Lint/SuperArgumentMismatch")
                );
            }
        }
    }
    #[test]
    fn explicit_index_fixed_arity_and_keyword_location() {
        let source = "class Base\n  def run(a, b); end\nend\nclass Child < Base\n  def run(a)\n    super(a)\n  end\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (6, 5, 6));
        assert_eq!(
            found[0].message,
            "Wrong number of arguments to `super` (given 1, expected 2 for `Base#run`)."
        );
        assert!(found[0].correction.is_none());
        assert!(
            crate::pipeline::lint_str(source)
                .iter()
                .all(|o| o.cop != "Lint/SuperArgumentMismatch")
        );
    }
}
