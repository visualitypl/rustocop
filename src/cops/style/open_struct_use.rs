//! `Style/OpenStructUse` — structural, unsafe suggestions only.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct OpenStructUse;
impl Cop for OpenStructUse {
    fn name(&self) -> &'static str {
        "Style/OpenStructUse"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !ast::is_global_const(node, &["OpenStruct"]) {
            return;
        }
        // Prism exposes the constant-path assignment target as a typed child;
        // Parser represents it as casgn, not a const use. RHS uses still count.
        if cx.parent().is_some_and(|p| {
            matches!(
                p,
                Node::ConstantPathWriteNode { .. }
                    | Node::ConstantPathOrWriteNode { .. }
                    | Node::ConstantPathAndWriteNode { .. }
                    | Node::ConstantPathOperatorWriteNode { .. }
            ) && p.location().start_offset() == node.location().start_offset()
        }) {
            return;
        }
        let declaration = cx.parent().and_then(|p| {
            p.as_class_node()
                .map(|c| c.constant_path())
                .or_else(|| p.as_module_node().map(|m| m.constant_path()))
        });
        if declaration
            .is_some_and(|n| n.location().start_offset() == node.location().start_offset())
        {
            return;
        }
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction: None,
            message:
                "Avoid using `OpenStruct`; use `Struct`, `Hash`, a class or test doubles instead."
                    .into(),
        });
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/open_struct_use_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::correction_offenses, pipeline::lint_str};
    #[test]
    fn pinned_bare_and_rooted_uses_have_exact_locations_and_no_fixes() {
        for klass in ["OpenStruct", "::OpenStruct"] {
            for (source, col) in [
                (format!("a = {klass}.new(a: 42)"), 5),
                (format!("class SubClass < {klass}; end"), 18),
                (format!("SubClass = Class.new({klass})"), 22),
                (klass.into(), 1),
            ] {
                let found: Vec<_> = lint_str(&source)
                    .into_iter()
                    .filter(|o| o.cop == OpenStructUse.name())
                    .collect();
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col, found[0].end_line),
                    (1, col, 1)
                );
                assert_eq!(
                    found[0].message,
                    "Avoid using `OpenStruct`; use `Struct`, `Hash`, a class or test doubles instead."
                );
                assert!(
                    correction_offenses(&OpenStructUse, &source)[0]
                        .correction
                        .is_none()
                );
            }
        }
    }
    #[test]
    fn ignores_custom_declarations_and_qualified_names_not_shadowed_uses() {
        for source in [
            "class A < SomeNamespace::OpenStruct; end",
            "a = SomeNamespace::OpenStruct.new",
            "module SomeNamespace; class OpenStruct; end; end",
            "module SomeNamespace; module OpenStruct; end; end",
            "class ::OpenStruct; end",
            "module ::OpenStruct; end",
            "class A < B; end",
            "a = 42",
            "OpenStruct = Object.new",
            "'OpenStruct' # OpenStruct\n",
            "::OpenStruct = 1",
            "::OpenStruct ||= 1",
            "::OpenStruct &&= 1",
            "::OpenStruct += 1",
            "::OpenStruct, x = values",
        ] {
            assert!(
                correction_offenses(&OpenStructUse, source).is_empty(),
                "{source}"
            );
        }
        for source in [
            "module M; class OpenStruct; end; OpenStruct.new; end",
            "class B; OpenStruct = Class.new; end; class C < B; OpenStruct.new; end",
            "class M::C; ::OpenStruct.new; end",
            "class C; end; class C; OpenStruct.new; end",
            "class << obj; OpenStruct.new; end",
            "OpenStruct::Nested",
        ] {
            assert_eq!(
                correction_offenses(&OpenStructUse, source).len(),
                1,
                "{source}"
            );
        }
    }
}
