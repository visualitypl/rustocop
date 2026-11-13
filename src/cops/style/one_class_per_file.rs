//! `Style/OneClassPerFile` — at most one immediate top-level declaration.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct OneClassPerFile;
impl Cop for OneClassPerFile {
    fn name(&self) -> &'static str {
        "Style/OneClassPerFile"
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
        let mut nodes: Vec<_> = program.statements().body().iter().collect();
        // Parser's root :begin is either the program's statement list, or
        // a sole parenthesized expression. Only its immediate children count;
        // explicit begin/end is :kwbegin, and nested parentheses stay nested.
        if let [only] = nodes.as_slice()
            && let Some(group) = only.as_parentheses_node()
        {
            nodes = group
                .body()
                .and_then(|body| body.as_statements_node())
                .map(|body| body.body().iter().collect())
                .unwrap_or_default();
        }
        let mut seen = false;
        for node in nodes {
            let name = if let Some(class) = node.as_class_node() {
                class.constant_path()
            } else if let Some(module) = node.as_module_node() {
                module.constant_path()
            } else {
                continue;
            };
            if !seen {
                seen = true;
                continue;
            }
            let (line, col) = file.line_col(node.location().start_offset());
            out.push(Offense {
                cop: self.name(),
                severity: self.severity(),
                message:
                    "Do not define multiple classes/modules at the top level in a single file."
                        .into(),
                line,
                col,
                end_line: file.last_line_of(name.location()),
                correction: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/one_class_per_file_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/OneClassPerFile")
            .collect()
    }
    #[test]
    fn flags_every_top_level_definition_after_the_first() {
        for first in ["class Foo", "module Foo"] {
            for second in [
                "class Bar",
                "module Bar",
                "class Foo::Bar",
                "module ::Bar",
                "class self.class::Bar",
                "class Foo",
            ] {
                let source = format!("{first}\nend\n\n{second}\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (4, 1, 4));
                assert_eq!(
                    found[0].message,
                    "Do not define multiple classes/modules at the top level in a single file."
                );
                assert!(found[0].correction.is_none());
            }
        }
        let found =
            offenses("require 'x'\nclass A; def a; end; end\nmodule B; end\nclass C; end\n");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(3, 1), (4, 1)]
        );
    }
    #[test]
    fn ignores_nested_singleton_and_dynamic_scope_definitions() {
        for source in [
            "",
            "require 'foo'\nrequire 'bar'",
            "class A; end",
            "module A; def a; end; end",
            "class A; class B; end; end",
            "module A; class B; end; module C; end; end",
            "class A; end; class << self; def foo; end; end",
            "class A; end; if cond; class B; end; class C; end; end",
            "class A; end; run { class B; end; class C; end }",
            "class A; end; class << self; class B; end; class C; end; end",
            "begin; class A; end; class B; end; end",
            "X = Class.new; Y = Module.new",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn preserves_parser_root_grouping_and_multiline_name_ranges() {
        let found = offenses("(class A; end; class B; end)");
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(1, 16)]
        );
        assert!(offenses("(class A; end); class B; end").is_empty());
        let found = offenses("class A; end\nclass Foo::\n Bar < Base\nend\n");
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 1, 3));
    }
    #[test]
    fn wide_file_reports_linearly_without_scanning_earlier_siblings_per_definition() {
        let source = "class A; end\n".repeat(2000);
        let found = offenses(&source);
        assert_eq!(found.len(), 1999);
        assert_eq!(
            (found.last().unwrap().line, found.last().unwrap().col),
            (2000, 1)
        );
    }
}
