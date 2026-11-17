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
                .then(|| Conditional::parse(&offending).and_then(|c| c.rewrite(&offending, file)))
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
    /// RuboCop's `autocorrect_block`.
    fn rewrite(&self, node: &Node<'_>, file: &SourceFile) -> Option<Correction> {
        let start = self.keyword.start_offset();
        let end = self.end.end_offset();
        let condition = std::str::from_utf8(self.condition.location().as_slice()).ok()?;
        let header_end = self
            .then
            .as_ref()
            .map_or(self.condition.location().end_offset(), |l| l.end_offset());
        let mut edits = vec![Correction {
            range: start..header_end,
            replacement: format!(
                "next {} {condition}",
                if self.unless { "if" } else { "unless" }
            ),
        }];
        let end_line = file.line_of(end);
        let mut end_begin = file.line_start(end_line);
        let rest = &file.line_text(end_line)[end - file.line_start(end_line)..];
        if end_begin > 0 && rest.iter().all(u8::is_ascii_whitespace) {
            end_begin -= 1;
        }
        edits.push(Correction {
            range: end_begin..end,
            replacement: String::new(),
        });
        // `reindentable_lines`, without lines starting inside a string.
        let content = string_content(node, file);
        let line = file.line_of(start);
        let indent = |n: u32| {
            std::str::from_utf8(file.line_text(n)).ok().and_then(|t| {
                t.find(|c: char| !c.is_whitespace())
                    .map(|i| t[..i].chars().count())
            })
        };
        let lines: Vec<u32> = (line + 1..end_line)
            .filter(|&n| {
                let at = file.line_start(n);
                !content.iter().any(|r| r.contains(&at))
                    && !file.line_text(n).iter().all(u8::is_ascii_whitespace)
            })
            .collect();
        if let Some(actual) = lines.iter().filter_map(|&n| indent(n)).min() {
            let target =
                indent(file.line_of(self.condition.location().start_offset())).unwrap_or(0);
            if actual > target {
                let delta = actual - target;
                for n in lines {
                    let at = file.line_start(n);
                    let text = std::str::from_utf8(file.line_text(n)).ok()?;
                    let bytes: usize = text.chars().take(delta).map(char::len_utf8).sum();
                    edits.push(Correction {
                        range: at..at + bytes,
                        replacement: String::new(),
                    });
                }
            }
        }
        super::correction::combine(file, edits)
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

/// RuboCop's `string_content_range` for every string in a node.
fn string_content(node: &Node<'_>, file: &SourceFile) -> Vec<std::ops::Range<usize>> {
    struct Found<'a> {
        file: &'a SourceFile,
        ranges: Vec<std::ops::Range<usize>>,
    }
    impl<'pr> crate::cops::walk::NodeVisitor<'pr> for Found<'_> {
        fn enter(&mut self, n: Node<'pr>) {
            let (opening, closing) = match &n {
                Node::StringNode { .. } => {
                    let s = n.as_string_node().unwrap();
                    (s.opening_loc(), s.closing_loc())
                }
                Node::InterpolatedStringNode { .. } => {
                    let s = n.as_interpolated_string_node().unwrap();
                    (s.opening_loc(), s.closing_loc())
                }
                Node::XStringNode { .. } => {
                    let s = n.as_x_string_node().unwrap();
                    (Some(s.opening_loc()), Some(s.closing_loc()))
                }
                Node::InterpolatedXStringNode { .. } => {
                    let s = n.as_interpolated_x_string_node().unwrap();
                    (Some(s.opening_loc()), Some(s.closing_loc()))
                }
                _ => return,
            };
            let (Some(opening), Some(closing)) = (opening, closing) else {
                return;
            };
            if opening.as_slice().starts_with(b"<<") {
                let body = self
                    .file
                    .line_start(self.file.line_of(opening.start_offset()) + 1);
                let terminator = self
                    .file
                    .line_start(self.file.line_of(closing.start_offset()));
                self.ranges.push(body.min(terminator)..terminator);
            } else {
                self.ranges
                    .push(opening.end_offset()..closing.start_offset());
            }
        }
    }
    let mut found = Found {
        file,
        ranges: Vec::new(),
    };
    crate::cops::walk::walk(&mut found, node);
    found.ranges
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
