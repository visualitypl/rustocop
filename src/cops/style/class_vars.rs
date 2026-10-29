//! `Style/ClassVars` — report writes, not reads, of shared class state.

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Offense, single_line},
    source::SourceFile,
};
use ruby_prism::{Location, Node};

pub struct ClassVars;
impl Cop for ClassVars {
    fn name(&self) -> &'static str {
        "Style/ClassVars"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let location = if let Some(location) = write_name(node) {
            location
        } else if let Some(call) = node.as_call_node() {
            if call.name().as_slice() != b"class_variable_set" || call.is_safe_navigation() {
                return;
            }
            let Some(argument) = ast::call_arguments(&call).into_iter().next() else {
                return;
            };
            argument.location()
        } else {
            return;
        };
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Replace class var {} with a class instance var.",
                single_line(location.as_slice())
            ),
            line,
            col,
            end_line: file.last_line_of(location),
            correction: None,
        });
    }
}

fn write_name<'pr>(node: &Node<'pr>) -> Option<Location<'pr>> {
    macro_rules! writes {
        ($($access:ident),*) => { $(if let Some(n) = node.$access() { return Some(n.name_loc()); })* };
    }
    writes!(
        as_class_variable_write_node,
        as_class_variable_or_write_node,
        as_class_variable_and_write_node,
        as_class_variable_operator_write_node
    );
    node.as_class_variable_target_node().map(|n| n.location())
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/style/class_vars_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ClassVars")
            .collect()
    }

    #[test]
    fn flags_declarations_and_reflective_writes_at_the_name_or_first_argument() {
        for (source, line, col, name) in [
            ("class TestClass; @@test = 10; end", 1, 18, "@@test"),
            (
                "class TestClass\n  class_variable_set(:@@test, 2)\nend",
                2,
                22,
                ":@@test",
            ),
            (
                "class TestClass; end\nTestClass.class_variable_set(:@@test, 42)",
                2,
                30,
                ":@@test",
            ),
            ("obj.class_variable_set(name, value)", 1, 24, "name"),
            (
                "class_variable_set(\"@@#{name}\", value)",
                1,
                20,
                "\"@@#{name}\"",
            ),
            ("class_variable_set(*names)", 1, 20, "*names"),
            ("class_variable_set(&block)", 1, 20, "&block"),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (line, col), "{source}");
            assert_eq!(
                found[0].message,
                format!("Replace class var {name} with a class instance var.")
            );
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn flags_each_compound_or_destructured_write_once() {
        for source in [
            "@@a ||= 1",
            "@@a &&= 1",
            "@@a += 1",
            "@@a, @b = values",
            "(@@a, b), c = values",
            "for @@a in values; end",
            "class << object; @@a = 1; end",
            "module M; @@a = 1; end",
            "class C; def f; @@a = 1; end; end",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            let offset = source.find("@@a").unwrap();
            assert_eq!((found[0].line, found[0].col), (1, offset as u32 + 1));
            assert_eq!(
                found[0].message,
                "Replace class var @@a with a class instance var."
            );
        }
        assert_eq!(offenses("@@a, @@b = @@c, @@d").len(), 2);
    }

    #[test]
    fn ignores_reads_and_non_class_variable_lookalikes() {
        for source in [
            "@@test.test(20)",
            "obj&.class_variable_set(:@@test, 1)",
            "class_variable_set",
            "class_variable_set { :@@a }",
            "class_variable_get(:@@test)",
            "@a = 1",
            "a = '@@test = 1'",
            "# @@test = 1",
            "defined?(@@a)",
            "@@a[0] = 1",
            "@@a.value += 1",
            "def class_variable_set(name, value); end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
