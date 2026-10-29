//! `Style/RedundantConstantBase` — unnecessary root qualification outside namespaces.
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
#[derive(Default)]
pub struct RedundantConstantBase {
    constant_resolution_enabled: bool,
}
impl Cop for RedundantConstantBase {
    fn name(&self) -> &'static str {
        "Style/RedundantConstantBase"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn with_constant_resolution(&self, enabled: bool) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            constant_resolution_enabled: enabled,
        }))
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if self.constant_resolution_enabled {
            return;
        }
        let delimiter = if let Some(path) = node.as_constant_path_node() {
            if path.parent().is_some() {
                return;
            }
            path.delimiter_loc()
        } else if let Some(path) = node.as_constant_path_target_node() {
            if path.parent().is_some() {
                return;
            }
            path.delimiter_loc()
        } else {
            return;
        };
        let start = delimiter.start_offset();
        if cx.ancestors().any(|ancestor| {
            ancestor.as_module_node().is_some()
                || ancestor.as_class_node().is_some_and(|class| {
                    !class.superclass().is_some_and(|parent| {
                        parent.location().start_offset() <= start
                            && start < parent.location().end_offset()
                    })
                })
        }) {
            return;
        }
        let (line, col) = file.line_col(start);
        let correction = cx
            .corrections_requested()
            .then(|| {
                // Eigenclasses can bind their own constants, including on top-level
                // self. Keep the upstream diagnostic, but do not change that lookup.
                if cx
                    .ancestors()
                    .any(|n| n.as_singleton_class_node().is_some())
                {
                    return None;
                }
                // Qualified reads reject private constants; unqualified reads do not.
                // Scan the complete file, not just earlier statements: a method can
                // execute after a later visibility declaration has run.
                let root = cx.ancestors().next_back()?;
                if file.constant_base_visibility(root).uncertain {
                    return None;
                }
                // `return::A` and tightly adjacent ternaries need token separation.
                let separator = file.bytes.get(start.wrapping_sub(1)).is_some_and(|b| {
                    b.is_ascii_alphanumeric() || !b.is_ascii() || matches!(b, b'_' | b'?' | b'!')
                });
                Some(Correction {
                    range: start..delimiter.end_offset(),
                    replacement: if separator { " " } else { "" }.into(),
                })
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: line,
            message: "Remove redundant `::`.".into(),
            correction,
        });
    }
}
/// A lazy, file-wide visibility barrier. Receivers/arguments are deliberately
/// not inferred: Object aliases, splats, deferred bodies and reflection can all
/// make root constants private. Keep diagnostics when that proof is uncertain.
#[derive(Default)]
pub(crate) struct Visibility {
    uncertain: bool,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Visibility {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(&mut facts, root);
        facts
    }
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Visibility {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
        self.uncertain |= node.as_alias_method_node().is_some();
        if let Some(call) = node.as_call_node() {
            self.uncertain |= matches!(
                call.name().as_slice(),
                b"private_constant"
                    | b"send"
                    | b"__send__"
                    | b"public_send"
                    | b"method"
                    | b"public_method"
                    | b"instance_method"
                    | b"public_instance_method"
                    | b"eval"
                    | b"class_eval"
                    | b"module_eval"
                    | b"instance_eval"
                    | b"load"
                    | b"require"
                    | b"require_relative"
                    | b"autoload"
            );
        }
    }
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_constant_base_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        offense::Offense,
        pipeline::lint_str,
    };
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantConstantBase")
            .collect()
    }
    #[test]
    fn all_pinned_diagnostics_canonical_edits_and_one_eigenclass_hold() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_constant_base_pinned.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let before = case["source"].as_str().unwrap();
            let found = correction_offenses(&RedundantConstantBase::default(), before);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(serde_json::json!(positions), case["positions"], "{case}");
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(serde_json::json!(messages), case["messages"], "{case}");
            if let Some(after) = case["correction"].as_str() {
                if case["spec_line"] == 54 {
                    assert!(found[0].correction.is_none());
                } else {
                    assert_correction(&RedundantConstantBase::default(), before, after);
                }
            }
        }
    }
    #[test]
    fn pinned_root_and_superclass_diagnostics_and_corrections() {
        for (before, after, line, col) in [
            ("::Bar\n", "Bar\n", 1, 1),
            ("::Bar::Baz\n", "Bar::Baz\n", 1, 1),
            (
                "class Foo < ::Bar\n  ::A\nend\n",
                "class Foo < Bar\n  ::A\nend\n",
                1,
                13,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (line, col));
            assert_eq!(found[0].message, "Remove redundant `::`.");
            assert_correction(&RedundantConstantBase::default(), before, after);
        }
    }
    #[test]
    fn assignments_and_tightly_adjacent_keywords_preserve_binding() {
        for (before, after) in [
            ("::A = 1", "A = 1"),
            ("::A ||= 1", "A ||= 1"),
            ("::A, ::B = 1, 2", "A, B = 1, 2"),
            ("def f; return::A; end", "def f; return A; end"),
            ("defined?(::A)", "defined?(A)"),
            ("true ?::A : ::B", "true ? A : B"),
        ] {
            assert_correction(&RedundantConstantBase::default(), before, after);
        }
    }
    #[test]
    fn lexical_namespaces_and_nested_superclasses_are_not_redundant() {
        for source in [
            "class Foo\n ::Bar\nend",
            "module Foo\n ::Bar\nend",
            "module Foo\n class Baz < ::Bar\n end\nend",
            "class Foo\n def self.f; ::Bar; end\nend",
            "class Foo\n class << self; ::Bar; end\nend",
            "class ::Foo; end",
            "module ::Foo; end",
            "Bar::Baz",
            "object::Bar",
            "'::Bar' # ::Bar\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn private_constant_visibility_reports_without_edits() {
        for declaration in [
            "Object.private_constant(:A)",
            "Object.private_constant(*names)",
            "Object.send(:private_constant, :A)",
            "Object.public_send(selector, :A)",
            "Object.class_eval { private_constant :A }",
            "private_constant :A",
        ] {
            for access in ["::A", "defined?(::A)", "::A::B"] {
                let source = format!("A = 1\n{declaration}\np {access}\n");
                let found = correction_offenses(&RedundantConstantBase::default(), &source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(
                    (found[0].line, found[0].col),
                    (
                        3,
                        if access.starts_with("defined?") {
                            12
                        } else {
                            3
                        }
                    )
                );
                assert_eq!(found[0].message, "Remove redundant `::`.");
                assert!(found[0].correction.is_none(), "{source}");
            }
        }
        // Literal/comment spellings do not change constant visibility.
        assert_correction(
            &RedundantConstantBase::default(),
            "# private_constant :A\n'private_constant'\n::A\n",
            "# private_constant :A\n'private_constant'\nA\n",
        );
    }
    #[test]
    fn singleton_scope_reports_without_unsafe_edits() {
        for source in [
            "class << self\n ::Bar\nend",
            "class << Object.new\n ::Bar\nend",
        ] {
            let found = correction_offenses(&RedundantConstantBase::default(), source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}
