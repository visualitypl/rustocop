//! `Lint/EmptyClass` — empty non-inheriting classes and singleton classes.
use ruby_prism::Node;

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct EmptyClass;

impl Cop for EmptyClass {
    fn name(&self) -> &'static str {
        "Lint/EmptyClass"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let message = if let Some(class) = node.as_class_node() {
            if class.body().is_some() || class.superclass().is_some() {
                return;
            }
            "Empty class detected."
        } else if let Some(class) = node.as_singleton_class_node() {
            if class.body().is_some() {
                return;
            }
            "Empty metaclass detected."
        } else {
            return;
        };
        // Fixed AllowComments=false: comments do not count as a body.
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: message.into(),
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's lint/empty_class_spec.rb.
    //! MIT License, Copyright (c) 2012-26 Bozhidar Batsov.
    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/EmptyClass")
            .collect()
    }

    #[test]
    fn flags_empty_classes_and_metaclasses_with_exact_messages() {
        for (source, line, col, end, message) in [
            ("class Foo\nend", 1, 1, 2, "Empty class detected."),
            (
                "class Foo\n  class << self\n  end\nend",
                2,
                3,
                3,
                "Empty metaclass detected.",
            ),
            (
                "  class << obj\n  end",
                1,
                3,
                2,
                "Empty metaclass detected.",
            ),
            ("class ::Foo::Bar; end", 1, 1, 1, "Empty class detected."),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end),
                "{source}"
            );
            assert_eq!(found[0].message, message);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn comments_do_not_make_a_class_nonempty() {
        for source in [
            "class Foo\n# Comment.\nend",
            "class Foo; end # Comment.",
            "class Foo\n  class << self\n    # Comment.\n  end\nend",
            "class << obj # Comment.\nend",
            "class Foo\n=begin\ncomment\n=end\nend",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
    }

    #[test]
    fn ignores_inherited_or_nonempty_classes_and_other_definitions() {
        for source in [
            "class Child < Parent\nend",
            "class Child < ::Object; end",
            "class Child < factory {}; end",
            "class Child < (work; Parent); end",
            "class Foo\n  attr_reader :bar\nend",
            "class Foo\n  class << self\n    attr_reader :bar\n  end\nend",
            "class << obj; nil; end",
            "class Foo; false; end",
            "class Foo; (); end",
            "class Foo; begin; end; end",
            "module Foo; end",
            "def foo; end",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn superclass_and_singleton_receiver_subtrees_are_still_checked() {
        let found =
            offenses("class Outer < (class Inner; end); end\nclass << (class Other; end); end\n");
        assert_eq!(
            found
                .iter()
                .map(|o| (o.line, o.col, o.message.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (1, 16, "Empty class detected."),
                (2, 1, "Empty metaclass detected."),
                (2, 11, "Empty class detected."),
            ]
        );
    }
}
