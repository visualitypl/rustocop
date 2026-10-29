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
                .then(|| correction(node, &outer, &inner, cx, file))
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

const REWRITE_BUDGET: usize = 16 * 1024;
#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

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

fn correction(
    node: &Node<'_>,
    outer: &Conditional<'_>,
    inner: &Conditional<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let span = range(&node.location());
    if span.len() > REWRITE_BUDGET
        || super::correction::touches_directive(file, span.clone())
        // Moving predicates can change values *inside* the replacement even
        // when its aggregate newline count stays the same.
        || file.bytes[span.clone()].windows(8).any(|s| s == b"__LINE__")
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, node) || file.heredoc_pending_at(root, span.start) {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + span.len()));
    // Normal headers retain lexical order. Moving a modifier predicate before
    // its body can change Ruby's parser-local declarations (including patterns
    // and captures), so retain the finding for every uncertain declaration.
    struct Hazards {
        declarations: bool,
        control: bool,
    }
    impl NodeVisitor<'_> for Hazards {
        fn enter(&mut self, n: Node<'_>) {
            self.declarations |= variables::is_local_assignment(&n)
                || matches!(
                    n,
                    Node::MatchWriteNode { .. }
                        | Node::MatchPredicateNode { .. }
                        | Node::MatchRequiredNode { .. }
                        | Node::ForNode { .. }
                        | Node::RescueNode { .. }
                );
            self.control |= matches!(
                n,
                Node::FlipFlopNode { .. }
                    | Node::ReturnNode { .. }
                    | Node::BreakNode { .. }
                    | Node::NextNode { .. }
                    | Node::RedoNode { .. }
                    | Node::RetryNode { .. }
            );
        }
    }
    let mut hazards = Hazards {
        declarations: false,
        control: false,
    };
    if outer.modifier || inner.modifier {
        walk(&mut hazards, node);
        if hazards.declarations {
            return None;
        }
    }
    // Predicate jumps/flip-flops need separate context proofs; branch jumps
    // themselves are untouched and are safe to keep.
    hazards.control = false;
    walk(&mut hazards, &outer.predicate);
    walk(&mut hazards, &inner.predicate);
    if hazards.control {
        return None;
    }
    // The removed conditional loses its keyword-to-predicate gap as well as
    // the keyword itself. These comments cannot be safely lifted like the
    // standalone comments between normal headers below.
    let removed = if outer.modifier { outer } else { inner };
    if super::correction::contains_comment(
        file,
        removed.keyword.end_offset()..removed.predicate.location().start_offset(),
    ) {
        return None;
    }
    let left = chainable(outer, false, span.start, None, cx, file)?;
    let right = chainable(inner, true, span.start, Some(&outer.predicate), cx, file)?;
    let mut edits = Vec::new();
    if outer.modifier {
        edits.push(edit(range(&inner.keyword), "if"));
        edits.push(edit(
            range(&inner.predicate.location()),
            format!("{left} && {right}"),
        ));
        edits.push(edit(modifier_range(outer, file), ""));
    } else {
        edits.push(edit(range(&outer.keyword), "if"));
        if inner.modifier {
            edits.push(edit(
                range(&outer.predicate.location()),
                format!("{left} && {right}"),
            ));
            edits.push(edit(modifier_range(inner, file), ""));
        } else {
            let between = outer.predicate.location().end_offset()..inner.keyword.start_offset();
            let comments = file.comments();
            let first = comments.partition_point(|c| c.end <= between.start);
            let mut lifted = String::new();
            let indent = &file.bytes[file.line_start(file.line_of(outer.keyword.start_offset()))
                ..outer.keyword.start_offset()];
            if comments.get(first).is_some_and(|c| c.start < between.end)
                && (indent.len() > REWRITE_BUDGET
                    || !indent.iter().all(|b| matches!(b, b' ' | b'\t')))
            {
                return None;
            }
            for c in comments[first..]
                .iter()
                .take_while(|c| c.start < between.end)
            {
                // Only ordinary standalone comments may move above the header.
                // Inline, embedded-document and magic comments stay put.
                if !file.bytes[file.line_start(file.line_of(c.start))..c.start]
                    .iter()
                    .all(|b| matches!(b, b' ' | b'\t'))
                    || !file.bytes[c.clone()].starts_with(b"#")
                    || !super::correction::comment_relocation_safe(&file.bytes[c.clone()])
                {
                    return None;
                }
                // Prism includes a CR in a CRLF comment range, but not LF.
                // Keep its exact bytes, including mixed line-ending files.
                lifted.push_str(std::str::from_utf8(&file.bytes[c.clone()]).ok()?);
                lifted.push('\n');
                lifted.push_str(std::str::from_utf8(indent).ok()?);
                if lifted.len() > REWRITE_BUDGET {
                    return None;
                }
            }
            if !lifted.is_empty() {
                // Merge with keyword replacement so equal-offset edits cannot
                // reorder the comment and keyword accidentally.
                edits[0].replacement = format!("{lifted}if");
            }
            edits.push(edit(
                outer.predicate.location().start_offset()..inner.predicate.location().end_offset(),
                format!("{left} && {right}"),
            ));
            let end = outer.end.as_ref()?;
            let end_range = range(end);
            let line_start = file.line_start(file.line_of(end_range.start));
            let line_end = file.line_start(file.line_of(end_range.end) + 1);
            let standalone = file.bytes[line_start..end_range.start]
                .iter()
                .all(|b| matches!(b, b' ' | b'\t'))
                && file.bytes[end_range.end..line_end]
                    .iter()
                    .all(u8::is_ascii_whitespace);
            if standalone
                && file.line_of(end_range.start) != file.line_of(inner.end.as_ref()?.start_offset())
            {
                edits.push(edit(line_start..line_end, ""));
            } else if file.line_of(end_range.start)
                == file.line_of(inner.end.as_ref()?.start_offset())
            {
                edits.push(edit(end_range, ""));
            } else {
                return None;
            }
        }
    }
    let edit = super::correction::combine(file, edits)?;
    if file.class_method_bindings(root).observes_sources
        || !super::correction::preserves_line_values(file, &edit)
        || !super::correction::preserves_data_positions(file, cx, &edit)
    {
        return None;
    }
    Some(edit)
}

fn modifier_range(n: &Conditional<'_>, file: &SourceFile) -> std::ops::Range<usize> {
    let mut start = n.keyword.start_offset();
    while start > 0 && matches!(file.bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    start..n.predicate.location().end_offset()
}

fn chainable(
    n: &Conditional<'_>,
    rhs: bool,
    before: usize,
    earlier: Option<&Node<'_>>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<String> {
    let negate = n.keyword.as_slice() == b"unless";
    if negate {
        // unless uses truthiness, not ! dispatch. Only closed immediate values
        // before effects can safely acquire the new method call.
        let root = cx.ancestors().next_back()?;
        if !super::case_equality::immediate(&n.predicate)
            || earlier.is_some_and(|p| !super::case_equality::immediate(p))
            || !file.negated_condition_safety(root).before_effects(before)
            || cx.ancestors().any(|a| {
                matches!(
                    a,
                    Node::DefNode { .. }
                        | Node::BlockNode { .. }
                        | Node::LambdaNode { .. }
                        | Node::WhileNode { .. }
                        | Node::UntilNode { .. }
                        | Node::ForNode { .. }
                        | Node::RescueNode { .. }
                        | Node::EnsureNode { .. }
                ) || a
                    .as_begin_node()
                    .is_some_and(|b| b.rescue_clause().is_some() || b.ensure_clause().is_some())
            })
        {
            return None;
        }
    }
    let source = operand(&n.predicate, rhs)?;
    Some(if negate { format!("!{source}") } else { source })
}

fn operand(n: &Node<'_>, rhs: bool) -> Option<String> {
    let source = text(n)?;
    if let Some(a) = n.as_and_node() {
        let left = a.left();
        let right = a.right();
        let gap = &source[left.location().end_offset() - n.location().start_offset()
            ..right.location().start_offset() - n.location().start_offset()];
        let joined = format!("{}{gap}{}", text(&left)?, operand(&right, false)?);
        // `a && b and c` is not a valid RHS without grouping: c could run
        // even when the outer condition is false.
        return Some(if rhs && a.operator_loc().as_slice() == b"and" {
            format!("({joined})")
        } else {
            joined
        });
    }
    let mut wrap = super::redundant_parentheses::is_assignment(n)
        || n.as_or_node().is_some()
        || n.as_rescue_modifier_node().is_some()
        || n.as_if_node().is_some()
        || n.as_unless_node().is_some()
        || n.as_match_predicate_node().is_some()
        || n.as_match_required_node().is_some()
        || n.as_defined_node()
            .is_some_and(|d| d.lparen_loc().is_none());
    if let Some(c) = n.as_call_node() {
        wrap |= c.block().is_some()
            || c.equal_loc().is_some()
            || c.name().as_slice() == b"!"
                && c.message_loc().is_some_and(|l| l.as_slice() == b"not");
        if c.arguments().is_some_and(|a| !a.arguments().is_empty()) && c.opening_loc().is_none() {
            if !ast::is_operator_method(c.name().as_slice())
                && c.block().is_none()
                && c.equal_loc().is_none()
            {
                let at = c.message_loc()?.end_offset() - n.location().start_offset();
                return Some(format!("{}({})", &source[..at], source[at..].trim_start()));
            }
            wrap = true;
        }
    }
    // A command yield/super on the left is parseable without grouping, but
    // would absorb `&& rhs` into its last argument instead of guarding it.
    wrap |= unparenthesized_keyword_arguments(n);
    Some(if wrap {
        format!("({source})")
    } else {
        source.into()
    })
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
    fn outer_command_yield_and_super_do_not_absorb_the_new_condition() {
        use crate::cops::testing::assert_correction;
        for keyword in ["yield", "super"] {
            assert_correction(
                &super::SoleNestedConditional,
                &format!(
                    "def f(item)\n  if {keyword} item\n    if enabled?\n      work\n    end\n  end\nend\n"
                ),
                &format!(
                    "def f(item)\n  if ({keyword} item) && enabled?\n      work\n    end\nend\n"
                ),
            );
        }
    }

    #[test]
    fn predicate_precedence_assignments_commands_and_blocks() {
        use crate::cops::testing::assert_correction;
        for (left, right, merged) in [
            ("foo || bar", "baz || qux", "(foo || bar) && (baz || qux)"),
            ("foo", "bar and baz", "foo && (bar and baz)"),
            ("foo", "value = bar and baz", "foo && (value = bar and baz)"),
            ("value = bar and baz", "foo", "value = bar and baz && foo"),
            ("foo", "bar && value = baz", "foo && bar && (value = baz)"),
            ("not foo", "bar", "(not foo) && bar"),
            ("foo", "obj&.ok? bar", "foo && obj&.ok?(bar)"),
            (
                "foo",
                "check arg do\n  work\nend",
                "foo && (check arg do\n  work\nend)",
            ),
            (
                "foo",
                "check do\n  work\nend",
                "foo && (check do\n  work\nend)",
            ),
            ("foo", "value in [item]", "foo && (value in [item])"),
        ] {
            assert_correction(
                &super::SoleNestedConditional,
                &format!("if {left}\n  if {right}\n    work\n  end\nend\n"),
                &format!("if {merged}\n    work\n  end\n"),
            );
        }
    }

    #[test]
    fn uncertain_individual_edits_keep_findings() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "if a\n  x = 1 if x\nend\n",
            "if a\n  work if (value in [item])\nend\n",
            "if a\n  work if /(?<var>x)/ =~ text\nend\n",
            "if var\n  work\nend if var = foo\n", // detection exemption
            "if a\n  if b\n    __LINE__\n  end\nend\n",
            "if a\n  if b\n    work\n  end\nend\np __LINE__\n",
            "if a # keep\n  if b\n    work\n  end\nend\n",
            "if a\n  # frozen_string_literal: true\n  if b\n    work\n  end\nend\n",
            "if a\n  if b\n    work\n  end\nend # keep\n",
            "if a\n  if b\n    <<~TEXT\nhello\nTEXT\n  end\nend\n",
            "if a\n  if b\n    DATA.read\n  end\nend\n__END__\npayload\n",
            "if a\n  if b\n    File.read(__FILE__)\n  end\nend\n",
            "if a\n  if b\n    work\n  end\nend\np $0\n",
        ] {
            let found = correction_offenses(&super::SoleNestedConditional, source);
            assert!(
                found.iter().all(|o| o.correction.is_none()),
                "{source}: {found:?}"
            );
            if !source.contains("if var") {
                assert_eq!(found.len(), 1, "{source}");
            }
        }
    }

    #[test]
    fn closed_unless_is_corrected_but_unknown_or_reexecuted_negation_is_not() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        for (before, after) in [
            (
                "unless false\n  work if true\nend\n",
                "if !false && true\n  work\nend\n",
            ),
            (
                "if true\n  work unless false\nend\n",
                "if true && !false\n  work\nend\n",
            ),
        ] {
            assert_correction(&super::SoleNestedConditional, before, after);
        }
        for source in [
            "unless foo\n  work if bar\nend\n",
            "if foo\n  work unless bar\nend\n",
            "class FalseClass; def !; false; end; end\nunless false\n  work if true\nend\n",
            "effect\nunless false\n  work if true\nend\n",
            "while running\n  unless false\n    work if true\n  end\nend\n",
            "def f\n  unless false\n    work if true\n  end\nend\n",
            "begin\n  unless false\n    work if true\n  end\nrescue\n  retry\nend\n",
            "begin\n  unless false\n    work if true\n  end\nensure\n  cleanup\nend\n",
        ] {
            let found = correction_offenses(&super::SoleNestedConditional, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn comments_in_removed_keyword_to_predicate_gaps_withhold_corrections() {
        use crate::cops::testing::correction_offenses;
        for source in [
            "if true\n  if # keep this explanation\n    true\n    puts :ok\n  end\nend\n",
            "if true\n  if\n    # keep this explanation\n    true\n    puts :ok\n  end\nend\n",
            "if true\n  puts :ok if # important explanation\n    true\nend\n",
            "if true\n  puts :ok\nend if # important explanation\n  true\n",
            "puts :ok if true if # important explanation\n  true\n",
        ] {
            for newline in ["\n", "\r\n"] {
                let source = source.replace('\n', newline);
                let found = correction_offenses(&super::SoleNestedConditional, &source);
                assert_eq!(found.len(), 1, "{source}: {found:?}");
                assert!(found[0].correction.is_none(), "{source}: {found:?}");
            }
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
    fn correction_work_is_lazy_and_bounded_for_wide_and_large_bodies() {
        use crate::cops::testing::correction_offenses;
        let source = "if a\n  if b\n    work\n  end\nend\n".repeat(1000);
        super::REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(offenses(&source).len(), 1000);
        assert_eq!(super::REWRITE_BYTES.with(|n| n.get()), 0);
        let found = correction_offenses(&super::SoleNestedConditional, &source);
        assert!(found.iter().all(|o| o.correction.is_some()));
        assert!(super::REWRITE_BYTES.with(|n| n.get()) <= source.len());
        let source = format!(
            "if a\n  if b\n    work # {}\n  end\nend\n",
            "x".repeat(super::REWRITE_BUDGET)
        );
        super::REWRITE_BYTES.with(|n| n.set(0));
        let found = correction_offenses(&super::SoleNestedConditional, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
        assert_eq!(super::REWRITE_BYTES.with(|n| n.get()), 0);
        // The node location excludes leading indentation. Do not repeatedly
        // scan/copy an unbounded prefix while lifting many small comments.
        let source = format!(
            "{}if a\n  # comment\n  if b\n    work\n  end\nend\n",
            " ".repeat(super::REWRITE_BUDGET + 1)
        );
        let found = correction_offenses(&super::SoleNestedConditional, &source);
        assert_eq!(found.len(), 1);
        assert!(found[0].correction.is_none());
    }

    #[test]
    fn all_pinned_fixed_policy_calls_and_corrections() {
        use crate::cops::testing::{assert_correction, correction_offenses};
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/sole_nested_conditional_pinned.json"
        ))
        .unwrap();
        let mut counts = [0; 3];
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
            let Some(after) = c["correction"].as_str() else {
                continue;
            };
            let line = c["spec_line"].as_u64().unwrap();
            // unless is truthiness; adding ! to an unknown send introduces
            // overridable dispatch, including before/after side effects.
            if [
                24, 41, 58, 250, 284, 326, 341, 365, 445, 461, 477, 493, 509, 525, 541, 813, 828,
                873, 888, 903, 918, 935, 952, 1198,
            ]
            .contains(&line)
            {
                counts[1] += 1;
                assert!(
                    correction_offenses(&super::SoleNestedConditional, source)
                        .iter()
                        .all(|o| o.correction.is_none()),
                    "{c}"
                );
            } else {
                let single_pass = match line {
                    556 => "if foo && bar\n    if baz\n      do_something\n    end\n  end\n",
                    576 => "if foo.is_a?(Foo) && bar && baz\n    do_something if quux\n  end\n",
                    613 => {
                        "# Comments.\nif foo && bar\n    # More comments.\n    if baz\n      do_something\n    end\n  end\n"
                    }
                    _ => after,
                };
                counts[usize::from(single_pass != after) * 2] += 1;
                assert_correction(&super::SoleNestedConditional, source, single_pass);
                if single_pass != after {
                    assert_correction(&super::SoleNestedConditional, single_pass, after);
                }
            }
        }
        assert_eq!(cases.len(), 71);
        assert_eq!(counts, [36, 24, 3]);
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
