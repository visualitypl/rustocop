//! `Layout/CaseIndentation` — indent `when` and `in` as deep as `case`
//! (fixed `EnforcedStyle: case`, `IndentOneStep: false`).

use ruby_prism::{Location, Node};

use super::alignment::end_indentation;
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct CaseIndentation;

impl Cop for CaseIndentation {
    fn name(&self) -> &'static str {
        "Layout/CaseIndentation"
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
        let (case, clauses, branch) = match node {
            Node::CaseNode { .. } => {
                let case = node.as_case_node().unwrap();
                let clauses = case
                    .conditions()
                    .iter()
                    .filter_map(|c| c.as_when_node().map(|w| w.keyword_loc()));
                (case.case_keyword_loc(), clauses.collect::<Vec<_>>(), "when")
            }
            Node::CaseMatchNode { .. } => {
                let case = node.as_case_match_node().unwrap();
                let clauses = case
                    .conditions()
                    .iter()
                    .filter_map(|c| c.as_in_node().map(|i| i.in_loc()));
                (case.case_keyword_loc(), clauses.collect::<Vec<_>>(), "in")
            }
            _ => return,
        };
        // `single_line?`: the whole `case ... end` on one line.
        let location = node.location();
        if file.line_of(location.start_offset()) == file.last_line_of(location) {
            return;
        }
        // Character columns, as the parser gem's `column`; a leading BOM is
        // not a column.
        let base = file.line_col(case.start_offset()).1;
        for keyword in clauses {
            let (line, col) = file.line_col(keyword.start_offset());
            if col == base {
                continue;
            }
            out.push(Offense {
                correction: cx
                    .corrections_requested()
                    .then(|| indentation(file, cx, node, keyword, base))
                    .flatten(),
                cop: self.name(),
                severity: self.severity(),
                message: format!("Indent `{branch}` as deep as `case`."),
                line,
                end_line: line,
                col,
            });
        }
    }
}

/// Upstream replaces the whitespace between the line start and the clause
/// keyword with `case`'s column in spaces; a clause after code on its line
/// keeps its bytes. The shared seam rewrites only a space/tab prefix and
/// withholds edits inside literal text and for the closed observer list.
fn indentation<'pr>(
    file: &SourceFile,
    cx: &NodeContext<'_, 'pr>,
    node: &Node<'pr>,
    keyword: Location<'pr>,
    base: u32,
) -> Option<crate::offense::Correction> {
    let root = cx.ancestors().next_back().unwrap_or(node);
    end_indentation(
        file,
        root,
        keyword.start_offset(),
        &" ".repeat(base as usize - 1),
    )
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/case_indentation_spec.rb`
    //! at 97f30c4 (MIT License, Copyright (c) 2012-26 Bozhidar Batsov). Every
    //! example, including the ones the spec declares for `IndentOneStep: true`,
    //! `EnforcedStyle: end` or tab indentation, is replayed at the fixed
    //! `case`/no-step/spaces defaults; the results match the fixed replay
    //! recorded in the pinned RuboCop reference run.

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
            .filter(|o| o.cop == "Layout/CaseIndentation")
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    const WHEN: &str = "Indent `when` as deep as `case`.";
    const IN: &str = "Indent `in` as deep as `case`.";

    fn at(list: &[(u32, u32)], message: &str) -> Vec<(u32, u32, String)> {
        list.iter()
            .map(|&(l, c)| (l, c, message.to_owned()))
            .collect()
    }

    /// Accepted at the fixed defaults.
    const ACCEPTED: &[&str] = &[
        "case foo; when :bar then 1; else 0; end\n",
        "case foo; in pattern then 1; else 0; end\n",
        "output = case variable\n         when 'value1'\n           'output1'\n         else\n           'output2'\n         end\n",
        "output = case variable\n         in pattern\n           'output1'\n         else\n           'output2'\n         end\n",
        "y = case a\n    when 0 then raise\n    when 0 then return\n    else\n      z = case b\n          when 1 then return\n          when 1 then raise\n          end\n    end\ncase c\nwhen 2 then encoding\nend\n",
        "y = case a\n    in 0 then raise\n    in 0 then return\n    else\n      z = case b\n          in 1 then return\n          in 1 then raise\n          end\n    end\ncase c\nin 2 then encoding\nend\n",
        "a = \"case\"\ncase x\nwhen 0\nend\n",
        "KEYWORDS = { :case => true, :when => true }\ncase type\nwhen 0\n  ParameterNode\nwhen 1\n  MethodCallNode\nend\n",
        "KEYWORDS = { :case => true, :in => true }\ncase type\nin 0\n  ParameterNode\nin 1\n  MethodCallNode\nend\n",
        "each {\n  case state\n  when 0\n    case name\n    when :a\n    end\n  when 1\n    loop {\n      case name\n      when :b\n      end\n    }\n  end\n}\ncase s\nwhen Array\nend\n",
        "each {\n  case state\n  in 0\n    case name\n    in :a\n    end\n  in 1\n    loop {\n      case name\n      in :b\n      end\n    }\n  end\n}\ncase s\nin Array\nend\n",
        // `EnforcedStyle: end` single-line-ish lookalikes: `when` under `case`.
        "case variable\nwhen 'value1'\nwhen 'value2'\nelse 'value3' end\n",
        "case variable\nwhen 'value1' then 'then1'\nwhen 'value2' then 'then2' end\n",
        "case variable\nin pattern then 'output1'\nin pattern then 'output2' end\n",
        // The tab examples: `when` one tab deep under a tab-indented `case`.
        "\tcase a\n\twhen b\n\t\tc\n\tend\n",
        "case a\nwhen b\n  c\nend\n",
    ];

    #[test]
    fn pinned_accepted_examples() {
        for source in ACCEPTED {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn pinned_assignment_examples_report_and_correct_only_the_when() {
        for (keyword, message) in [("when 'value1'", WHEN), ("in pattern", IN)] {
            // indented as end
            let before =
                format!("output = case variable\n{keyword}\n  'output1'\nelse\n  'output2'\nend\n");
            assert_eq!(positions(&before), at(&[(2, 1)], message));
            assert_correction(
                &CaseIndentation,
                &before,
                &format!(
                    "output = case variable\n         {keyword}\n  'output1'\nelse\n  'output2'\nend\n"
                ),
            );
            // indented some other way
            let before = format!(
                "output = case variable\n  {keyword}\n    'output1'\n  else\n    'output2'\nend\n"
            );
            assert_eq!(positions(&before), at(&[(2, 3)], message));
            assert_correction(
                &CaseIndentation,
                &before,
                &format!(
                    "output = case variable\n         {keyword}\n    'output1'\n  else\n    'output2'\nend\n"
                ),
            );
            // correct + opposite style
            let good = format!(
                "output = case variable\n         {keyword}\n           'output1'\n         else\n           'output2'\n         end\n"
            );
            let before = format!(
                "{good}output = case variable\n{keyword}\n  'output1'\nelse\n  'output2'\nend\n"
            );
            assert_eq!(positions(&before), at(&[(8, 1)], message));
            assert_correction(
                &CaseIndentation,
                &before,
                &format!(
                    "{good}output = case variable\n         {keyword}\n  'output1'\nelse\n  'output2'\nend\n"
                ),
            );
        }
    }

    #[test]
    fn pinned_deeper_clauses_are_moved_to_the_case_column() {
        for (keyword, message) in [("when", WHEN), ("in", IN)] {
            let before = format!(
                "case a\n    {keyword} 0 then return\n    else\n        case b\n         {keyword} 1 then return\n        end\nend\n"
            );
            let found = offenses(&before);
            assert_eq!(found[0].severity, Severity::Convention);
            assert_eq!(found[0].end_line, 2);
            assert_eq!(positions(&before), at(&[(2, 5), (5, 10)], message));
            assert_correction(
                &CaseIndentation,
                &before,
                &format!(
                    "case a\n{keyword} 0 then return\n    else\n        case b\n        {keyword} 1 then return\n        end\nend\n"
                ),
            );
        }
    }

    #[test]
    fn indent_one_step_examples_replay_at_the_fixed_style() {
        for (keyword, message) in [("when 'value1'", WHEN), ("in pattern", IN)] {
            // 'accepts a correctly indented assignment' (one step) is an offense.
            let before = format!(
                "output = case variable\n           {keyword}\n             'output1'\n           else\n             'output2'\n         end\n"
            );
            assert_eq!(positions(&before), at(&[(2, 12)], message));
            assert_correction(
                &CaseIndentation,
                &before,
                &format!(
                    "output = case variable\n         {keyword}\n             'output1'\n           else\n             'output2'\n         end\n"
                ),
            );
            // IndentationWidth 5 lookalike.
            let before = format!(
                "output = case variable\n              {keyword}\n             'output1'\n              else\n             'output2'\n         end\n"
            );
            assert_eq!(positions(&before), at(&[(2, 15)], message));
        }
        for (keyword, message) in [("when", WHEN), ("in", IN)] {
            // 2 spaces deeper than `case` (both levels).
            let before = format!(
                "case a\n  {keyword} 0 then return\n  else\n        case b\n          {keyword} 1 then return\n        end\nend\n"
            );
            assert_eq!(positions(&before), at(&[(2, 3), (5, 11)], message));
            assert_correction(
                &CaseIndentation,
                &before,
                &format!(
                    "case a\n{keyword} 0 then return\n  else\n        case b\n        {keyword} 1 then return\n        end\nend\n"
                ),
            );
        }
    }

    #[test]
    fn end_style_examples_replay_at_the_fixed_style() {
        for (keyword, message) in [("when 'value1'", WHEN), ("in pattern", IN)] {
            // `when` aligned with `end` and one step past `end`.
            for (indent, col) in ["", "  "].iter().zip([1, 3]) {
                let before = format!(
                    "output = case variable\n{indent}{keyword}\n  'output1'\nelse\n  'output2'\nend\n"
                );
                assert_eq!(positions(&before), at(&[(2, col)], message));
            }
            // One step past a deeper `end` (5 columns).
            let before = format!(
                "output = case variable\n       {keyword}\n         'output1'\n       else\n         'output2'\n       end\n"
            );
            assert_eq!(positions(&before), at(&[(2, 8)], message));
            assert_correction(
                &CaseIndentation,
                &before,
                &format!(
                    "output = case variable\n         {keyword}\n         'output1'\n       else\n         'output2'\n       end\n"
                ),
            );
        }
    }

    #[test]
    fn when_on_the_case_line_is_reported_without_an_edit() {
        let source = "case test when something\nend\n";
        assert_eq!(positions(source), at(&[(1, 11)], WHEN));
        let found = correction_offenses(&CaseIndentation, source);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn tab_examples_use_space_indentation_at_the_fixed_style() {
        // `\tcase a\n\t\twhen b`: char columns 1 and 2.
        let before = "\tcase a\n\t\twhen b\n\t\t\tc\n\tend\n";
        assert_eq!(positions(before), at(&[(2, 3)], WHEN));
        assert_correction(
            &CaseIndentation,
            before,
            "\tcase a\n when b\n\t\t\tc\n\tend\n",
        );
        // A non-leading `case` is still a space-indentation target.
        let before = "a = case b\n  when c\n    d\nend\n";
        assert_eq!(positions(before), at(&[(2, 3)], WHEN));
        assert_correction(
            &CaseIndentation,
            before,
            "a = case b\n    when c\n    d\nend\n",
        );
        // `\tx = case a` with `when` four spaces deep: case column 5.
        let before = "\tx = case a\n    when b\n\t\t\tc\n\tend\n";
        assert_eq!(positions(before), at(&[(2, 5)], WHEN));
        assert!(
            correction_offenses(&CaseIndentation, before)[0]
                .correction
                .is_some()
        );
        let before = "\tx = case a\n\t\twhen b\n\t\t\tc\n\tend\n";
        assert_eq!(positions(before), at(&[(2, 3)], WHEN));
        assert_correction(
            &CaseIndentation,
            before,
            "\tx = case a\n     when b\n\t\t\tc\n\tend\n",
        );
        let before = "case a\n\twhen b\n  c\nend\n";
        assert_eq!(positions(before), at(&[(2, 2)], WHEN));
        assert_correction(&CaseIndentation, before, "case a\nwhen b\n  c\nend\n");
    }

    #[test]
    fn columns_are_characters_and_bom_is_not_a_column() {
        for source in [
            "é = case x\n    when 1\n    end\n",
            "\u{feff}case x\nwhen 1\nend\n",
            "x = case y\n    in [a, b] if a\n    in {k:}\n    end\n",
            "case\nwhen a\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        let before = "é = case x\n  when 1\n  end\n";
        assert_eq!(positions(before), at(&[(2, 3)], WHEN));
        assert_correction(&CaseIndentation, before, "é = case x\n    when 1\n  end\n");
        let before = "\u{feff}case x\r\n  when 1\r\nend\r\n";
        assert_eq!(positions(before), at(&[(2, 3)], WHEN));
        assert_correction(
            &CaseIndentation,
            before,
            "\u{feff}case x\r\nwhen 1\r\nend\r\n",
        );
    }

    #[test]
    fn inline_clauses_and_literals_keep_their_bytes() {
        // A clause after code on its line is reported without an edit.
        let source = "case x\nwhen 1 then a; when 2 then b\nend\n";
        assert_eq!(positions(source), at(&[(2, 16)], WHEN));
        assert!(
            correction_offenses(&CaseIndentation, source)[0]
                .correction
                .is_none()
        );
        // Inside heredoc interpolation the clause keeps its bytes.
        let source = "x = <<~H\n  #{case y\n      when 1 then 2\n    end}\nH\n";
        let found = correction_offenses(&CaseIndentation, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        // Keywords in strings, symbols and comments are not clauses.
        for source in [
            "x = \"case y\n  when 1\nend\"\n",
            "# case y\n  # when 1\n",
            "x = %w[case\n  when]\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
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
            let source = format!("case x\n  when 1\nend\n{tail}\n");
            let found = correction_offenses(&CaseIndentation, &source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
        for tail in [
            "p File.read('example.rb')",
            "p STDIN.read",
            "p DATA",
            "system('cat x.rb')",
        ] {
            assert_correction(
                &CaseIndentation,
                &format!("case x\n  when 1\nend\n{tail}\n"),
                &format!("case x\nwhen 1\nend\n{tail}\n"),
            );
        }
    }

    #[test]
    fn both_directive_spellings_disable_the_finding() {
        for prefix in ["rustocop", "rubocop"] {
            for target in ["Layout/CaseIndentation", "Layout", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\ncase x\n  when 1\nend\n"),
                    format!("case x\n  when 1 # {prefix}:disable {target}\nend\n"),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                    assert!(correction_offenses(&CaseIndentation, &source).is_empty());
                }
            }
        }
    }
}
