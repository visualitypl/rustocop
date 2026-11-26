//! `Layout/DefEndAlignment` — align the `end` of a method definition with the
//! start of its line (fixed `EnforcedStyleAlignWith: start_of_line`).

use ruby_prism::{CallNode, Node};

use super::alignment::{display_column, misaligned_end};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct DefEndAlignment;

impl Cop for DefEndAlignment {
    fn name(&self) -> &'static str {
        "Layout/DefEndAlignment"
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
        let Some(def) = node.as_def_node() else {
            return;
        };
        // Endless definitions have no `end`.
        let Some(end) = def.end_keyword_loc() else {
            return;
        };
        let keyword = def.def_keyword_loc();
        let chain = modifier_chain(node, cx);
        let anchor = match chain.last() {
            // `on_send` for the outermost `def` modifier: a single-line `def`
            // has nothing to align, even after a line continuation.
            Some(_)
                if file.line_of(node.location().start_offset())
                    == file.line_of(end.start_offset()) =>
            {
                return;
            }
            Some(outer) => outer.location().start_offset()..keyword.end_offset(),
            None => keyword.start_offset()..keyword.end_offset(),
        };
        let Some(message) = misaligned_end(file, &anchor, end.start_offset()) else {
            return;
        };
        // Upstream aligns `end` with the def's parent send, or else the def.
        // That is the reported anchor except for nested modifiers or a def
        // passed to an ordinary call, where the upstream edit would not
        // converge; those keep the finding without an edit.
        let target = chain
            .first()
            .map(|call| call.location().start_offset())
            .or_else(|| parent_send_start(node, cx))
            .unwrap_or(node.location().start_offset());
        let (line, col) = file.line_col(end.start_offset());
        let correction = (cx.corrections_requested()
            && file.line_col(target).1 == file.line_col(anchor.start).1)
            .then(|| {
                let indentation = " ".repeat(display_column(file, anchor.start));
                super::alignment::align_end(file, end.start_offset(), &indentation)
            });
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            end_line: line,
            col,
        });
    }
}

fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    let (a, b) = (a.location(), b.location());
    (a.start_offset(), a.end_offset()) == (b.start_offset(), b.end_offset())
}

/// rubocop-ast `def_modifier`: receiverless sends whose first argument is the
/// definition or the next send of the chain (`private def`, `foo bar def`).
/// Innermost first. A send with a literal block is wrapped in a block node by
/// the parser gem, so the chain cannot continue above it.
fn modifier_chain<'pr>(node: &Node<'pr>, cx: &NodeContext<'_, 'pr>) -> Vec<CallNode<'pr>> {
    let mut chain: Vec<CallNode<'pr>> = Vec::new();
    let mut child = node.location();
    let mut ancestors = cx.ancestors();
    while let Some(arguments) = ancestors.next().and_then(|n| n.as_arguments_node()) {
        if !arguments.arguments().iter().next().is_some_and(|first| {
            let l = first.location();
            (l.start_offset(), l.end_offset()) == (child.start_offset(), child.end_offset())
        }) {
            break;
        }
        let Some(call) = ancestors.next().and_then(|n| n.as_call_node()) else {
            break;
        };
        if call.receiver().is_some() {
            break;
        }
        child = call.location();
        let has_block = call.block().is_some_and(|b| b.as_block_node().is_some());
        chain.push(call);
        if has_block {
            break;
        }
    }
    chain
}

/// The start of the definition's parser-gem parent when that is a `send`
/// (not `csend`): a call taking the definition as receiver or argument.
fn parent_send_start<'pr>(node: &Node<'pr>, cx: &NodeContext<'_, 'pr>) -> Option<usize> {
    let mut ancestors = cx.ancestors();
    let parent = ancestors.next()?;
    let call = if parent.as_arguments_node().is_some() {
        ancestors.next()?.as_call_node()?
    } else {
        let call = parent.as_call_node()?;
        if !call.receiver().is_some_and(|r| same(&r, node)) {
            return None;
        }
        call
    };
    (!call.is_safe_navigation()).then(|| call.location().start_offset())
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/def_end_alignment_spec.rb`
    //! and `spec/support/alignment_examples.rb` at 97f30c4
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every example is
    //! replayed at the fixed `start_of_line` style, including the ones the
    //! spec declares for the `def` style (their fixed results are recorded in
    //! the pinned RuboCop reference run).

    use super::*;
    use crate::{cops::testing::assert_correction, offense::Severity, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/DefEndAlignment")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    #[test]
    fn pinned_misaligned_def_and_defs() {
        for (before, after) in [
            ("def test\n  end\n", "def test\nend\n"),
            ("def Test.test\n  end\n", "def Test.test\nend\n"),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].end_line, found[0].col), (2, 2, 3));
            assert_eq!(found[0].severity, Severity::Warning);
            assert_eq!(
                found[0].message,
                "`end` at 2, 2 is not aligned with `def` at 1, 0."
            );
            assert_correction(&DefEndAlignment, before, after);
        }
    }

    #[test]
    fn pinned_aligned_definitions_and_modifiers() {
        for source in [
            "\u{feff}def test\nend",
            "def test\nend",
            "def Test.test\nend",
            "foo def test\nend",
            "foo bar def test\nend",
            // The `def` style's misaligned example is aligned at start_of_line.
            "foo def test\nend\n",
            "using Module.new {\n  refine Hash do\n    class << Hash\n      private def _ruby2_keywords_hash(*args)\n      end\n    end\n  end\n}\n",
            "include Module.new {\n  private def foo\n  end\n}\n",
            "foo \\\n  def test; end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_modifier_misalignment_and_correct_plus_opposite() {
        // misaligned "foo def test\n    end", also the `def` style's aligned
        // 'foo def', 'test', '    end' example.
        for before in ["foo def test\n    end\n", "foo def test\n    end"] {
            assert_eq!(
                positions(before),
                [(
                    2,
                    5,
                    "`end` at 2, 4 is not aligned with `foo def` at 1, 0.".into()
                )]
            );
            assert_correction(&DefEndAlignment, before, &before.replace("    end", "end"));
        }
        let before = "foo def a\n  a1\nend\n\nfoo def b\n      b1\n    end\n";
        assert_eq!(
            positions(before),
            [(
                7,
                5,
                "`end` at 7, 4 is not aligned with `foo def` at 5, 0.".into()
            )]
        );
        assert_correction(
            &DefEndAlignment,
            before,
            "foo def a\n  a1\nend\n\nfoo def b\n      b1\nend\n",
        );
        // The `def` style's correct + opposite source.
        let before = "foo def a\n  a1\nend\nfoo def b\n      b1\n    end\n";
        assert_eq!(
            positions(before),
            [(
                6,
                5,
                "`end` at 6, 4 is not aligned with `foo def` at 4, 0.".into()
            )]
        );
        assert_correction(
            &DefEndAlignment,
            before,
            "foo def a\n  a1\nend\nfoo def b\n      b1\nend\n",
        );
    }

    #[test]
    fn modifier_chains_follow_receiverless_first_arguments() {
        for source in [
            "private(def f\nend)\n",
            "  protected def f\n    x\n  end\n",
            "private def f\n  x\nrescue\n  y\nend\n",
            "private def f = 1\n",
            "private def f =\n  1\n",
            "private def f; end\n",
            "foo(def x\nend) { }\n",
            "class A\n  def f\n  end\n\n  def self.g(a)\n    a\n  end\nend\n",
            "x = def f\n    end\n",
            "def f\n  def g\n  end\nend\n",
            "def é; end\ndef f\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (before, expected) in [
            ("private def f\n  end\n", (2, 3, "`private def` at 1, 0")),
            ("x = def f\n  end\n", (2, 3, "`def` at 1, 4")),
            ("foo 1, def x\nend\n", (2, 1, "`def` at 1, 7")),
            ("foo.bar def x\nend\n", (2, 1, "`def` at 1, 8")),
            ("foo bar def x\n  end\n", (2, 3, "`foo bar def` at 1, 0")),
            (
                "foo \\\n  def x\n  end\n",
                (3, 3, "`foo \\\n  def` at 1, 0"),
            ),
            ("def f\n  def g\n    end\nend\n", (3, 5, "`def` at 2, 2")),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (expected.0, expected.1));
            assert!(
                found[0].message.ends_with(&format!("{}.", expected.2)),
                "{}",
                found[0].message
            );
        }
    }
}
