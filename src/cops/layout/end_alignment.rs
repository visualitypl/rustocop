//! `Layout/EndAlignment` — align closing `end` with its opening keyword.

use ruby_prism::{Location, Node};

use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};

pub struct EndAlignment;

impl Cop for EndAlignment {
    fn name(&self) -> &'static str {
        "Layout/EndAlignment"
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
        let Some((keyword, end)) = keywords(node) else {
            return;
        };
        let (align_line, align_col) = file.line_col(keyword.start_offset());
        let (line, col) = file.line_col(end.start_offset());
        if line == align_line || col == align_col {
            return;
        }
        let correction = cx.corrections_requested().then(|| {
            super::alignment::align_end(
                file,
                end.start_offset(),
                &" ".repeat((align_col - 1) as usize),
            )
        });
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "`end` at {line}, {} is not aligned with `{}` at {align_line}, {}.",
                col - 1,
                String::from_utf8_lossy(keyword.as_slice()),
                align_col - 1,
            ),
            line,
            end_line: line,
            col,
        });
    }
}

/// Only constructs owned by EndAlignment. Prism shares an if chain's closing
/// location with its elsif nodes; the opening `if` alone owns that end.
fn keywords<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Location<'pr>)> {
    Some(match node {
        Node::ClassNode { .. } => {
            let n = node.as_class_node().unwrap();
            (n.class_keyword_loc(), n.end_keyword_loc())
        }
        Node::SingletonClassNode { .. } => {
            let n = node.as_singleton_class_node().unwrap();
            (n.class_keyword_loc(), n.end_keyword_loc())
        }
        Node::ModuleNode { .. } => {
            let n = node.as_module_node().unwrap();
            (n.module_keyword_loc(), n.end_keyword_loc())
        }
        Node::IfNode { .. } => {
            let n = node.as_if_node().unwrap();
            let keyword = n.if_keyword_loc()?;
            if keyword.as_slice() != b"if" {
                return None;
            }
            (keyword, n.end_keyword_loc()?)
        }
        Node::UnlessNode { .. } => {
            let n = node.as_unless_node().unwrap();
            (n.keyword_loc(), n.end_keyword_loc()?)
        }
        Node::WhileNode { .. } => {
            let n = node.as_while_node().unwrap();
            (n.keyword_loc(), n.closing_loc()?)
        }
        Node::UntilNode { .. } => {
            let n = node.as_until_node().unwrap();
            (n.keyword_loc(), n.closing_loc()?)
        }
        Node::CaseNode { .. } => {
            let n = node.as_case_node().unwrap();
            (n.case_keyword_loc(), n.end_keyword_loc())
        }
        Node::CaseMatchNode { .. } => {
            let n = node.as_case_match_node().unwrap();
            (n.case_keyword_loc(), n.end_keyword_loc())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `spec/rubocop/cop/layout/end_alignment_spec.rb`
    //! and `spec/support/alignment_examples.rb` at 97f30c4
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        offense::{Offense, Severity},
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
            .filter(|o| o.cop == "Layout/EndAlignment")
            .collect()
    }

    #[test]
    fn pinned_assignments_arguments_and_chained_calls_align_with_keyword() {
        for (prefix, opening) in [
            ("var = ", "if test"),
            ("@var = ", "unless test"),
            ("@@var = ", "while test"),
            ("$var = ", "until test"),
            ("CNST = ", "if test"),
            ("a, b = ", "if test"),
            ("var ||= ", "if test"),
            ("var &&= ", "unless test"),
            ("var += ", "if test"),
            ("h[k] = ", "case a when b"),
            ("h.k = ", "case a; in b"),
            ("var << ", "until test"),
            ("test ", "case a when b"),
            ("test ", "case a; in b"),
            ("var = ", "class << self"),
        ] {
            for suffix in [
                "",
                ".ab.join(\"\")",
                "&.ab",
                ".tap { _1 }",
                " || x",
                " && x",
            ] {
                let before = format!("{prefix}{opening}\nend{suffix}\n");
                let after = format!(
                    "{prefix}{opening}\n{}end{suffix}\n",
                    " ".repeat(prefix.len())
                );
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                assert_eq!((found[0].line, found[0].col), (2, 1));
                assert_correction(&EndAlignment, &before, &after);
                assert!(offenses(&after).is_empty(), "{after}");
            }
        }
        for (before, after) in [
            ("var =\n  if test\nend\n", "var =\n  if test\n  end\n"),
            ("foo((if test\nend).ab)\n", "foo((if test\n     end).ab)\n"),
            (
                "x = y || case a\nwhen b\n  end\n",
                "x = y || case a\nwhen b\n         end\n",
            ),
        ] {
            assert_correction(&EndAlignment, before, after);
            assert!(offenses(after).is_empty());
        }
    }

    #[test]
    fn elsif_chains_have_one_closing_end_owned_by_initial_if() {
        let before =
            "if a\n  if b\n    x\n  elsif c\n    y\n    end\nelsif d\n  z\nelse\n  q\n  end\n";
        let after = "if a\n  if b\n    x\n  elsif c\n    y\n  end\nelsif d\n  z\nelse\n  q\nend\n";
        let found = offenses(before);
        assert_eq!(
            found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
            [(6, 5), (11, 3)]
        );
        assert_eq!(
            found[0].message,
            "`end` at 6, 4 is not aligned with `if` at 2, 2."
        );
        assert_eq!(
            found[1].message,
            "`end` at 11, 2 is not aligned with `if` at 1, 0."
        );
        assert_correction(&EndAlignment, before, after);
        assert!(offenses(after).is_empty());
    }

    #[test]
    fn ignores_one_line_postfix_ternary_and_ends_owned_by_other_cops() {
        for source in [
            "",
            "\n",
            "# end\n",
            "__END__\n  if a\nend\n",
            "class A; end\nmodule B; end\nclass << self; end\n",
            "if a; b; elsif c; d; else; e; end\n",
            "unless a; end\nwhile a; end\nuntil a; end\n",
            "case a; when b; end\ncase a; in b; end\n",
            "a = cond ? x : y",
            "a = x if cond",
            "a unless cond",
            "a while cond",
            "a until cond",
            "begin\n x\n    end while cond\n",
            "def x\n  end\ndef self.x\n    end\n",
            "begin\nrescue\nensure\n  end\n",
            "foo do\n  end\n-> do\n  end\nfoo { }\n",
            "for a in b\n  end\n",
            "x = \"if a\\n  end\"\ny = /end/\nz = %w(if end)\n",
            "x = <<~H\nif a\n  end\nH\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn utf8_bom_crlf_tabs_and_missing_final_newline_preserve_every_other_byte() {
        for (before, after, position, message) in [
            (
                "\u{feff}class Test\r\n  end",
                "\u{feff}class Test\r\nend",
                (2, 3),
                "`end` at 2, 2 is not aligned with `class` at 1, 0.",
            ),
            (
                "é = if true\r\nend # 日本語\r\n",
                "é = if true\r\n    end # 日本語\r\n",
                (2, 1),
                "`end` at 2, 0 is not aligned with `if` at 1, 4.",
            ),
            (
                "  if a\n\tend\n",
                "  if a\n  end\n",
                (2, 2),
                "`end` at 2, 1 is not aligned with `if` at 1, 2.",
            ),
        ] {
            let found = offenses(before);
            assert_eq!(found.len(), 1);
            assert_eq!((found[0].line, found[0].col), position);
            assert_eq!(found[0].message, message);
            assert_correction(&EndAlignment, before, after);
        }
        let before = "class A\n  end\n__END__\r\n  end\n\traw";
        assert_correction(&EndAlignment, before, &before.replacen("  end", "end", 1));
    }

    #[test]
    fn both_directive_spellings_protect_end_indentation() {
        for prefix in ["rustocop", "rubocop"] {
            for target in ["Layout/EndAlignment", "EndAlignment", "Layout", "all"] {
                for source in [
                    format!("# {prefix}:disable {target}\nif a\n  end\n"),
                    format!("if a\n  end # {prefix}:disable {target}\n"),
                    format!("# {prefix}:disable-next {target}\nif a\n  end\n"),
                ] {
                    assert!(offenses(&source).is_empty(), "{source}");
                    assert!(correction_offenses(&EndAlignment, &source).is_empty());
                }
                let before = format!(
                    "# {prefix}:disable {target}\nif a\n  end\n# {prefix}:enable {target}\nif b\n  end\n"
                );
                let after = before.strip_suffix("  end\n").unwrap().to_owned() + "end\n";
                assert_eq!(offenses(&before).len(), 1);
                assert_correction(&EndAlignment, &before, &after);
            }
        }
    }

    #[test]
    fn pinned_constructs_report_exact_end_and_correct_only_its_indentation() {
        for (opening, keyword) in [
            ("class Test", "class"),
            ("class << self", "class"),
            ("module Test", "module"),
            ("if test", "if"),
            ("unless test", "unless"),
            ("while test", "while"),
            ("until test", "until"),
            ("case a when b", "case"),
            ("case a; in b", "case"),
        ] {
            for prefix in ["", "puts 1; "] {
                let before = format!("{prefix}{opening}\n  end # keep\n");
                let after = format!(
                    "{prefix}{opening}\n{}end # keep\n",
                    " ".repeat(prefix.len())
                );
                let found = offenses(&before);
                assert_eq!(found.len(), 1, "{before}");
                let o = &found[0];
                assert_eq!((o.line, o.end_line, o.col), (2, 2, 3));
                assert_eq!(o.severity, Severity::Warning);
                assert_eq!(
                    o.message,
                    format!(
                        "`end` at 2, 2 is not aligned with `{keyword}` at 1, {}.",
                        prefix.len()
                    )
                );
                assert!(o.correction.is_none());
                assert_correction(&EndAlignment, &before, &after);
                assert!(offenses(&after).is_empty());
            }
        }
    }
}
