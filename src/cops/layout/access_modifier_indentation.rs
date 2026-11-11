//! `Layout/AccessModifierIndentation` — fixed two-space indent style.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct AccessModifierIndentation;
impl Cop for AccessModifierIndentation {
    fn name(&self) -> &'static str {
        "Layout/AccessModifierIndentation"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if !super::indentation_width::bare_access(node, cx.ancestors()) {
            return;
        }
        let mut ancestors = cx.ancestors();
        let Some(statements) = ancestors.next().and_then(Node::as_statements_node) else {
            return;
        };
        // RuboCop checks only a begin body, not a single expression, nor sends
        // nested inside control flow/rescue/explicit kwbegin wrappers.
        let mut owner = ancestors.next();
        let grouped = owner.is_some_and(|n| n.as_parentheses_node().is_some());
        if statements.body().len() < 2 && !grouped {
            return;
        }
        if grouped {
            owner = ancestors.next();
            if let Some(outer) = owner.and_then(Node::as_statements_node) {
                // With siblings, the grouped begin is a child of the owner's
                // begin body: its sends are not direct body children upstream.
                if outer.body().len() != 1 {
                    return;
                }
                owner = ancestors.next();
            }
        }
        let Some(owner) = owner else {
            return;
        };
        // Prism represents ordinary, numbered and `it` blocks with the same
        // BlockNode/LambdaNode shape. RuboCop dispatches the latter two as
        // numblock/itblock, for which this cop has no callback. Inspect only
        // this effective owner: an outer implicit block must not suppress a
        // nested class/module/ordinary block's real callback.
        let parameters = if let Some(block) = owner.as_block_node() {
            block.parameters()
        } else if let Some(lambda) = owner.as_lambda_node() {
            lambda.parameters()
        } else {
            None
        };
        if matches!(
            parameters,
            Some(Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. })
        ) {
            return;
        }
        let end = match owner {
            Node::ClassNode { .. } => owner.as_class_node().unwrap().end_keyword_loc(),
            Node::ModuleNode { .. } => owner.as_module_node().unwrap().end_keyword_loc(),
            Node::SingletonClassNode { .. } => {
                owner.as_singleton_class_node().unwrap().end_keyword_loc()
            }
            Node::BlockNode { .. } => owner.as_block_node().unwrap().closing_loc(),
            Node::LambdaNode { .. } => owner.as_lambda_node().unwrap().closing_loc(),
            _ => return,
        };
        let location = node.location();
        let owner_start = if owner.as_block_node().is_some() {
            ancestors
                .next()
                .map_or(owner.location().start_offset(), |n| {
                    n.location().start_offset()
                })
        } else {
            owner.location().start_offset()
        };
        if file.line_of(owner_start) == file.line_of(location.start_offset()) {
            return;
        }
        // RangeHelp uses effective character columns here, not display width
        // (notably a tab counts one). AssignmentIndentation uses display width.
        let expected = super::alignment::effective_column(file, end.start_offset()) as i64
            + super::alignment::INDENT as i64;
        let delta =
            expected - super::alignment::effective_column(file, location.start_offset()) as i64;
        if delta == 0 {
            return;
        }
        let alignment = super::alignment::Alignment {
            expression: location.start_offset()..location.end_offset(),
            delta,
        };
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            correction: cx
                .corrections_requested()
                .then(|| alignment.correction(file, cx.ancestors().next_back().unwrap_or(node)))
                .flatten(),
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Indent access modifiers like `{}`.",
                String::from_utf8_lossy(location.as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(location),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's access_modifier_indentation_spec.rb at 97f30c4
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == AccessModifierIndentation.name())
            .collect()
    }
    #[test]
    fn pinned_indent_bodies_and_modifiers() {
        for (opening, closing) in [
            ("class Test", "end"),
            ("module Test", "end"),
            ("class << self", "end"),
            ("Test = Class.new do", "end"),
            ("Test = Module.new do", "end"),
            ("func do", "end"),
        ] {
            for name in ["private", "protected", "public", "module_function"] {
                let before = format!("{opening}\n\n{name}\n\n  def test; end\n{closing}\n");
                let after = before.replacen(&format!("\n{name}"), &format!("\n  {name}"), 1);
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
                assert_eq!(
                    found[0].message,
                    format!("Indent access modifiers like `{name}`.")
                );
                assert_correction(&AccessModifierIndentation, &before, &after);
                assert!(offenses(&after).is_empty());
            }
        }
    }
    #[test]
    fn implicit_block_kinds_have_no_callback_but_classic_blocks_do() {
        for opening in ["foo do", "foo {", "-> do", "-> {"] {
            let closing = if opening.ends_with('{') { "}" } else { "end" };
            for parameter in ["_1", "it"] {
                let source = format!("{opening}\nprivate\n  {parameter}\n{closing}\n");
                assert!(offenses(&source).is_empty(), "{source}");
                assert!(
                    crate::cops::testing::correction_offenses(&AccessModifierIndentation, &source,)
                        .is_empty()
                );
            }
            for explicit in [false, true] {
                let header = if !explicit {
                    opening.to_string()
                } else if opening.starts_with("->") {
                    opening.replacen("->", "->(arg)", 1)
                } else {
                    format!("{opening} |arg|")
                };
                let source = format!("{header}\nprivate\n  1\n{closing}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_eq!(found[0].message, "Indent access modifiers like `private`.");
                assert_correction(
                    &AccessModifierIndentation,
                    &source,
                    &source.replacen("\nprivate", "\n  private", 1),
                );
            }
        }
    }
    #[test]
    fn nested_classic_owners_inside_implicit_blocks_keep_their_callbacks() {
        for parameter in ["_1", "it"] {
            for opening in [
                "class Inner",
                "module Inner",
                "class << self",
                "bar do",
                "-> do",
            ] {
                let source = format!(
                    "foo do\nprivate\n  {opening}\nprotected\n    1\n  end\n  {parameter}\nend\n"
                );
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}: {found:?}");
                assert_eq!((found[0].line, found[0].col), (4, 1));
                assert_eq!(
                    found[0].message,
                    "Indent access modifiers like `protected`."
                );
                assert_correction(
                    &AccessModifierIndentation,
                    &source,
                    &source.replacen("\nprotected", "\n    protected", 1),
                );
            }
        }
    }
    #[test]
    fn pinned_lookalikes_and_exact_begin_body_requirement() {
        for source in [
            "class A; private; def foo; end; end\n",
            "class A\nprivate\nend\n",
            "module A\n  private :foo\nend\n",
            "class A\n  private def foo; end\nend\n",
            "class A\n  self.private\n  def foo; end\nend\n",
            "private\ndef foo; end\n",
            "def foo\nprivate\nbar\nend\n",
            "class A\n  if x\nprivate\nfoo\n  end\nend\n",
            "class A\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
