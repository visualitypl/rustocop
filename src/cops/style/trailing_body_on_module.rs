//! `Style/TrailingBodyOnModule` — start multiline module bodies below the header.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct TrailingBodyOnModule;
impl Cop for TrailingBodyOnModule {
    fn name(&self) -> &'static str {
        "Style/TrailingBodyOnModule"
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
        let Some(module) = node.as_module_node() else {
            return;
        };
        super::trailing_body_on_class::check_body(
            self,
            node,
            module.body(),
            module.constant_path().location().end_offset(),
            cx,
            file,
            out,
        );
    }
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's style/trailing_body_on_module_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TrailingBodyOnModule")
            .collect()
    }
    #[test]
    fn pinned_positions_messages_and_corrections() {
        for (before, after, line, col) in [
            (
                "module Foo body\nend\n",
                "module Foo \n  body\nend\n",
                1,
                12,
            ),
            (
                "module Bar extend self\nend\n",
                "module Bar \n  extend self\nend\n",
                1,
                12,
            ),
            (
                "module Bar; def bar; end\nend\n",
                "module Bar \n  def bar; end\nend\n",
                1,
                13,
            ),
            (
                "module Bar def bar; end\nend\n",
                "module Bar \n  def bar; end\nend\n",
                1,
                12,
            ),
            (
                "module Foo body\n  def bar\n    qux\n  end\nend\n",
                "module Foo \n  body\n  def bar\n    qux\n  end\nend\n",
                1,
                12,
            ),
            (
                "module Foo; do_stuff\nend\n",
                "module Foo \n  do_stuff\nend\n",
                1,
                13,
            ),
            (
                "module BarQux; foo # comment\nend\n",
                "# comment\nmodule BarQux \n  foo \nend\n",
                1,
                16,
            ),
            (
                "\nmodule Foo; body \nend\n",
                "\nmodule Foo \n  body \nend\n",
                2,
                13,
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, line)
            );
            assert_eq!(
                found[0].message,
                "Place the first line of module body on its own line."
            );
            assert_correction(&TrailingBodyOnModule, before, after);
        }
    }
    #[test]
    fn qualified_names_rescue_and_multiline_first_statements() {
        for (before, after, last) in [
            ("module A::B; foo\nend\n", "module A::B \n  foo\nend\n", 1),
            (
                "module ::C; def f\n  1\nend\nend\n",
                "module ::C \n  def f\n  1\nend\nend\n",
                3,
            ),
            (
                "module C; foo\nensure\nbar\nend\n",
                "module C \n  foo\nensure\nbar\nend\n",
                3,
            ),
        ] {
            assert_eq!(offenses(before)[0].end_line, last);
            assert_correction(&TrailingBodyOnModule, before, after);
        }
    }
    #[test]
    fn uncertain_newlines_and_comment_relocations_remain_reported() {
        for source in [
            "module C; <<~TXT\npayload\nTXT\nend\n",
            "module C; foo\nend\np __LINE__\n",
            "module C; foo # rubocop:disable Layout\nend\n",
            "module C; foo # rustocop:disable Layout\nend\n",
            "module C; foo # encoding: UTF-8\nend\n",
            "x = module C; foo\nend\n",
            "module C; foo\nend\np errors.map(&:full_message)\n",
            "module C; foo\nend\np Object.const_get(:DATA).lineno\n__END__\nraw\n",
        ] {
            let found = crate::cops::testing::correction_offenses(&TrailingBodyOnModule, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
    #[test]
    fn accepts_regular_empty_and_single_line() {
        for source in [
            "module Foo\n  def no_op; end\nend",
            "module Foo; body; end",
            "module Foo\nend",
            "module Foo\n  extend self\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
