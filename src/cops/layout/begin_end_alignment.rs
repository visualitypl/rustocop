//! `Layout/BeginEndAlignment` — align the `end` of `begin` with the start of
//! the line holding `begin` (fixed `EnforcedStyleAlignWith: start_of_line`).

use ruby_prism::Node;

use super::alignment::{display_column, end_indentation, misaligned_end, start_line_range};
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct BeginEndAlignment;

impl Cop for BeginEndAlignment {
    fn name(&self) -> &'static str {
        "Layout/BeginEndAlignment"
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
        // Only explicit `begin` (kwbegin); Prism also uses BeginNode for the
        // implicit bodies of `def ... rescue`, which have no keyword.
        let Some(begin) = node.as_begin_node() else {
            return;
        };
        let (Some(keyword), Some(end)) = (begin.begin_keyword_loc(), begin.end_keyword_loc())
        else {
            return;
        };
        let anchor = start_line_range(file, keyword.start_offset());
        let Some(message) = misaligned_end(file, &anchor, end.start_offset()) else {
            return;
        };
        let (line, col) = file.line_col(end.start_offset());
        let correction = cx
            .corrections_requested()
            .then(|| {
                let root = cx.ancestors().next_back().unwrap_or(node);
                let indentation = " ".repeat(display_column(file, anchor.start));
                end_indentation(file, root, end.start_offset(), &indentation)
            })
            .flatten();
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

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/begin_end_alignment_spec.rb`
    //! and `spec/support/alignment_examples.rb` at 97f30c4
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every example is
    //! replayed at the fixed `start_of_line` style, including the ones the
    //! spec declares for the `begin` style (their fixed results are recorded
    //! in the pinned RuboCop reference run).

    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        offense::Severity,
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Layout/BeginEndAlignment")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    #[test]
    fn pinned_start_of_line_examples() {
        // aligned 'puts 1; begin', '', 'end' / aligned 'var = begin', '', 'end'
        for source in ["puts 1; begin \nend", "var = begin \nend", "begin \nend"] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        // misaligned "begin\n  end"
        let before = "begin\n  end\n";
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].end_line, found[0].col), (2, 2, 3));
        assert_eq!(found[0].severity, Severity::Convention);
        assert_eq!(
            found[0].message,
            "`end` at 2, 2 is not aligned with `begin` at 1, 0."
        );
        assert_correction(&BeginEndAlignment, before, "begin\nend\n");
        // misaligned "var << begin\n       end"
        let before = "var << begin\n       end\n";
        assert_eq!(
            positions(before),
            [(
                2,
                8,
                "`end` at 2, 7 is not aligned with `var << begin` at 1, 0.".into()
            )]
        );
        assert_correction(&BeginEndAlignment, before, "var << begin\nend\n");
    }

    #[test]
    fn begin_style_examples_replay_at_the_fixed_style() {
        // The spec's `begin`-style 'puts 1; begin ... end' with `end` under
        // `begin` is a start_of_line offense.
        let before = "puts 1; begin \n        end";
        assert_eq!(
            positions(before),
            [(
                2,
                9,
                "`end` at 2, 8 is not aligned with `puts 1; begin` at 1, 0.".into()
            )]
        );
        assert_correction(&BeginEndAlignment, before, "puts 1; begin \nend");
    }

    #[test]
    fn anchors_on_the_whole_begin_line_and_compares_display_columns() {
        for source in [
            "  foo ||= begin\n    bar\n  end\n",
            "x = begin; y; end\n",
            "begin end\n",
            "foo(begin\n  1\nend)\n",
            "begin\n  x\nend while y\n",
            "\tx = begin\n\tend\n",
            "x = [1,\n  begin\n    2\n  end]\n",
            "h = { a: begin\n  1\nend }\n",
            "日本 = begin\n  1\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (before, after, expected) in [
            (
                "foo ||= begin\n          bar\n        end\n",
                "foo ||= begin\n          bar\nend\n",
                (
                    3,
                    9,
                    "`end` at 3, 8 is not aligned with `foo ||= begin` at 1, 0.",
                ),
            ),
            (
                "  x = [1, begin   # note\n    2\n    end]\n",
                "  x = [1, begin   # note\n    2\n  end]\n",
                (
                    3,
                    5,
                    "`end` at 3, 4 is not aligned with `x = [1, begin   # note` at 1, 2.",
                ),
            ),
            (
                "def f\n  begin\n    x\n  rescue\n    y\n    end\nend\n",
                "def f\n  begin\n    x\n  rescue\n    y\n  end\nend\n",
                (6, 5, "`end` at 6, 4 is not aligned with `begin` at 2, 2."),
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].message.as_str()),
                expected
            );
            assert_correction(&BeginEndAlignment, before, after);
            assert!(offenses(after).is_empty(), "{after}");
        }
    }

    #[test]
    fn implicit_bodies_and_other_ends_are_not_owned() {
        for source in [
            "def f\n  x\nrescue\n  y\n    end\n",
            "def f\n  x\n  ensure\n  y\n    end\n",
            "if a\n  end\n",
            "foo do\n  end\n",
            "x = \"begin\n  end\"\n",
            "x = <<~H\n  begin\n    end\nH\n",
            "# begin\n  # end\n",
            "BEGIN {\n  }\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn inline_ends_are_reported_but_never_moved_to_a_new_line() {
        for before in ["begin\n  foo end\n", "x = begin\n  1; end\n"] {
            let found = correction_offenses(&BeginEndAlignment, before);
            assert_eq!(found.len(), 1, "{before}");
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn bom_crlf_tabs_and_literals_preserve_every_other_byte() {
        for (before, after) in [
            ("\u{feff}begin\r\n  end", "\u{feff}begin\r\nend"),
            ("x = begin # é\r\n\t end\r\n", "x = begin # é\r\nend\r\n"),
            ("  x = begin\nend\n", "  x = begin\n  end\n"),
            (
                "x = \"#{begin\n  1\n   end}\"\n",
                "x = \"#{begin\n  1\n   end}\"\n",
            ),
            (
                "x = <<~H\n  #{begin\n    1\n      end}\nH\n",
                "x = <<~H\n  #{begin\n    1\n      end}\nH\n",
            ),
        ] {
            assert_eq!(offenses(before).len(), 1, "{before}");
            assert_correction(&BeginEndAlignment, before, after);
        }
    }

    #[test]
    fn closed_observers_hold_the_edit_but_keep_the_finding() {
        for tail in [
            "p __FILE__",
            "p __dir__",
            "p $0",
            "p $PROGRAM_NAME",
            "p caller",
            "p caller_locations",
            "p method(:f).source_location",
            "SCRIPT_LINES__ = {}",
            "p DATA.read\n__END__\nx",
        ] {
            let source = format!("x = begin\n  1\n  end\n{tail}\n");
            let found = correction_offenses(&BeginEndAlignment, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for tail in ["p File.read('example.rb')", "p STDIN.read", "p DATA"] {
            let before = format!("x = begin\n  1\n  end\n{tail}\n");
            let after = format!("x = begin\n  1\nend\n{tail}\n");
            assert_correction(&BeginEndAlignment, &before, &after);
        }
    }

    #[test]
    fn both_directive_spellings_disable_the_finding() {
        for prefix in ["rustocop", "rubocop"] {
            for target in ["Layout/BeginEndAlignment", "Layout", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\nbegin\n  end\n"),
                    format!("begin\n  end # {prefix}:disable {target}\n"),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                    assert!(correction_offenses(&BeginEndAlignment, &source).is_empty());
                }
            }
        }
    }
}
