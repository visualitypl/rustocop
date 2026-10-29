//! `Lint/ConstantDefinitionInBlock` — lexical definitions directly in blocks.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct ConstantDefinitionInBlock;

impl Cop for ConstantDefinitionInBlock {
    fn name(&self) -> &'static str {
        "Lint/ConstantDefinitionInBlock"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !matches!(
            node,
            Node::ConstantWriteNode { .. } | Node::ClassNode { .. } | Node::ModuleNode { .. }
        ) || !direct_block_definition(cx)
        {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            correction: None,
            cop: self.name(),
            severity: self.severity(),
            message: "Do not define constants this way within a block.".into(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
        });
    }
}

/// Port `{^any_block [^begin ^^any_block]}`, not an arbitrary block ancestor
/// search. Prism always wraps bodies in StatementsNode; RuboCop only adds a
/// `begin` for multiple statements or parentheses. Explicit `begin ... end`
/// is `kwbegin`, and conditionals/classes/singleton classes are barriers.
fn direct_block_definition(cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors().peekable();
    let mut begin = false;
    while let Some(parent) = ancestors.next() {
        let adds_begin = match parent {
            Node::StatementsNode { .. } => {
                parent.as_statements_node().unwrap().body().len() > 1
                    && !ancestors
                        .peek()
                        .is_some_and(|n| n.as_parentheses_node().is_some())
            }
            Node::ParenthesesNode { .. } => true,
            Node::LambdaNode { .. } => return true,
            Node::BlockNode { .. } => {
                // In Prism the block is under its send. The fixed allowance
                // applies to that nearest block's method, with any receiver.
                return !ancestors
                    .next()
                    .and_then(Node::as_call_node)
                    .is_some_and(|call| call.name().as_slice() == b"enums");
            }
            _ => return false,
        };
        if adds_begin {
            if begin {
                return false;
            }
            begin = true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's constant_definition_in_block_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/ConstantDefinitionInBlock")
            .collect()
    }

    #[test]
    fn direct_definitions_with_single_or_multiple_statements() {
        for definition in ["FOO = 1", "class Foo; end", "module Foo; end"] {
            for tail in ["", "  bar\n", "  _1\n", "  it\n"] {
                let source = format!("describe do\n  {definition}\n{tail}end\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    "Do not define constants this way within a block."
                );
                assert!(found[0].correction.is_none());
            }
        }
    }

    #[test]
    fn top_level_qualified_and_allowed_method_lookalikes() {
        for definition in ["FOO = 1", "class Foo; end", "module Foo; end"] {
            for tail in ["", "bar\n"] {
                assert!(offenses(&format!("{definition}\n{tail}")).is_empty());
            }
            for call in ["enums", "other.enums", "other&.enums"] {
                assert!(offenses(&format!("{call} do\n {definition}\n it\nend")).is_empty());
            }
        }
        for definition in ["self::FOO = 1", "::FOO = 1", "Other::FOO = 1"] {
            assert!(offenses(&format!("describe do\n {definition}\n bar\nend")).is_empty());
        }
    }
}
