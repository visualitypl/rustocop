//! `Style/RedundantConstantBase` — unnecessary root qualification outside namespaces.
use crate::{
    cops::{Companions, Cop, NodeContext},
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
    fn with_companions(&self, companions: &Companions) -> Option<Box<dyn Cop>> {
        Some(Box::new(Self {
            constant_resolution_enabled: companions.constant_resolution,
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
        let correction = cx.corrections_requested().then(|| Correction {
            range: start..delimiter.end_offset(),
            replacement: String::new(),
        });
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
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's style/redundant_constant_base_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::{cops::testing::assert_correction, offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/RedundantConstantBase")
            .collect()
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
}
