//! `Style/ColonMethodDefinition` — prefer `.` in singleton definitions.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ColonMethodDefinition;
impl Cop for ColonMethodDefinition {
    fn name(&self) -> &'static str {
        "Style/ColonMethodDefinition"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_def_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(definition) = node.as_def_node() else {
            return;
        };
        let Some(operator) = definition
            .operator_loc()
            .filter(|loc| loc.as_slice() == b"::")
        else {
            return;
        };
        let (line, col) = file.line_col(operator.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Do not use `::` for defining class methods.".into(),
            line,
            col,
            end_line: line,
            correction: cx.corrections_requested().then(|| Correction {
                range: operator.start_offset()..operator.end_offset(),
                replacement: ".".into(),
            }),
        });
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/colon_method_definition_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    #[test]
    fn pinned_singleton_definitions() {
        for (receiver, col) in [("self", 11), ("Foo", 10)] {
            let source = format!("class Foo\n  def {receiver}::bar\n    something\n  end\nend\n");
            let found: Vec<_> = lint_str(&source)
                .into_iter()
                .filter(|o| o.cop == "Style/ColonMethodDefinition")
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (2, col, 2)
            );
            assert_eq!(
                found[0].message,
                "Do not use `::` for defining class methods."
            );
            assert!(found[0].correction.is_none());
            assert_correction(&ColonMethodDefinition, &source, &source.replace("::", "."));
        }
    }
    #[test]
    fn singleton_receivers_parameters_endless_operators_and_setters() {
        for source in [
            "def (::Outer::Inner)::foo(a = effect, *rest, key:, **kw, &block)\n  yield a\nend\n",
            "def (receiver)::foo() = 42\n",
            "def self::[]=(key, value); end\n",
            "def self::+(other); other; end\n",
            "private_class_method def self::secret; 42; end\n",
            "def (receiver)::Foo; end\n",
            "def self:: # keep receiver separator comment\nfoo\n  __LINE__\nend\n",
        ] {
            let pos = source.rfind("::").unwrap();
            let mut after = source.to_owned();
            after.replace_range(pos..pos + 2, ".");
            assert_correction(&ColonMethodDefinition, source, &after);
        }
    }
    #[test]
    fn dot_and_ordinary_definitions_are_clean() {
        let source = "class Foo\n  def self.bar\n    something\n  end\n  def foo(a); a; end\nend\nFoo::bar\n";
        assert!(
            !lint_str(source)
                .iter()
                .any(|o| o.cop == "Style/ColonMethodDefinition")
        );
    }
}
