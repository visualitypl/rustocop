//! `Layout/EmptyLineBetweenDefs` — separate consecutive definitions.

use ruby_prism::Node;

use crate::{
    cops::{
        Cop, NodeContext,
        style::correction::{line_value_boundary, touches_directive},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct EmptyLineBetweenDefs;
impl Cop for EmptyLineBetweenDefs {
    fn name(&self) -> &'static str {
        "Layout/EmptyLineBetweenDefs"
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
        let Some(statements) = node.as_statements_node() else {
            return;
        };
        // A handler-free explicit begin maps to RuboCop's `kwbegin`, not
        // the `begin` statement groups this cop checks. Rescue/ensure bodies
        // still contain a `begin` group, as do nested scopes in either case.
        if cx.parent().and_then(Node::as_begin_node).is_some_and(|n| {
            n.begin_keyword_loc().is_some()
                && n.rescue_clause().is_none()
                && n.ensure_clause().is_none()
        }) {
            return;
        }
        let root = cx.ancestors().next_back().unwrap_or(node);
        let mut previous = None;
        for current in statements.body().iter() {
            if let Some(prev) = previous.as_ref()
                && let Some(kind) = definition_kind(&current)
                && definition_kind(prev).is_some()
            {
                let first = file.definition_end_line(root, prev);
                let last = file.line_of(current.location().start_offset());
                let mut count = 0;
                let mut nonblank_seen = false;
                let mut multiple_groups = false;
                for line in first + 1..last {
                    if blank(file.line_text(line)) {
                        count += 1;
                        multiple_groups |= nonblank_seen;
                    } else {
                        nonblank_seen = true;
                    }
                }
                let one_line = |n: &Node<'_>| {
                    file.line_of(n.location().start_offset()) == file.last_line_of(n.location())
                };
                if count != 1 && !multiple_groups && !(one_line(prev) && one_line(&current)) {
                    let (line, col) = file.line_col(current.location().start_offset());
                    out.push(Offense {
                        correction: cx
                            .corrections_requested()
                            // Physical lines inside heredoc interpolation can
                            // affect squiggly dedentation. Keep detection, but
                            // conservatively leave the complete value alone.
                            .then_some(())
                            .filter(|_| {
                                !cx.ancestors().any(|ancestor| {
                                    ancestor
                                        .as_interpolated_string_node()
                                        .and_then(|n| n.opening_loc())
                                        .or_else(|| {
                                            ancestor
                                                .as_interpolated_x_string_node()
                                                .map(|n| n.opening_loc())
                                        })
                                        .is_some_and(|loc| loc.as_slice().starts_with(b"<<"))
                                })
                            })
                            .and_then(|_| correction(file, root, first, &current, count)),
                        cop: self.name(),
                        severity: self.severity(),
                        message: format!(
                            "Expected 1 empty line between {kind} definitions; found {count}."
                        ),
                        line,
                        end_line: header_end_line(file, &current),
                        col,
                    });
                }
            }
            previous = Some(current);
        }
    }
}

fn definition_kind(node: &Node<'_>) -> Option<&'static str> {
    match node {
        Node::DefNode { .. } => Some("method"),
        Node::ClassNode { .. } => Some("class"),
        Node::ModuleNode { .. } => Some("module"),
        _ => None,
    }
}

fn header_end_line(file: &SourceFile, node: &Node<'_>) -> u32 {
    let loc = if let Some(n) = node.as_def_node() {
        n.name_loc()
    } else if let Some(n) = node.as_class_node() {
        n.constant_path().location()
    } else {
        node.as_module_node().unwrap().constant_path().location()
    };
    file.last_line_of(loc)
}

fn blank(text: &[u8]) -> bool {
    text.iter()
        .all(|&b| super::brace_spacing::is_ruby_whitespace(b))
}

fn correction(
    file: &SourceFile,
    root: &Node<'_>,
    first: u32,
    current: &Node<'_>,
    count: u32,
) -> Option<Correction> {
    let last = file.line_of(current.location().start_offset());
    // Heredoc bodies may physically follow a subsequent definition on the
    // opener line. There is no safe ordinary separator at this boundary.
    if first > last {
        return None;
    }
    let same_line = first == last;
    let start = if same_line {
        current.location().start_offset()
    } else {
        file.line_start(first + 1)
    };
    if start < line_value_boundary(file)
        || file.line_text(first).ends_with(b"\\")
        || touches_directive(file, file.line_start(first)..file.line_start(last + 1))
    {
        return None;
    }
    let range = if count > 1 {
        start..file.line_start(first + count)
    } else {
        start..start
    };
    // Zero-width insertion still needs a nonempty literal-safety query. At
    // a heredoc terminator's end, inspect the following code, not its value.
    let safety_range = range.start..range.end.max(range.start + 1);
    if file.overlaps_literal(root, &safety_range) {
        return None;
    }
    let replacement = if count > 1 {
        String::new()
    } else {
        let newline = if file.bytes[..file.line_start(first + 1)].ends_with(b"\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        newline.repeat(if same_line { 2 } else { 1 })
    };
    Some(Correction { range, replacement })
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's layout/empty_line_between_defs_spec.rb at 97f30c4
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::assert_correction,
        offense::{Offense, Severity},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        assert_eq!(
            ruby_prism::parse(source.as_bytes()).errors().count(),
            0,
            "{source:?}"
        );
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == EmptyLineBetweenDefs.name())
            .collect()
    }

    #[test]
    fn pinned_definition_kinds_positions_messages_and_corrections() {
        for (header, kind) in [
            ("def bar", "method"),
            ("def self.bar", "method"),
            ("class B", "class"),
            ("module B", "module"),
        ] {
            let before = format!("def foo\nend\n{header}\nend\n");
            let after = format!("def foo\nend\n\n{header}\nend\n");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (3, 1, 3));
            assert_eq!(found[0].severity, Severity::Convention);
            assert_eq!(
                found[0].message,
                format!("Expected 1 empty line between {kind} definitions; found 0.")
            );
            assert!(found[0].correction.is_none());
            assert_correction(&EmptyLineBetweenDefs, &before, &after);
            assert!(offenses(&after).is_empty());
        }
    }

    #[test]
    fn pinned_comments_stay_with_the_following_definition() {
        let before = "class J\n  def n\n  end # n-related\n  # checks something o-related\n  # and more\n  def o\n  end\nend\n";
        let after = before.replace("end # n-related\n", "end # n-related\n\n");
        let found = offenses(before);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (6, 3));
        assert_correction(&EmptyLineBetweenDefs, before, &after);
    }

    #[test]
    fn explicit_begin_without_handlers_has_no_offenses_or_corrections() {
        for newline in ["\n", "\r\n"] {
            for header in ["def a", "class A", "module A"] {
                for gap in ["", "\n\n"] {
                    let source = format!("begin\n  {header}\n  end\n{gap}  def b\n  end\nend\n")
                        .replace('\n', newline);
                    assert!(offenses(&source).is_empty(), "{source}");
                    let found =
                        crate::cops::testing::correction_offenses(&EmptyLineBetweenDefs, &source);
                    assert!(found.is_empty(), "{source}");
                    let applied = crate::corrections::apply(source.as_bytes(), &found).unwrap();
                    assert_eq!(applied.bytes, source.as_bytes());
                }
            }
        }
    }

    #[test]
    fn explicit_begin_still_checks_nested_scopes() {
        for scope in ["class C", "module M", "def outer"] {
            let before =
                format!("begin\n  {scope}\n    def a\n    end\n    def b\n    end\n  end\nend\n");
            let after = before.replace("    end\n    def b", "    end\n\n    def b");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (5, 5));
            assert_correction(&EmptyLineBetweenDefs, &before, &after);
        }
    }

    #[test]
    fn pinned_nested_and_conditional_siblings() {
        for (open, close) in [
            ("class K", "end"),
            ("module M", "end"),
            ("class << self", "end"),
            ("def outer", "end"),
            ("if condition", "else\n  def a\n  end\nend"),
            ("Class.new do", "end"),
            ("begin", "rescue\n  recover\nend"),
            ("begin", "ensure\n  cleanup\nend"),
        ] {
            let before = format!("{open}\n  def a\n  end\n  def b\n  end\n{close}\n");
            let after = before.replace("  end\n  def b", "  end\n\n  def b");
            let found = offenses(&before);
            assert_eq!(found.len(), 1, "{before}");
            assert_eq!((found[0].line, found[0].col), (4, 3));
            assert_correction(&EmptyLineBetweenDefs, &before, &after);
        }
    }

    #[test]
    fn pinned_one_line_allowance_and_excess_blank_lines() {
        for source in [
            "def a; end\ndef b; end\n",
            "def a = x\ndef b = y\n",
            "class A; end\nclass B; end\n",
            "module A; end; module B; end\n",
            "def a; end\n\n\n\ndef b; end\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for count in [0, 2, 3] {
            let before = format!("def a\nend\n{}def b; end\n", "\n".repeat(count));
            let found = offenses(&before);
            assert_eq!(found.len(), 1);
            assert_eq!(
                found[0].message,
                format!("Expected 1 empty line between method definitions; found {count}.")
            );
            assert_correction(&EmptyLineBetweenDefs, &before, "def a\nend\n\ndef b; end\n");
        }
    }

    #[test]
    fn pinned_endless_and_heredoc_methods() {
        for (first, second) in [
            ("def foo() = x", "def bar\n  y\nend"),
            ("def foo\n  x\nend", "def bar() = y"),
            ("def self.foo = x", "def self.bar\n  y\nend"),
            ("def foo = <<~TEXT\n  hello\nTEXT", "def bar\n  y\nend"),
            (
                "def foo = [<<A, <<B]\nhello\nA\nworld\nB",
                "def bar\n  y\nend",
            ),
        ] {
            let before = format!("{first}\n{second}\n");
            let after = format!("{first}\n\n{second}\n");
            assert_eq!(offenses(&before).len(), 1, "{before}");
            assert_correction(&EmptyLineBetweenDefs, &before, &after);
        }
    }

    #[test]
    fn exact_ranges_preserve_crlf_bom_comments_and_data() {
        use crate::cops::testing::correction_offenses;
        for newline in ["\n", "\r\n"] {
            let before = format!(
                "\u{feff}# 日本語{newline}def café{newline}end # é{newline}# doc{newline}def thé{newline}end{newline}__END__{newline}raw\n\n"
            );
            let after = before.replace(
                &format!("end # é{newline}"),
                &format!("end # é{newline}{newline}"),
            );
            let found = correction_offenses(&EmptyLineBetweenDefs, &before);
            assert_eq!((found[0].line, found[0].col), (5, 1));
            let c = found[0].correction.as_ref().unwrap();
            let start = before.find("# doc").unwrap();
            assert_eq!(c.range, start..start);
            assert_eq!(c.replacement, newline);
            assert_correction(&EmptyLineBetweenDefs, &before, &after);
            let before = format!(
                "def a{newline}end{newline} \t{newline}{newline}\t{newline}# doc{newline}def b{newline}end"
            );
            let after =
                format!("def a{newline}end{newline}\t{newline}# doc{newline}def b{newline}end");
            let found = correction_offenses(&EmptyLineBetweenDefs, &before);
            let c = found[0].correction.as_ref().unwrap();
            assert_eq!(
                &before.as_bytes()[c.range.clone()],
                format!(" \t{newline}{newline}").as_bytes()
            );
            assert_correction(&EmptyLineBetweenDefs, &before, &after);
        }
        assert_correction(
            &EmptyLineBetweenDefs,
            "def a\nend;def b\nend\n",
            "def a\nend;\n\ndef b\nend\n",
        );
        let found = offenses("def a\nend;def b\nend\n");
        assert_eq!((found[0].line, found[0].col), (2, 5));
    }

    #[test]
    fn newline_guards_withhold_without_hiding_offenses() {
        for gap in ["", "\n\n"] {
            for tail in [
                "p __LINE__\n",
                "class Lines\ndef self.line; __LINE__; end\nend\n",
                "p \"#{__LINE__}\"\n",
                "# __LINE__ lookalike\n",
            ] {
                let before = format!("def a\nend\n{gap}def b\nend\np :separator\n{tail}");
                assert_eq!(offenses(&before).len(), 1);
                assert_correction(&EmptyLineBetweenDefs, &before, &before);
            }
            let before = format!("p __LINE__\ndef a\nend\n{gap}def b\nend\n");
            assert_correction(
                &EmptyLineBetweenDefs,
                &before,
                "p __LINE__\ndef a\nend\n\ndef b\nend\n",
            );
        }
        for prefix in ["rubocop", "rustocop"] {
            for directive in [
                "disable Lint/Debugger",
                "disable-next Lint/Debugger",
                "enable Lint/Debugger",
            ] {
                for before in [
                    format!("def a\nend # {prefix}:{directive}\ndef b\nend\n"),
                    format!("def a\nend\n# {prefix}:{directive}\ndef b\nend\n"),
                    format!("def a\nend\n\n\n# {prefix}:{directive}\ndef b\nend\n"),
                ] {
                    assert_eq!(offenses(&before).len(), 1, "{before}");
                    assert_correction(&EmptyLineBetweenDefs, &before, &before);
                }
            }
            for target in [
                EmptyLineBetweenDefs.name(),
                "EmptyLineBetweenDefs",
                "Layout",
                "all",
            ] {
                for action in ["disable", "todo"] {
                    let before = format!("# {prefix}:{action} {target}\ndef a\nend\ndef b\nend\n");
                    assert!(offenses(&before).is_empty());
                    assert!(
                        crate::cops::testing::correction_offenses(&EmptyLineBetweenDefs, &before)
                            .is_empty()
                    );
                }
            }
            let before =
                format!("def a\nend\ndef b # {prefix}:disable Layout/EmptyLineBetweenDefs\nend\n");
            assert!(offenses(&before).is_empty());
        }
    }

    #[test]
    fn overlapping_heredoc_extents_and_explicit_continuations_withhold_edits() {
        for source in [
            "def a = <<~H; def b\n  hello\nH\n  1\nend\n",
            "def a\nend \\\n\n\n\ndef b\nend\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
            assert_correction(&EmptyLineBetweenDefs, source, source);
        }
    }

    #[test]
    fn interpolation_and_literal_lookalikes_are_not_rewritten_unsafely() {
        for literal in [
            "'def a\nend\ndef b\nend'",
            "\"def a\nend\ndef b\nend\"",
            "/def a\nend\ndef b\nend/",
            "%q(def a\nend\ndef b\nend)",
            "<<~H\ndef a\nend\ndef b\nend\nH",
        ] {
            let source = format!("x = {literal}\n");
            assert!(offenses(&source).is_empty(), "{source}");
        }
        for gap in ["", "\n\n"] {
            let code = format!("def a\nend\n{gap}def b\nend");
            for (open, close, safe) in [
                ("\"#{", "}\"", true),
                ("%W(a#{", "})", false),
                ("<<~H\n  #{", "}\nH", false),
            ] {
                let before = format!("x = {open}{code}{close}\n");
                let after = if safe {
                    format!("x = {open}def a\nend\n\ndef b\nend{close}\n")
                } else {
                    before.clone()
                };
                assert_eq!(offenses(&before).len(), 1, "{before}");
                assert_correction(&EmptyLineBetweenDefs, &before, &after);
            }
        }
    }

    #[test]
    fn mixed_padding_and_empty_line_corrections_are_atomic_and_converge() {
        use crate::cops::layout::{
            EmptyLines, EmptyLinesAroundClassBody, EmptyLinesAroundMethodBody,
            EmptyLinesAroundModuleBody,
        };
        let cops: [&dyn Cop; 5] = [
            &EmptyLineBetweenDefs,
            &EmptyLines,
            &EmptyLinesAroundClassBody,
            &EmptyLinesAroundMethodBody,
            &EmptyLinesAroundModuleBody,
        ];
        let before = "module M\n\n  class C\n\n    def a\n\n      work\n\n    end\n\n\n\n    # doc for b\n    def b\n\n      work\n\n    end\n\n  end\n  class D\n\n  end\n\nend\n";
        let after = "module M\n  class C\n    def a\n      work\n    end\n\n    # doc for b\n    def b\n      work\n    end\n  end\n\n  class D\n  end\nend\n";
        let mut first_pass = None;
        for selected in [cops.to_vec(), cops.into_iter().rev().collect()] {
            let mut source = before.to_owned();
            let mut seen = std::collections::HashSet::new();
            for pass in 0..5 {
                assert!(seen.insert(source.clone()), "oscillation");
                let found = crate::pipeline::lint_str_with_corrections(&selected, &source);
                let fixed = crate::corrections::apply(source.as_bytes(), &found)
                    .unwrap()
                    .bytes;
                if pass == 0 {
                    if let Some(expected) = &first_pass {
                        assert_eq!(&fixed, expected);
                    } else {
                        first_pass = Some(fixed.clone());
                    }
                }
                if fixed == source.as_bytes() {
                    break;
                }
                source = String::from_utf8(fixed).unwrap();
                assert_eq!(ruby_prism::parse(source.as_bytes()).errors().count(), 0);
            }
            assert_eq!(source, after);
            assert!(crate::pipeline::lint_str_with_corrections(&selected, &source).is_empty());
        }
    }

    #[test]
    fn pinned_no_offense_lookalikes_and_comment_groups() {
        for source in [
            "",
            "x = 0\ndef m\nend\n",
            "x = 0\n# 123\ndef m\nend\n",
            "class K\ndef m\nend\nend\n",
            "def a\nend\nprivate\ndef b\nend\n",
            "def a\nend\nprivate def b\nend\n",
            "private def a\nend\ndef b\nend\n",
            "if condition\ndef foo\nend\nelse\ndef foo\nend\nend\n",
            "def a\nend\nfoo :attribute\ndef b\nend\n",
            "foo do\nend\nfoo do\nend\n",
            "def a\nend\n# doc\n\n\ndef b\nend\n",
            "def a\nend\n\n# doc\n\ndef b\nend\n",
            "def a\nend\nclass << self\nend\n",
            "__END__\ndef a\nend\ndef b\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
