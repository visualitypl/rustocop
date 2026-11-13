//! `Layout/EmptyLinesAroundModuleBody` — no padding inside module bodies.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;

pub struct EmptyLinesAroundModuleBody;
impl Cop for EmptyLinesAroundModuleBody {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundModuleBody"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_module_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if node.as_module_node().is_none() {
            return;
        }
        super::body_padding::check(
            self,
            "module",
            (
                file.line_of(node.location().start_offset()),
                file.last_line_of(node.location()),
            ),
            cx,
            file,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's layout/empty_lines_around_module_body_spec.rb at
    //! 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == EmptyLinesAroundModuleBody.name())
            .collect()
    }
    #[test]
    fn namespace_nesting_and_adjacent_definitions() {
        let before =
            "module Parent\n\n  module Child\n\n    work\n\n  end\n\nend\n\nmodule Next; end\n";
        assert_eq!(
            offenses(before).iter().map(|o| o.line).collect::<Vec<_>>(),
            [2, 4, 6, 8]
        );
        assert_correction(
            &EmptyLinesAroundModuleBody,
            before,
            "module Parent\n  module Child\n    work\n  end\nend\n\nmodule Next; end\n",
        );
    }

    #[test]
    fn module_no_offense_lookalikes() {
        for source in [
            "",
            "module M; work; end\n\nnext_call\n",
            "module M\nend\n",
            "module M\n  \n  work\n \nend\n",
            "module M\n  # lead\n\n  work\n\n  # tail\nend\n",
            "module Parent::Child\n  module Inner; end\n\n  def foo; end\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_module_padding_positions_and_correction() {
        let before = "module SomeModule\n\n  do_something\n\nend\n";
        let found = offenses(before);
        assert_eq!(found.len(), 2);
        for (o, line, edge) in [(&found[0], 2, "beginning"), (&found[1], 4, "end")] {
            assert_eq!((o.line, o.col, o.end_line), (line, 1, line));
            assert_eq!(
                o.message,
                format!("Extra empty line detected at module body {edge}.")
            );
            assert!(o.correction.is_none());
        }
        assert_correction(
            &EmptyLinesAroundModuleBody,
            before,
            "module SomeModule\n  do_something\nend\n",
        );
    }
}
