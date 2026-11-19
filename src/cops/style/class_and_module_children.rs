//! `Style/ClassAndModuleChildren`: fixed nested style; unsafe correction.

use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};

/// Enforce the fixed nested style.
pub struct ClassAndModuleChildren;

impl Cop for ClassAndModuleChildren {
    fn name(&self) -> &'static str {
        "Style/ClassAndModuleChildren"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let identifier = if let Some(class) = node.as_class_node() {
            class.constant_path()
        } else if let Some(module) = node.as_module_node() {
            module.constant_path()
        } else {
            return;
        };
        let Some(path) = identifier.as_constant_path_node() else {
            return;
        };
        // ::Foo alone has no namespace to split. ::Foo::Bar does.
        let Some(mut namespace) = path.parent() else {
            return;
        };
        // Only constant namespaces qualify, never dynamic expression::Foo.
        loop {
            if namespace.as_constant_read_node().is_some() {
                break;
            }
            let Some(path) = namespace.as_constant_path_node() else {
                return;
            };
            let Some(parent) = path.parent() else {
                break;
            };
            namespace = parent;
        }

        // Parser omits a single-expression StatementsNode. Multiple statements
        // form a :begin parent, so merely having a class/module ancestor does
        // NOT exempt the definition (e.g. after include/private/another class).
        let parent = cx.ancestors().find(|ancestor| {
            !ancestor
                .as_statements_node()
                .is_some_and(|statements| statements.body().len() == 1)
        });
        if parent.is_some_and(|parent| {
            matches!(parent, Node::ClassNode { .. } | Node::ModuleNode { .. })
        }) {
            return;
        }
        let location = identifier.location();
        let correction = cx
            .corrections_requested()
            .then(|| nest_definition(node, &path, cx, file))
            .flatten();
        let (line, col) = file.line_col(location.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use nested module/class definitions instead of compact style.".into(),
            line,
            col,
            end_line: file.last_line_of(location),
            correction,
        });
    }
}

/// Upstream's `nest_definition`: the keyword becomes the namespace's
/// (`class` when the previous sibling defines it as one, else `module`), the
/// last `::` starts a nested definition, and a second `end` closes it.
fn nest_definition(
    node: &Node<'_>,
    path: &ruby_prism::ConstantPathNode<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let (keyword, end) = match node.as_class_node() {
        Some(class) => (class.class_keyword_loc(), class.end_keyword_loc()),
        None => {
            let module = node.as_module_node()?;
            (module.module_keyword_loc(), module.end_keyword_loc())
        }
    };
    let namespace = path.parent()?;
    let namespace_keyword = if left_sibling_defines_class(node, &namespace, cx, file) {
        "class"
    } else {
        "module"
    };
    let start = node.location().start_offset();
    let line_start = file.line_start(file.line_of(start));
    let leading: String = file.bytes[line_start..]
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\x0b'))
        .map(|&b| b as char)
        .collect();
    let column = |offset: usize| file.line_col(offset).1 as usize - 1;
    // indentation(node) + leading_spaces(node), as upstream computes it.
    let padding = format!("{}  {leading}", " ".repeat(column(start)));
    let end_padding = padding.replacen(&" ".repeat(column(end.start_offset())), "", 1);
    let span = |l: &ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
    Correction::from_edits(
        &file.bytes,
        vec![
            (span(&keyword), namespace_keyword.to_owned()),
            (
                span(&path.delimiter_loc()),
                format!(
                    "\n{padding}{} ",
                    String::from_utf8_lossy(keyword.as_slice())
                ),
            ),
            (span(&end), format!("{end_padding}end\n{leading}end")),
        ],
    )
}

/// `heuristic_namespace_keyword`: whether the left sibling statement holds a
/// `class` definition of the namespace.
fn left_sibling_defines_class(
    node: &Node<'_>,
    namespace: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> bool {
    use crate::cops::walk::{NodeVisitor, walk};
    let Some(statements) = cx
        .parent()
        .and_then(|p| p.as_statements_node())
        .filter(|s| s.body().len() > 1)
    else {
        return false;
    };
    let body: Vec<_> = statements.body().iter().collect();
    let Some(index) = body
        .iter()
        .position(|n| n.location().start_offset() == node.location().start_offset())
    else {
        return false;
    };
    let Some(left) = index.checked_sub(1).map(|i| &body[i]) else {
        return false;
    };
    struct Finder<'a, 'b> {
        namespace: &'a Node<'b>,
        file: &'a SourceFile,
        found: bool,
    }
    impl<'pr> NodeVisitor<'pr> for Finder<'_, '_> {
        fn enter(&mut self, node: Node<'pr>) {
            if let Some(class) = node.as_class_node() {
                self.found |= ast::fingerprint_for_detection(&class.constant_path(), self.file)
                    == ast::fingerprint_for_detection(self.namespace, self.file);
            }
        }
    }
    let mut finder = Finder {
        namespace,
        file,
        found: false,
    };
    walk(&mut finder, left);
    finder.found
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's
    //! `spec/rubocop/cop/style/class_and_module_children_spec.rb`.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/ClassAndModuleChildren")
            .collect()
    }

    fn assert_offense(source: &str, line: u32, col: u32, end_line: u32) {
        let found = offenses(source);
        assert_eq!(found.len(), 1, "{source}");
        let offense = &found[0];
        assert_eq!(
            (offense.line, offense.col, offense.end_line),
            (line, col, end_line),
            "{source}"
        );
        assert_eq!(
            offense.message,
            "Use nested module/class definitions instead of compact style."
        );
        assert!(offense.correction.is_none());
    }

    #[test]
    fn flags_compact_class_and_module_names() {
        for (source, line, col) in [
            ("class FooClass::BarClass\nend\n", 1, 7),
            (
                "class FooClass\nend\n\nclass FooClass::BarClass\nend\n",
                4,
                7,
            ),
            (
                "module FooClass\nend\n\nclass FooClass::BarClass\nend\n",
                4,
                7,
            ),
            ("class FooClass::BarClass < Super\nend\n", 1, 7),
            ("module FooModule::BarModule\nend\n", 1, 8),
            ("class Foo::Bar\n  class Baz\n  end\nend\n", 1, 7),
            ("module Foo::Bar\n  module Baz\n  end\nend\n", 1, 8),
            (
                "# top comment\nclass Foo::Bar # describe Foo::Bar\nend\n",
                2,
                7,
            ),
            ("class ::FooClass::BarClass\nend\n", 1, 7),
            ("module ::Foo::Bar::Baz\nend\n", 1, 8),
            ("X =\nclass FooClass::BarClass\nend\n", 2, 7),
        ] {
            assert_offense(source, line, col, line);
        }
    }

    #[test]
    fn parent_exemption_is_immediate_not_lexically_enclosing() {
        for prefix in [
            "include M",
            "extend M",
            "private",
            "X = 1",
            "class Other; end",
        ] {
            for outer in ["class Z", "module Z"] {
                assert_offense(
                    &format!("{outer}\n  {prefix}\n  class X::Y\n  end\nend\n"),
                    3,
                    9,
                    3,
                );
                assert_offense(
                    &format!("{outer}\n  class X::Y\n  end\n  {prefix}\nend\n"),
                    2,
                    9,
                    2,
                );
            }
        }
        for (source, line, col) in [
            ("class << self\n  class X::Y\n  end\nend\n", 2, 9),
            (
                "module Z\n  class << self\n    module X::Y\n    end\n  end\nend\n",
                3,
                12,
            ),
            (
                "module Z\n  begin\n    class X::Y\n    end\n  end\nend\n",
                3,
                11,
            ),
            (
                "module Z\n  if condition\n    class X::Y\n    end\n  end\nend\n",
                3,
                11,
            ),
            (
                "module Z\n  wrap do\n    class X::Y\n    end\n  end\nend\n",
                3,
                11,
            ),
            ("module Z\n  X = class X::Y\n  end\nend\n", 2, 13),
        ] {
            assert_offense(source, line, col, line);
        }
        let reopened = offenses("class X::Y\nend\nclass X::Y\nend\n");
        assert_eq!(reopened.len(), 2);
        assert_eq!((reopened[0].line, reopened[0].col), (1, 7));
        assert_eq!((reopened[1].line, reopened[1].col), (3, 7));
        // Comments and empty semicolons do not create another AST statement.
        assert!(offenses("module Z\n  # comment\n  ;class X::Y\n  end\nend\n").is_empty());
        assert!(
            offenses("module Z\n  module X::Y\n    class A::B\n    end\n  end\nend\n").is_empty()
        );
    }

    #[test]
    fn multiline_names_report_only_the_name_range() {
        assert_offense("class Foo::\n  Bar < Parent::Base\n  value\nend\n", 1, 7, 2);
        assert_offense("module ::Foo::\n  Bar\nend\n", 1, 8, 2);
    }

    #[test]
    fn work_is_bounded_by_header_size_and_immediate_parent() {
        // Many sibling headers: no root walk, sibling scan, or namespace index.
        let source =
            "module Outer\n".to_owned() + &"  class A::B::C::D; end\n".repeat(500) + "end\n";
        let found = offenses(&source);
        assert_eq!(found.len(), 500);
        assert_eq!((found[0].line, found[0].col), (2, 9));
        assert_eq!((found[499].line, found[499].col), (501, 9));
        assert!(found.iter().all(|o| o.correction.is_none()));
    }

    #[test]
    fn accepts_nested_root_only_and_dynamic_namespaces() {
        for source in [
            "class FooClass\n  class BarClass\n  end\nend\nmodule FooModule\n  module BarModule\n  end\nend\n",
            "class ::Foo\nend\n",
            "module ::Foo\nend\n",
            "class self.class::Foo\nend\n",
            "module self.class::Foo\nend\n",
            "class self.class::Foo::Bar\nend\n",
            "module self.class::Foo::Bar\nend\n",
            "class FooClass\n  class BarClass\n  end\nend\nclass BazClass < FooClass::BarClass\nend\n",
            "module Z\n  module X::Y\n  end\nend\n",
            "class Z\n  module X::Y\n  end\nend\n",
            "class Z\n  class X::Y < Base\n  end\nend\n",
            "class (Foo)::Bar; end",
            "module namespace::Foo::Bar; end",
            "class << Foo::Bar; end",
            "# class Foo::Bar\ntext = 'module X::Y'\nFoo::Bar\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
