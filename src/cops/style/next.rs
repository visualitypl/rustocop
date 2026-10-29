//! `Style/Next` — prefer an iteration guard at the end of an enumerator/loop.

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node, StatementsNode};

pub struct Next;
impl Cop for Next {
    fn name(&self) -> &'static str {
        "Style/Next"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/SafeNavigation"]
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let body = if let Some(n) = node.as_while_node() {
            n.statements().map(|s| s.as_node())
        } else if let Some(n) = node.as_until_node() {
            n.statements().map(|s| s.as_node())
        } else if let Some(n) = node.as_for_node() {
            n.statements().map(|s| s.as_node())
        } else if let Some(n) = node.as_call_node() {
            if !enumerator(n.name().as_slice()) {
                return;
            }
            n.block()
                .and_then(|b| b.as_block_node())
                .and_then(|b| b.body())
        } else {
            None
        };
        let Some(body) = body else {
            return;
        };
        let singleton = body.as_statements_node().is_some_and(|s| {
            s.body().len() == 1
                && s.body()
                    .iter()
                    .next()
                    .is_some_and(|n| n.as_parentheses_node().is_none())
        });
        let Some(tail) = tail(body) else {
            return;
        };
        let Some(c) = Conditional::of(&tail, file) else {
            return;
        };
        // RuboCop selects the last AST child when it is itself an if. For
        // unless that is its sole body expression, not the wrapping unless.
        let offending = (singleton && c.unless)
            .then(|| {
                c.body
                    .as_ref()
                    .filter(|s| s.body().len() == 1)
                    .and_then(|s| s.body().iter().next())
                    .filter(|n| n.as_if_node().is_some() || n.as_unless_node().is_some())
            })
            .flatten()
            .unwrap_or(tail);
        let condition = if let Some(n) = offending.as_if_node() {
            n.predicate()
        } else {
            offending.as_unless_node().unwrap().predicate()
        };
        let (line, col) = file.line_col(offending.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: "Use `next` to skip iteration.".into(),
            line,
            col,
            end_line: file.line_of(condition.location().end_offset().saturating_sub(1)),
            correction: cx
                .corrections_requested()
                .then(|| Conditional::parse(&offending).and_then(|c| c.rewrite(cx, file)))
                .flatten(),
        });
    }
}

fn enumerator(name: &[u8]) -> bool {
    name.starts_with(b"each_")
        || matches!(
            name,
            b"collect"
                | b"collect_concat"
                | b"detect"
                | b"downto"
                | b"each"
                | b"find"
                | b"find_all"
                | b"find_index"
                | b"inject"
                | b"loop"
                | b"map!"
                | b"map"
                | b"reduce"
                | b"reject"
                | b"reject!"
                | b"reverse_each"
                | b"select"
                | b"select!"
                | b"times"
                | b"upto"
        )
}

fn tail(body: Node<'_>) -> Option<Node<'_>> {
    // A singleton StatementsNode is transparent upstream, but parentheses
    // and multi-statement lists each introduce a begin wrapper. Inspect only
    // one such wrapper, not a parenthesized last child of another wrapper.
    let body = if let Some(s) = body.as_statements_node() {
        let last = s.body().iter().last()?;
        if s.body().len() > 1 {
            return Some(last);
        }
        last
    } else {
        body
    };
    if let Some(p) = body.as_parentheses_node() {
        p.body()?.as_statements_node()?.body().iter().last()
    } else {
        Some(body)
    }
}
struct Conditional<'pr> {
    keyword: Location<'pr>,
    condition: Node<'pr>,
    body: Option<StatementsNode<'pr>>,
    end: Location<'pr>,
    then: Option<Location<'pr>>,
    unless: bool,
}
impl<'pr> Conditional<'pr> {
    fn rewrite(&self, cx: &NodeContext<'_, '_>, file: &SourceFile) -> Option<Correction> {
        // A finite local proof budget bounds nested candidates as well as wide
        // sibling lists; detection has no such limit. No correction reparses.
        const BUDGET: usize = 32 * 1024;
        let start = self.keyword.start_offset();
        let end = self.end.end_offset();
        if end - start > BUDGET {
            return None;
        }
        #[cfg(test)]
        REWRITE_BYTES.with(|n| n.set(n.get() + end - start));
        let root = cx.ancestors().next_back()?;
        let line = file.line_of(start);
        let end_line = file.line_of(end);
        let prefix = &file.bytes[file.line_start(line)..start];
        let suffix = &file.line_text(end_line)[end - file.line_start(end_line)..];
        let end_prefix = &file.bytes[file.line_start(end_line)..self.end.start_offset()];
        if !end_prefix.iter().all(|b| matches!(b, b' ' | b'\t'))
            || !prefix.iter().all(|b| matches!(b, b' ' | b'\t'))
            || suffix.len() > BUDGET
            || suffix
                .iter()
                .copied()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| b != b'#')
            || file.line_of(self.condition.location().end_offset()) != line
            || file.heredoc_pending_at(root, start)
            || file.contains_heredoc(root, &self.condition)
            || super::correction::touches_directive(file, start..end)
            || cx.ancestors().any(|n| n.as_defined_node().is_some())
        {
            return None;
        }
        let condition = std::str::from_utf8(self.condition.location().as_slice()).ok()?;
        let header_end = self
            .then
            .as_ref()
            .map_or(self.condition.location().end_offset(), |l| l.end_offset());
        if super::correction::contains_comment(file, start..header_end)
            // Removing `then` must not merge an inline body expression into
            // the guard predicate (e.g. `next unless ready? first`). Keep the
            // offense but withhold the ambiguous inline-header rewrite.
            || self.then.is_some()
                && file.bytes[header_end..file.line_start(file.line_of(header_end) + 1)]
                    .iter()
                    .copied()
                    .find(|b| !b.is_ascii_whitespace())
                    .is_some_and(|b| b != b'#')
        {
            return None;
        }
        let mut edits = vec![Correction {
            range: start..header_end,
            replacement: format!(
                "next {} {condition}",
                if self.unless { "if" } else { "unless" }
            ),
        }];
        // A deleted end line must not shift any later __LINE__. Keep a blank
        // physical line in that case; never flatten a multiline predicate.
        let preserve_lines = start < file.line_value_boundary();
        let remove_end = if preserve_lines {
            self.end.start_offset()..end
        } else if suffix.iter().all(u8::is_ascii_whitespace) {
            file.line_start(end_line)..file.line_start(end_line + 1)
        } else {
            file.line_start(end_line)..end
        };
        edits.push(Correction {
            range: remove_end,
            replacement: String::new(),
        });

        // Reuse the main-parse literal index. Leading bytes inside a literal
        // are values, not indentation. Keep heredoc terminators byte-exact too:
        // the shared physical-ownership proof must protect adjacent heredocs.
        let mut lines = Vec::new();
        for number in line + 1..end_line {
            let at = file.line_start(number);
            let text = file.line_text(number);
            let indent = text
                .iter()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            if text.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if file.overlaps_literal(root, &(at..at + 1)) {
                continue;
            }
            // Tabs have display-dependent widths. Keep the offense, do not
            // guess at canonical indentation or alter a literal terminator.
            if text[..indent].contains(&b'\t') || prefix.contains(&b'\t') {
                return None;
            }
            lines.push((at, indent));
        }
        let delta = lines
            .iter()
            .map(|(_, n)| *n)
            .min()
            .unwrap_or(prefix.len())
            .saturating_sub(prefix.len());
        for (at, _) in lines {
            if delta > 0 {
                edits.push(Correction {
                    range: at..at + delta,
                    replacement: String::new(),
                });
            }
        }
        let edit = super::correction::combine(file, edits)?;
        (super::correction::preserves_line_values(file, &edit)
            && super::correction::preserves_data_positions(file, cx, &edit))
        .then_some(edit)
    }

    fn parse(node: &Node<'pr>) -> Option<Self> {
        Some(if let Some(n) = node.as_if_node() {
            if n.subsequent().is_some() {
                return None;
            }
            Self {
                keyword: n.if_keyword_loc()?,
                condition: n.predicate(),
                body: n.statements(),
                end: n.end_keyword_loc()?,
                then: n.then_keyword_loc(),
                unless: false,
            }
        } else {
            let n = node.as_unless_node()?;
            if n.else_clause().is_some() {
                return None;
            }
            Self {
                keyword: n.keyword_loc(),
                condition: n.predicate(),
                body: n.statements(),
                end: n.end_keyword_loc()?,
                then: n.then_keyword_loc(),
                unless: true,
            }
        })
    }

    fn of(node: &Node<'pr>, file: &SourceFile) -> Option<Self> {
        let c = Self::parse(node)?;
        if file.line_of(c.end.start_offset()) <= file.line_of(c.keyword.start_offset()) + 3 {
            return None;
        }
        let child = c
            .body
            .as_ref()
            .filter(|s| s.body().len() == 1)
            .and_then(|s| s.body().iter().next());
        // RuboCop checks direct if children, not arbitrary descendants. Prism's
        // singleton StatementsNode is transparent in that AST.
        if std::iter::once(&c.condition)
            .chain(child.as_ref())
            .any(|n| {
                n.as_if_node()
                    .is_some_and(|n| n.if_keyword_loc().is_some() && n.subsequent().is_some())
                    || n.as_unless_node()
                        .is_some_and(|n| n.else_clause().is_some())
            })
            || child.is_some_and(|n| matches!(n, Node::BreakNode { .. } | Node::ReturnNode { .. }))
        {
            return None;
        }
        Some(c)
    }
}

#[cfg(test)]
thread_local! {
    static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop next_spec.rb (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::offense::Offense;
    use crate::pipeline::lint_str;
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/Next")
            .collect()
    }
    #[test]
    fn ternaries_are_not_explicit_nested_else_branches() {
        for source in [
            "each do\n  if ready? ? first : second\n    a\n    b\n    c\n  end\nend\n",
            "each do\n  if ready?\n    choice ?\n      first :\n      second\n  end\nend\n",
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3));
            assert_eq!(found[0].message, "Use `next` to skip iteration.");
        }
    }

    #[test]
    fn trailing_condition_checks_only_one_upstream_begin_wrapper() {
        for source in [
            "each do\n((if c\n a\n b\n c\nend))\nend\n",
            "each do\nbefore\n(if c\n a\n b\n c\nend)\nend\n",
            "each do\n((before; if c\n a\n b\n c\nend))\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
        for (source, col) in [
            ("each do\n(if c\n a\n b\n c\nend)\nend\n", 2),
            ("each do\n(before; if c\n a\n b\n c\nend)\nend\n", 10),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, col));
        }
    }

    #[test]
    fn an_end_sharing_a_body_statement_is_never_deleted_as_a_whole_line() {
        let source = "each do\n if c\n  first\n  second\n  third\n  result; end\nend\n";
        let found = crate::cops::testing::correction_offenses(&super::Next, source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn unless_exit_branches_and_single_nested_if_match_upstream_locations() {
        for kw in ["if", "unless"] {
            for exit in ["break", "return"] {
                let source =
                    format!("def f\n each do\n  {kw} c\n   {exit}\n\n\n  end\n end\nend\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        let source =
            "each do\n unless a\n  if b\n   first\n   second\n   third\n  end\n end\nend\n";
        let found = offenses(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (3, 3));
        let source = source.replace("each do\n", "each do\n before\n");
        let found = offenses(&source);
        assert_eq!((found[0].line, found[0].col), (3, 2));
    }

    #[test]
    fn then_comments_and_assignment_predicates_correct_atomically() {
        use crate::cops::testing::assert_correction;
        for predicate in [
            "ready?",
            "(value = ready?)",
            "ready? and other?",
            "not ready?",
        ] {
            assert_correction(
                &super::Next,
                &format!(
                    "each do\n  if {predicate} then # condition\n    first\n    second\n    third\n  end # tail\nend\n"
                ),
                &format!(
                    "each do\n  next unless {predicate} # condition\n  first\n  second\n  third\n # tail\nend\n"
                ),
            );
        }
    }

    #[test]
    fn implicit_parameters_parenthesized_tails_and_consecutive_conditionals() {
        for source in [
            "each do\n if _1\n  a\n  b\n  c\n end\nend\n",
            "each do\n unless it\n  a\n  b\n  c\n end\nend\n",
            "each do\n (before; if c\n  a\n  b\n  c\n end)\nend\n",
            "each do\n if first\n  a\n end\n if second\n  a\n  b\n  c\n end\nend\n",
        ] {
            assert_eq!(offenses(source).len(), 1, "{source}");
        }
        let source = "each do\n if\n  condition\n  first\n  second\n end\nend\n";
        let found = offenses(source);
        assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 2, 3));
        assert!(
            crate::cops::testing::correction_offenses(&super::Next, source)[0]
                .correction
                .is_none()
        );
    }

    #[test]
    fn incompatible_safe_navigation_selection_keeps_diagnostics_without_edits() {
        use crate::{
            cops::{Cop, run_cops},
            source::SourceFile,
        };
        struct SafeNavigation;
        impl Cop for SafeNavigation {
            fn name(&self) -> &'static str {
                "Style/SafeNavigation"
            }
        }
        let source = "each do\n if c\n  a\n  b\n  c\n end\nend\n";
        let file = SourceFile::new("x.rb".into(), source.as_bytes().to_vec());
        let parsed = ruby_prism::parse(source.as_bytes());
        for cops in [
            vec![&super::Next as &dyn Cop, &SafeNavigation],
            vec![&SafeNavigation as &dyn Cop, &super::Next],
        ] {
            let mut found = Vec::new();
            assert!(run_cops(&cops, &parsed, &file, &mut found).is_empty());
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none());
        }
    }

    #[test]
    fn correction_work_is_lazy_and_locally_bounded() {
        use crate::{cops::testing::correction_offenses, pipeline::lint_str_with};
        for count in [1500, 3000] {
            let source = "each do\n if c\n  a\n  b\n  c\n end\nend\n".repeat(count);
            super::REWRITE_BYTES.with(|n| n.set(0));
            assert_eq!(lint_str_with(&[&super::Next], &source).len(), count);
            super::REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
            let found = correction_offenses(&super::Next, &source);
            assert_eq!(found.len(), count);
            assert!(found.iter().all(|o| o.correction.is_some()));
            super::REWRITE_BYTES.with(|n| assert!(n.get() <= source.len()));
        }
        let source = format!(
            "each do\n if c\n{}\n  a\n  b\n  c\n end\nend\n",
            " # large body\n".repeat(3000)
        );
        let found = correction_offenses(&super::Next, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn canonical_guard_correction() {
        let before = "[].map do |x|\n  if ready?(x)\n    first(x)\n    second(x)\n    result(x)\n  end\nend\n";
        let found = crate::cops::testing::correction_offenses(&super::Next, before);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_some());
        let applied = crate::corrections::apply(before.as_bytes(), &found).unwrap();
        assert_eq!(
            String::from_utf8(applied.bytes).unwrap(),
            "[].map do |x|\n  next unless ready?(x)\n  first(x)\n  second(x)\n  result(x)\nend\n"
        );
    }
    #[test]
    fn fixed_minimum_counts_physical_lines_not_statements() {
        for kw in ["if", "unless"] {
            for (body, expected) in [
                ("    work\n", 0),
                ("    a\n    b\n", 0),
                ("    a\n    b\n    c\n", 1),
                ("    # comment\n\n    work\n", 1),
            ] {
                let source = format!("[].each do |o|\n  {kw} o == 1\n{body}  end\nend\n");
                let found = offenses(&source);
                assert_eq!(found.len(), expected, "{source}");
                if expected == 1 {
                    assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                    assert_eq!(found[0].message, "Use `next` to skip iteration.");
                }
            }
        }
    }
    #[test]
    fn enumerator_names_and_all_loop_shapes() {
        for head in [
            "3.downto(1) do",
            "[]&.each do",
            "[].each_with_object({}) do",
            "each_custom do",
            "loop do",
            "[].collect do",
            "[].select! do",
            "[].reject! do",
            "[].map! do",
            "for o in xs do",
            "while active",
            "until done",
            "begin",
        ] {
            let source =
                format!("{head}\n  before\n  if ready?\n    a\n    b\n    c\n  end\nend\n");
            let found = offenses(&source);
            assert_eq!(found.len(), usize::from(head != "begin"), "{source}");
            if !found.is_empty() {
                assert_eq!((found[0].line, found[0].col), (3, 3));
            }
        }
    }
    #[test]
    fn skips_modifiers_non_enumerators_non_tail_and_else_association() {
        for source in [
            "each { work if ready? }",
            "custom do\n if c\n a\n b\n c\n end\nend",
            "each do\n if c\n a\n b\n c\n end\n after\nend",
            "each do\n if c\n a\n b\n c\n else\n fallback\n end\nend",
            "each do\n if c\n a\n b\n c\n elsif d\n fallback\n end\nend",
            "each do\n if c\n if d\n a\n else\n b\n end\n end\nend",
            "each do\n begin\n if c\n a\n b\n c\n end\n rescue\n recover\n ensure\n cleanup\n end\nend",
            "each do\n if c\n break\n\n\n end\nend",
            "def f\n each do\n if c\n return\n\n\n end\nend\nend",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
