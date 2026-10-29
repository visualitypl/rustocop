//! `Layout/EmptyLinesAroundClassBody` — no padding inside class bodies.

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct EmptyLinesAroundClassBody;

impl Cop for EmptyLinesAroundClassBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundClassBody"
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
        let first = if let Some(class) = node.as_class_node() {
            class.superclass().map_or_else(
                || file.line_of(node.location().start_offset()),
                |parent| file.last_line_of(parent.location()),
            )
        } else if node.as_singleton_class_node().is_some() {
            file.line_of(node.location().start_offset())
        } else {
            return;
        };
        super::body_padding::check(
            self,
            "class",
            (first, file.last_line_of(node.location())),
            cx,
            file,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's layout/empty_lines_around_class_body_spec.rb at
    //! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == EmptyLinesAroundClassBody.name())
            .collect()
    }

    #[test]
    fn pinned_singleton_empty_and_multiline_superclass() {
        for header in [
            "class << self",
            "class SomeClass < Struct.new(\n  :attr,\n  keyword_init: true\n)",
        ] {
            let before = format!("{header}\n\n  do_something\n\nend\n");
            let found = offenses(&before);
            let first = header.lines().count() as u32;
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                [(first + 1, 1), (first + 3, 1)]
            );
            assert_correction(
                &EmptyLinesAroundClassBody,
                &before,
                &format!("{header}\n  do_something\nend\n"),
            );
        }
        let before = "class << self\n\nend\n";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "Extra empty line detected at class body beginning."
        );
        assert_correction(&EmptyLinesAroundClassBody, before, "class << self\nend\n");
    }

    #[test]
    fn namespace_nesting_and_adjacent_definitions() {
        let before =
            "class Parent\n\n  class Child\n\n    work\n\n  end\n\nend\n\nclass Next; end\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| o.line).collect::<Vec<_>>(),
            [2, 4, 6, 8]
        );
        assert_correction(
            &EmptyLinesAroundClassBody,
            before,
            "class Parent\n  class Child\n    work\n  end\nend\n\nclass Next; end\n",
        );
    }

    #[test]
    fn class_no_offense_lookalikes() {
        for source in [
            "",
            "class C; work; end\n\nnext_call\n",
            "class C\nend\n",
            "class << self\nend\n",
            "class C\n  \n  work\n \nend\n",
            "class C\n  # lead\n\n  work\n\n  # tail\nend\n",
            "class Parent::Child\n  class Inner; end\n\n  def foo; end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_class_padding_positions_and_correction() {
        let before = "class SomeClass\n\n  do_something\n\nend\n";
        let found = offenses(before);
        assert_eq!(found.len(), 2);
        for (o, line, location) in [(&found[0], 2, "beginning"), (&found[1], 4, "end")] {
            assert_eq!((o.line, o.col, o.end_line), (line, 1, line));
            assert_eq!(
                o.message,
                format!("Extra empty line detected at class body {location}.")
            );
            assert!(o.correction.is_none());
        }
        assert_correction(
            &EmptyLinesAroundClassBody,
            before,
            "class SomeClass\n  do_something\nend\n",
        );
    }
}
