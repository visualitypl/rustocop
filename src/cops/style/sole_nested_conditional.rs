//! `Style/SoleNestedConditional` — merge sole nested guards with atomic safe edits.

use ruby_prism::{Location, Node, StatementsNode};

use crate::{
    cops::{
        Cop, NodeContext, ast, variables,
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

#[cfg(test)]
thread_local! {
    static HEREDOC_PREDICATE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub struct SoleNestedConditional;

impl Cop for SoleNestedConditional {
    fn name(&self) -> &'static str {
        "Style/SoleNestedConditional"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/NegatedIf", "Style/NegatedUnless"]
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(outer) = conditional(node) else {
            return;
        };
        let Some(ref statements) = outer.statements else {
            return;
        };
        if statements.body().len() != 1 {
            return;
        }
        let branch = statements.body().iter().next().unwrap();
        let Some(inner) = conditional(&branch) else {
            return;
        };
        // Mirror upstream's correction_parses? detection gate structurally:
        // no corrections or per-candidate reparses are needed for detection.
        if !merge_is_parseable(&outer, &inner, cx, file) {
            return;
        }
        if variables::condition_assigns_inner_source(&outer.predicate, &inner.predicate) {
            return;
        }
        let (line, col) = file.line_col(inner.keyword.start_offset());
        let outer_keyword = if outer.keyword.as_slice() == b"unless" {
            "unless"
        } else {
            "if"
        };
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Consider merging nested conditions into outer `{outer_keyword}` conditions."
            ),
            line,
            col,
            end_line: line,
            correction: cx
                .corrections_requested()
                .then(|| correction(&outer, &inner, file))
                .flatten(),
        });
    }
}

struct Conditional<'pr> {
    keyword: Location<'pr>,
    predicate: Node<'pr>,
    modifier: bool,
    statements: Option<StatementsNode<'pr>>,
    end: Option<Location<'pr>>,
}

// Prism keeps the syntactic body of unless in `statements`, unlike parser's
// truth-value-ordered branches. Else/elsif and ternary forms cannot be merged.
fn conditional<'pr>(node: &Node<'pr>) -> Option<Conditional<'pr>> {
    if let Some(n) = node.as_if_node() {
        let keyword = n.if_keyword_loc()?;
        if keyword.as_slice() == b"elsif" || n.subsequent().is_some() {
            return None;
        }
        Some(Conditional {
            keyword,
            predicate: n.predicate(),
            modifier: n.end_keyword_loc().is_none(),
            statements: n.statements(),
            end: n.end_keyword_loc(),
        })
    } else {
        let n = node.as_unless_node()?;
        if n.else_clause().is_some() {
            return None;
        }
        Some(Conditional {
            keyword: n.keyword_loc(),
            predicate: n.predicate(),
            modifier: n.end_keyword_loc().is_none(),
            statements: n.statements(),
            end: n.end_keyword_loc(),
        })
    }
}

fn range(loc: &Location<'_>) -> std::ops::Range<usize> {
    loc.start_offset()..loc.end_offset()
}
fn edit(range: std::ops::Range<usize>, replacement: impl Into<String>) -> Correction {
    Correction {
        range,
        replacement: replacement.into(),
    }
}
fn text<'a>(node: &'a Node<'_>) -> Option<&'a str> {
    std::str::from_utf8(node.location().as_slice()).ok()
}

/// RuboCop's `autocorrect`.
fn correction(
    outer: &Conditional<'_>,
    inner: &Conditional<'_>,
    file: &SourceFile,
) -> Option<Correction> {
    let mut edits = Vec::new();
    let header =
        |n: &Conditional<'_>| n.keyword.start_offset()..n.predicate.location().end_offset();
    if outer.modifier {
        correct_node(inner, &mut edits, file)?;
        let at = inner.predicate.location().start_offset();
        edits.push(edit(at..at, format!("{} && ", chainable(outer, file)?)));
        edits.push(edit(spaces_around(file, header(outer)), ""));
    } else {
        correct_node(outer, &mut edits, file)?;
        let outer_end = outer.predicate.location().end_offset();
        if inner.modifier {
            edits.push(edit(
                outer_end..outer_end,
                format!(" && {}", chainable(inner, file)?),
            ));
            edits.push(edit(spaces_around(file, header(inner)), ""));
        } else {
            edits.push(edit(
                outer_end..inner.predicate.location().start_offset(),
                " && ",
            ));
            edits.push(edit(
                range(&inner.predicate.location()),
                chainable(inner, file)?,
            ));
            let end = outer.end.as_ref()?;
            let end_range = if file.line_of(end.start_offset())
                == file.line_of(inner.end.as_ref()?.start_offset())
            {
                range(end)
            } else {
                super::correction::whole_lines(file, range(end))
            };
            edits.push(edit(end_range, ""));
            // The comments upstream associates with the inner conditional:
            // those before it, but not one trailing the outer condition.
            let condition_line = file.last_line_of(outer.predicate.location());
            let lifted = file
                .comments()
                .iter()
                .filter(|c| {
                    c.start >= outer_end
                        && c.end <= inner.keyword.start_offset()
                        && file.line_of(c.start) != condition_line
                })
                .map(|c| std::str::from_utf8(&file.bytes[c.clone()]).ok())
                .collect::<Option<Vec<_>>>()?;
            if !lifted.is_empty() {
                let at = outer.keyword.start_offset();
                edits.push(edit(at..at, format!("{}\n", lifted.join("\n"))));
            }
        }
    }
    super::correction::combine(file, edits)
}

fn correct_node(n: &Conditional<'_>, edits: &mut Vec<Correction>, file: &SourceFile) -> Option<()> {
    if n.keyword.as_slice() == b"unless" {
        edits.push(edit(range(&n.keyword), "if"));
    }
    edits.push(edit(range(&n.predicate.location()), chainable(n, file)?));
    Some(())
}

/// `range_with_surrounding_space(range, newlines: false)`.
fn spaces_around(file: &SourceFile, r: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let blank = |b: &u8| matches!(b, b' ' | b'\t');
    let start = r.start
        - file.bytes[..r.start]
            .iter()
            .rev()
            .take_while(|b| blank(b))
            .count();
    let end = r.end + file.bytes[r.end..].iter().take_while(|b| blank(b)).count();
    start..end
}

fn chainable(n: &Conditional<'_>, file: &SourceFile) -> Option<String> {
    let wrapped = with_parentheses(&n.predicate, file)?;
    if n.keyword.as_slice() != b"unless" {
        return Some(wrapped);
    }
    Some(if n.predicate.as_and_node().is_some() {
        format!("!({wrapped})")
    } else {
        format!("!{wrapped}")
    })
}

/// A call's arguments as RuboCop counts them, a block pass included.
fn has_arguments(c: &ruby_prism::CallNode<'_>) -> bool {
    c.arguments().is_some_and(|a| !a.arguments().is_empty())
        || c.block()
            .is_some_and(|b| b.as_block_argument_node().is_some())
}

/// RuboCop's `add_parentheses_if_needed`.
fn with_parentheses(condition: &Node<'_>, file: &SourceFile) -> Option<String> {
    let source = text(condition)?;
    let call = condition.as_call_node();
    let block = call
        .as_ref()
        .is_some_and(|c| c.block().is_some_and(|b| b.as_block_node().is_some()));
    let parenthesized =
        |c: &ruby_prism::CallNode<'_>| c.opening_loc().is_some_and(|o| o.as_slice() == b"(");
    let needed = crate::cops::ast::is_assignment(condition)
        || condition.as_or_node().is_some()
        || condition
            .as_and_node()
            .is_some_and(|_| contains_assignment(condition))
        || call.as_ref().is_some_and(|c| {
            (has_arguments(c) && !parenthesized(c))
                || c.message_loc().is_some_and(|m| m.as_slice() == b"not")
        });
    if !needed {
        return Some(source.into());
    }
    if let Some(c) = call.filter(|c| {
        !block
            && has_arguments(c)
            && !parenthesized(c)
            && !matches!(
                c.name().as_slice(),
                b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<"
            )
            && !ast::is_operator_method(c.name().as_slice())
    }) {
        let start = condition.location().start_offset();
        let selector_end = c.message_loc()?.end_offset();
        let first = ast::call_arguments(&c)
            .first()
            .map(|a| a.location().start_offset())
            .or_else(|| c.block().map(|b| b.location().start_offset()))?;
        let end = condition.location().end_offset();
        let part = |r: std::ops::Range<usize>| std::str::from_utf8(&file.bytes[r]).ok();
        return Some(format!(
            "{}({})",
            part(start..selector_end)?,
            part(first..end)?
        ));
    }
    if condition.as_and_node().is_some() {
        return parenthesized_and(condition, file);
    }
    Some(format!("({source})"))
}

fn contains_assignment(node: &Node<'_>) -> bool {
    struct Found(bool);
    impl NodeVisitor<'_> for Found {
        fn enter(&mut self, n: Node<'_>) {
            self.0 |= crate::cops::ast::is_assignment(&n);
        }
    }
    let mut found = Found(false);
    walk(&mut found, node);
    found.0
}

/// RuboCop's `parenthesized_and`: only an assignment as the last clause
/// gains parentheses.
fn parenthesized_and(node: &Node<'_>, file: &SourceFile) -> Option<String> {
    let and = node.as_and_node()?;
    let rhs = and.right();
    let rhs = if rhs.as_and_node().is_some() {
        parenthesized_and(&rhs, file)?
    } else if crate::cops::ast::is_assignment(&rhs) {
        format!("({})", text(&rhs)?)
    } else {
        text(&rhs)?.to_owned()
    };
    let op = and.operator_loc();
    let space = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c');
    let start = op.start_offset()
        - file.bytes[..op.start_offset()]
            .iter()
            .rev()
            .take_while(|b| space(b))
            .count();
    let end = op.end_offset()
        + file.bytes[op.end_offset()..]
            .iter()
            .take_while(|b| space(b))
            .count();
    Some(format!(
        "{}{}{rhs}",
        text(&and.left())?,
        std::str::from_utf8(&file.bytes[start..end]).ok()?
    ))
}

fn merge_is_parseable(
    outer: &Conditional<'_>,
    inner: &Conditional<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> bool {
    // The merged header appends `&&` after the outer predicate, which Ruby
    // would interpret as part of an unparenthesized pattern. Conversely a
    // command-form yield/super cannot start the RHS of `&&`. Ordinary method
    // arguments are parenthesized by upstream, but yield/super are not.
    if outer.predicate.as_match_predicate_node().is_some()
        || unparenthesized_keyword_arguments(&inner.predicate)
    {
        return false;
    }
    // Removing an outer modifier also consumes surrounding horizontal space.
    // For a block-form inner conditional this joins its closing `end` to
    // `rescue`, leaving the block unterminated. A double modifier has no such
    // closing keyword (e.g. `ready?rescue` is still parseable).
    if outer.modifier
        && !inner.modifier
        && let Some(rescue) = cx.parent().and_then(|n| n.as_rescue_modifier_node())
        && rescue.expression().location().end_offset() == outer.predicate.location().end_offset()
        && file.bytes[outer.predicate.location().end_offset()..rescue.keyword_loc().start_offset()]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t'))
    {
        return false;
    }
    !((outer.modifier || !inner.modifier) && trailing_heredoc(&outer.predicate))
}

fn unparenthesized_keyword_arguments(predicate: &Node<'_>) -> bool {
    if let Some(n) = predicate.as_yield_node() {
        n.arguments().is_some() && n.lparen_loc().is_none()
    } else if let Some(n) = predicate.as_super_node() {
        n.arguments().is_some() && n.lparen_loc().is_none()
    } else {
        false
    }
}

// Prism's heredoc node location covers the opener, not the body/terminator.
// A terminator outside the predicate source would be cut off by header merging.
fn trailing_heredoc(predicate: &Node<'_>) -> bool {
    struct Trailing {
        end: usize,
        found: bool,
    }
    impl NodeVisitor<'_> for Trailing {
        fn enter(&mut self, node: Node<'_>) {
            #[cfg(test)]
            HEREDOC_PREDICATE_VISITS.with(|count| count.set(count.get() + 1));
            let closing = match node {
                Node::StringNode { .. } => node.as_string_node().unwrap().closing_loc(),
                Node::InterpolatedStringNode { .. } => {
                    node.as_interpolated_string_node().unwrap().closing_loc()
                }
                Node::XStringNode { .. } => Some(node.as_x_string_node().unwrap().closing_loc()),
                Node::InterpolatedXStringNode { .. } => {
                    Some(node.as_interpolated_x_string_node().unwrap().closing_loc())
                }
                _ => None,
            };
            self.found |= closing.is_some_and(|loc| loc.end_offset() > self.end);
        }
    }
    let mut trailing = Trailing {
        end: predicate.location().end_offset(),
        found: false,
    };
    walk(&mut trailing, predicate);
    trailing.found
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `style/sole_nested_conditional_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use crate::{offense::Offense, pipeline::lint_str};

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/SoleNestedConditional")
            .collect()
    }

    fn assert_offenses(source: &str, expected: &[(u32, u32, &str)]) {
        let found = offenses(source);
        assert_eq!(found.len(), expected.len(), "{source}: {found:?}");
        for (offense, &(line, col, outer)) in found.iter().zip(expected) {
            assert_eq!(
                (offense.line, offense.col, offense.end_line),
                (line, col, line),
                "{source}"
            );
            assert_eq!(
                offense.message,
                format!("Consider merging nested conditions into outer `{outer}` conditions.")
            );
            assert!(offense.correction.is_none());
        }
    }

    #[test]
    fn exempts_conditions_reading_an_outer_assignment() {
        for condition in [
            "var = foo",
            "cond && var = foo",
            "(var = foo)",
            "var ||= foo",
            "var &&= foo",
            "var += foo",
            "(var, other = foo)",
            "check(var = foo)",
            "items.each { var = foo }",
        ] {
            for inner in ["if", "unless"] {
                assert_offenses(&format!("if {condition}\n  work {inner} var\nend\n"), &[]);
            }
        }
        for variable in ["@var", "@@var", "$var"] {
            assert_offenses(
                &format!("if {variable} = foo\n  if {variable}\n    work\n  end\nend\n"),
                &[],
            );
        }
    }

    #[test]
    fn modifier_forms_report_the_inner_keyword_and_outer_type() {
        for outer in ["if", "unless"] {
            for inner in ["if", "unless"] {
                assert_offenses(
                    &format!("{outer} foo\n  do_something {inner} bar\nend\n"),
                    &[(2, 16, outer)],
                );
                assert_offenses(
                    &format!("{inner} foo\n  do_something\nend {outer} bar\n"),
                    &[(1, 1, outer)],
                );
            }
        }
        // AllowModifier is fixed false, including at both levels.
        assert_offenses(
            "if foo\n  work if bar\nend if baz\n",
            &[(1, 1, "if"), (2, 8, "if")],
        );
    }

    #[test]
    fn condition_expression_variants_from_upstream() {
        let conditions = [
            "foo == bar",
            "foo === bar",
            "foo & bar",
            "foo || bar",
            "(foo || bar)",
            "foo && bar && baz",
            "not foo",
            "!foo",
            "foo.is_a? Foo",
            "foo.is_a?(Foo)",
            "foo.bar arg1, arg2",
            "ok? bar",
            "obj&.ok? bar",
            "ok?(bar)",
            "h[:a]",
            "foo = bar",
            "foo = bar and baz",
            "baz && foo = bar",
            "baz &&\n     foo = bar",
            "baz && foo = bar and fred = garply",
            "baz && (foo = bar)",
            "foo = bar or baz",
            "baz || foo = bar",
            "ok? bar do\n      work\n    end",
            "ok? bar do\n      _1\n    end",
            "ok? bar { it }",
        ];
        for condition in conditions {
            for keyword in ["if", "unless"] {
                assert_offenses(
                    &format!("{keyword} {condition}\n  if quux\n    work\n  end\nend\n"),
                    &[(condition.lines().count() as u32 + 1, 3, keyword)],
                );
                assert_offenses(
                    &format!("if quux\n  {keyword} {condition}\n    work\n  end\nend\n"),
                    &[(2, 3, "if")],
                );
                assert_offenses(
                    &format!("if quux\n  work {keyword} {condition}\nend\n"),
                    &[(2, 8, "if")],
                );
            }
        }
        assert_offenses(
            "class A\n  def foo\n    if h[:a]\n      h[:b] = true unless h.has_key?(:b)\n    end\n  end\nend",
            &[(4, 20, "if")],
        );
    }

    #[test]
    fn empty_and_single_line_bodies() {
        assert_offenses("if foo; if bar; end; end", &[(1, 9, "if")]);
        assert_offenses("unless foo\n  unless bar\n  end\nend", &[(2, 3, "unless")]);
        assert_offenses(
            "if foo\n  if bar\n    # Comments.\n  end\nend",
            &[(2, 3, "if")],
        );
        assert_offenses("if foo\nend", &[]);
    }

    #[test]
    fn comments_and_multiple_nesting_levels() {
        assert_offenses(
            "# Outer comment.\nif foo\n  # Comment.\n  unless bar # nested condition\n    # More comments.\n    if baz\n      do_something\n    end\n  end\nend\n",
            &[(4, 3, "if"), (6, 5, "unless")],
        );
        assert_offenses(
            "if foo.is_a? Foo\n  if bar && baz\n    do_something if quux\n  end\nend",
            &[(2, 3, "if"), (3, 18, "if")],
        );
    }

    #[test]
    fn else_elsif_ternary_and_statement_lookalikes() {
        for source in [
            "if foo\n  bar ? baz : quux\nend",
            "foo ? (work if bar) : nil",
            "if foo\n  do_something\nend",
            "if foo\n  if bar\n    work\n  end\n  more_work\nend",
            "if foo\n  more_work\n  if bar\n    work\n  end\nend",
            "if foo\n  if bar\n    work\n  else\n    other_work\n  end\nend",
            "if foo\n  work if bar\nelse\n  other_work\nend",
            "unless foo\n  if bar\n    work\n  end\nelse\n  other_work\nend",
            "if foo\nelsif bar\n  if baz\n  end\nend",
            "if foo\n  if bar\n  end\nelsif baz\n  work\nend",
            "if foo\n  if bar\n  elsif baz\n    work\n  end\nend",
            "if foo\n  (if bar\n    work\n  end)\nend",
            "if foo\n  begin\n    work if bar\n  end\nend",
            "# if foo; if bar; end; end\ntext = 'if foo; if bar; end; end'",
        ] {
            assert_offenses(source, &[]);
        }
        // A deeper eligible pair still reports inside an ineligible else/elsif.
        assert_offenses(
            "if a\nelse\n  if b\n    work if c\n  end\nend",
            &[(4, 10, "if")],
        );
        assert_offenses(
            "if a\nelsif b\n  if c\n    work if d\n  end\nend",
            &[(4, 10, "if")],
        );
    }

    #[test]
    fn assignment_exemption_is_exact_source_not_liveness() {
        for (condition, inner) in [
            ("var = foo", "(var)"),
            ("var = foo", "var && other"),
            ("var = foo", "var.valid?"),
            ("var = foo", "other"),
            ("var = foo", "!var"),
            ("var = foo", "var = bar"),
            ("(foo in var)", "var"),
            ("/x(?<var>y)/ =~ text", "var"),
            ("CONST = foo", "CONST"),
            ("obj.var = foo", "obj.var"),
            ("foo", "var"),
        ] {
            assert_offenses(
                &format!("if {condition}\n  work if {inner}\nend"),
                &[(2, 8, "if")],
            );
        }
        // The condition's source, not the variable's scope or dynamic value,
        // controls RuboCop's syntactic exemption even under a modifier.
        assert_offenses("if var\n  work\nend if var = foo", &[]);
        assert_offenses("if foo\n  var = bar if var\nend", &[(2, 13, "if")]);
        assert_offenses("if (for var in items; end)\n  work if var\nend", &[]);
        assert_offenses(
            "if (begin; work; rescue => var; end)\n  work if var\nend",
            &[],
        );
    }

    #[test]
    fn return_values_guards_and_branch_effects_still_detect_without_fixes() {
        for (source, line, col) in [
            ("result = if a\n  if b\n    42\n  end\nend", 2, 3),
            ("def f\n  if a\n    return 42 unless b\n  end\nend", 3, 15),
            ("each do\n  if a\n    next if b\n  end\nend", 3, 10),
            ("while running\n  if a\n    break if b\n  end\nend", 3, 11),
            (
                "if first_effect\n  if second_effect\n    record\n    publish\n  end\nend",
                2,
                3,
            ),
            ("def f\n  if a\n    yield if b\n  end\nend", 3, 11),
            (
                "if a\n  if b\n    begin\n      work\n    rescue\n      recover\n    ensure\n      cleanup\n    end\n  end\nend",
                2,
                3,
            ),
            (
                "begin\n  if a\n    if b\n      __LINE__\n    end\n  end\nrescue\n  recover\nensure\n  cleanup\nend",
                3,
                5,
            ),
        ] {
            assert_offenses(source, &[(line, col, "if")]);
        }
        assert_offenses(
            "if a\n  begin\n    work if b\n  rescue\n    recover\n  ensure\n    cleanup\n  end\nend",
            &[],
        );
        assert_offenses(
            "def f\n  if a\n    return if guard\n    work if b\n  end\nend",
            &[],
        );
    }

    #[test]
    fn trailing_heredocs_preserve_upstream_parse_feasibility_exemptions() {
        // Merging normal headers would delete the outer heredoc body; moving
        // an outer modifier opener would leave its body behind. Upstream's
        // correction_parses? suppresses these offenses even without --fix.
        for (opener, body) in [
            ("<<~TEXT", "hello"),
            ("<<~TEXT", "hello #{value}"),
            ("<<~`TEXT`", "hello"),
            ("<<~`TEXT`", "hello #{value}"),
        ] {
            assert_offenses(
                &format!("if foo({opener})\n  {body}\nTEXT\n  if bar\n    work\n  end\nend\n"),
                &[],
            );
        }
        assert_offenses("if foo\n  work\nend if bar(<<~TEXT)\n  hello\nTEXT\n", &[]);
        assert_offenses(
            "if bar\n  if foo(<<~TEXT)\n    hello\n  TEXT\n    work\n  end\nend\n",
            &[(2, 3, "if")],
        );
        assert_offenses(
            "if bar\n  work if foo(<<~TEXT)\n    hello\n  TEXT\nend\n",
            &[(2, 8, "if")],
        );
        assert_offenses(
            "if foo(<<~TEXT)\n  hello\nTEXT\n  work if bar\nend\n",
            &[(4, 8, "if")],
        );
        assert_offenses(
            "if foo(<<~TEXT)\n  hello\nTEXT\n  work\nend if bar\n",
            &[(1, 1, "if")],
        );
        // When the body/terminator is already inside the predicate's source
        // range, joining the headers does not cut it away.
        assert_offenses(
            "if foo(<<~TEXT\n  hello\nTEXT\n)\n  if bar\n    work\n  end\nend\n",
            &[(5, 3, "if")],
        );
    }

    #[test]
    fn command_yield_and_super_only_fail_on_the_right_of_the_merge() {
        for outer in ["if", "unless"] {
            for inner in ["if", "unless"] {
                for condition in [
                    "yield item",
                    "super item",
                    "yield item, other",
                    "super item, other",
                ] {
                    assert_offenses(
                        &format!(
                            "def f\n  {outer} enabled?\n    {inner} {condition}\n      work\n    end\n  end\nend\n"
                        ),
                        &[],
                    );
                    assert_offenses(
                        &format!(
                            "def f\n  {outer} enabled?\n    work {inner} {condition}\n  end\nend\n"
                        ),
                        &[],
                    );
                    assert_offenses(
                        &format!(
                            "def f\n  {inner} {condition}\n    work\n  end {outer} enabled?\nend\n"
                        ),
                        &[],
                    );
                    assert_offenses(
                        &format!(
                            "def f\n  {outer} {condition}\n    {inner} enabled?\n      work\n    end\n  end\nend\n"
                        ),
                        &[(3, 5, outer)],
                    );
                }
                for condition in [
                    "yield",
                    "super",
                    "yield(item)",
                    "super(item)",
                    "(yield item)",
                    "(super item)",
                    "check(yield item)",
                    "check(super item)",
                    "enabled? and yield item",
                    "enabled? and super item",
                ] {
                    assert_offenses(
                        &format!(
                            "def f\n  {outer} enabled?\n    {inner} {condition}\n      work\n    end\n  end\nend\n"
                        ),
                        &[(3, 5, outer)],
                    );
                }
            }
        }
    }

    #[test]
    fn unparenthesized_outer_patterns_cannot_be_followed_by_a_conjunction() {
        for outer in ["if", "unless"] {
            for inner in ["if", "unless"] {
                for condition in ["value in [item]", "value in item"] {
                    assert_offenses(
                        &format!("{outer} {condition}\n  {inner} enabled?\n    work\n  end\nend\n"),
                        &[],
                    );
                    assert_offenses(
                        &format!("{outer} {condition}\n  work {inner} enabled?\nend\n"),
                        &[],
                    );
                    assert_offenses(
                        &format!("{inner} enabled?\n  work\nend {outer} {condition}\n"),
                        &[],
                    );
                    assert_offenses(
                        &format!(
                            "{outer} ({condition})\n  {inner} enabled?\n    work\n  end\nend\n"
                        ),
                        &[(2, 3, outer)],
                    );
                    assert_offenses(
                        &format!("{outer} enabled?\n  {inner} {condition}\n    work\n  end\nend\n"),
                        &[(2, 3, outer)],
                    );
                }
            }
        }
    }

    #[test]
    fn modifier_removal_must_not_join_the_inner_end_to_rescue() {
        for outer in ["if", "unless"] {
            for inner in ["if", "unless"] {
                assert_offenses(
                    &format!("{inner} enabled?\n  work\nend {outer} other rescue recover\n"),
                    &[],
                );
                for condition in ["(other rescue recover)", "other"] {
                    assert_offenses(
                        &format!("{inner} enabled?\n  work\nend {outer} {condition}\n"),
                        &[(1, 1, outer)],
                    );
                }
                assert_offenses(
                    &format!("({inner} enabled?\n  work\nend {outer} other) rescue recover\n"),
                    &[(1, 2, outer)],
                );
                assert_offenses(
                    &format!("work {inner} enabled? {outer} other rescue recover\n"),
                    &[(1, 6, outer)],
                );
                assert_offenses(
                    &format!("{inner} enabled?\n  work\nend {outer} other \\\nrescue recover\n"),
                    &[(1, 1, outer)],
                );
            }
        }
    }

    #[test]
    fn wide_sibling_conditions_have_bounded_work() {
        use crate::cops::variables::CONDITION_BRANCH_VISITS;
        let source = "if check(var = foo)\n  work if other\nend\n".repeat(1000);
        CONDITION_BRANCH_VISITS.with(|count| count.set(0));
        super::HEREDOC_PREDICATE_VISITS.with(|count| count.set(0));
        assert_eq!(offenses(&source).len(), 1000);
        assert!(super::HEREDOC_PREDICATE_VISITS.with(|count| count.get()) <= 6000);
        let visits = CONDITION_BRANCH_VISITS.with(|count| count.get());
        // Other active cops can share this counter, but total work stays
        // linear in these disjoint conditions, not the whole-file size.
        assert!(visits <= 20_000, "{visits}");
    }

    #[test]
    fn branch_results_jumps_exception_bodies_and_else_association_are_unchanged() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "def f\n  if a\n    return 42 if b\n  end\nend\n",
                "def f\n  if a && b\n    return 42\n  end\nend\n",
            ),
            (
                "each do\n  if a\n    next if b\n  end\nend\n",
                "each do\n  if a && b\n    next\n  end\nend\n",
            ),
            (
                "while running\n  if a\n    break if b\n  end\nend\n",
                "while running\n  if a && b\n    break\n  end\nend\n",
            ),
            (
                "if a\nelse\n  if b\n    work if c\n  end\nend\n",
                "if a\nelse\n  if b && c\n    work\n  end\nend\n",
            ),
            (
                "if a\nelsif b\n  if c\n    work if d\n  end\nend\n",
                "if a\nelsif b\n  if c && d\n    work\n  end\nend\n",
            ),
            (
                "result = if a\n  if b\n    begin\n      work\n    rescue\n      recover\n    ensure\n      cleanup\n    end\n  end\nend\n",
                "result = if a && b\n    begin\n      work\n    rescue\n      recover\n    ensure\n      cleanup\n    end\n  end\n",
            ),
        ] {
            assert_correction(&super::SoleNestedConditional, before, after);
        }
    }

    #[test]
    fn ordinary_comments_crlf_bom_encoding_and_data_payloads_survive() {
        use crate::cops::testing::assert_correction;
        for newline in ["\n", "\r\n"] {
            let before = "\u{feff}# encoding: UTF-8\n# Outer.\nif a\n  # Inner.\n  if b # inline\n    \"é\"\n  end\nend\n__END__\npayload\n".replace('\n', newline);
            let after = "\u{feff}# encoding: UTF-8\n# Outer.\n# Inner.\nif a && b # inline\n    \"é\"\n  end\n__END__\npayload\n".replace('\n', newline);
            assert_correction(&super::SoleNestedConditional, &before, &after);
        }
    }

    #[test]
    fn all_pinned_fixed_policy_calls_and_corrections() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/sole_nested_conditional_pinned.json"
        ))
        .unwrap();
        for c in &cases {
            let source = c["source"].as_str().unwrap();
            let found = offenses(source);
            let positions: Vec<_> = found.iter().map(|o| vec![o.line, o.col]).collect();
            assert_eq!(
                serde_json::to_value(positions).unwrap(),
                c["positions"],
                "{c}"
            );
            let messages: Vec<_> = found.iter().map(|o| &o.message).collect();
            assert_eq!(
                serde_json::to_value(messages).unwrap(),
                c["messages"],
                "{c}"
            );
        }
        assert_eq!(cases.len(), 71);
    }

    #[test]
    fn canonical_normal_and_modifier_corrections() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (
                "if foo\n  if bar\n    work\n  end\nend\n",
                "if foo && bar\n    work\n  end\n",
            ),
            (
                "if foo\n  work if bar\nend\n",
                "if foo && bar\n  work\nend\n",
            ),
            (
                "if foo\n  work\nend if bar\n",
                "if bar && foo\n  work\nend\n",
            ),
            ("work if foo if bar\n", "work if bar && foo\n"),
            ("if foo; if bar; end; end\n", "if foo && bar; end; \n"),
        ] {
            assert_correction(&super::SoleNestedConditional, before, after);
        }
    }

    #[test]
    fn normal_if_and_unless_pairs() {
        for outer in ["if", "unless"] {
            for inner in ["if", "unless"] {
                assert_offenses(
                    &format!("{outer} foo\n  {inner} bar\n    do_something\n  end\nend\n"),
                    &[(2, 3, outer)],
                );
            }
        }
    }
}
