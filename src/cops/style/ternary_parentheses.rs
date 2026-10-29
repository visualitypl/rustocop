//! `Style/TernaryParentheses` — omit unnecessary grouping around ternary predicates.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;

pub struct TernaryParentheses;
impl Cop for TernaryParentheses {
    fn name(&self) -> &'static str {
        "Style/TernaryParentheses"
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
        let Some(n) = node.as_if_node().filter(|n| n.if_keyword_loc().is_none()) else {
            return;
        };
        let condition = n.predicate();
        let Some(p) = condition.as_parentheses_node() else {
            return;
        };
        let loc = condition.location();
        // Query the cached line index rather than splitting every nested predicate.
        let last_start = file
            .line_start(file.last_line_of(condition.location()))
            .max(loc.start_offset());
        if &file.bytes[last_start..loc.end_offset()] == b")" {
            return;
        }
        let body = p.body().and_then(|b| b.as_statements_node());
        let first = body.as_ref().and_then(|b| b.body().first());
        if first.as_ref().is_some_and(|first| {
            first
                .as_if_node()
                .is_some_and(|n| n.if_keyword_loc().is_some() && n.end_keyword_loc().is_none())
                || first
                    .as_unless_node()
                    .is_some_and(|n| n.end_keyword_loc().is_none())
                || first.as_match_predicate_node().is_some()
                || (body.as_ref().is_some_and(|b| b.body().len() == 1) && safe_assignment(first))
        }) {
            return;
        }
        let correction = cx
            .corrections_requested()
            .then(|| {
                let body = body.as_ref()?;
                if body.body().len() != 1 {
                    return None;
                }
                rewrite(&condition, &first?, cx, file)
            })
            .flatten();
        let (line, col) = file.line_col(node.location().start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.last_line_of(node.location()),
            message: "Omit parentheses for ternary conditions.".into(),
            correction,
        });
    }
}

fn safe_assignment(n: &Node<'_>) -> bool {
    matches!(
        n,
        Node::LocalVariableWriteNode { .. }
            | Node::InstanceVariableWriteNode { .. }
            | Node::ClassVariableWriteNode { .. }
            | Node::GlobalVariableWriteNode { .. }
            | Node::ConstantWriteNode { .. }
            | Node::ConstantPathWriteNode { .. }
            | Node::MultiWriteNode { .. }
    ) || n.as_call_node().is_some_and(|c| c.equal_loc().is_some())
}

// A bounded precedence proof on the main parse, not a per-candidate reparse.
// Delimited arguments/collections/groupings keep their own binding. Only the
// exposed symbolic operators need recursive checking; English operators,
// assignments, control flow, range endpoints and command operands do not qualify.
fn tight(n: &Node<'_>, depth: usize) -> bool {
    if depth == 64 {
        return false;
    }
    if matches!(n, Node::RangeNode { .. }) {
        return false;
    }
    if ast::is_literal(n)
        || ast::is_variable(n)
        || ast::is_const(n)
        || matches!(n, Node::SelfNode { .. } | Node::ParenthesesNode { .. })
    {
        return true;
    }
    if let Some(n) = n.as_and_node() {
        return n.operator_loc().as_slice() == b"&&"
            && tight(&n.left(), depth + 1)
            && tight(&n.right(), depth + 1);
    }
    if let Some(n) = n.as_or_node() {
        return n.operator_loc().as_slice() == b"||"
            && tight(&n.left(), depth + 1)
            && tight(&n.right(), depth + 1);
    }
    if let Some(c) = n.as_call_node() {
        if c.equal_loc().is_some()
            || c.block().is_some()
            || c.message_loc().is_some_and(|l| l.as_slice() == b"not")
        {
            return false;
        }
        if !c.receiver().is_none_or(|r| tight(&r, depth + 1)) {
            return false;
        }
        if c.opening_loc().is_some() || c.arguments().is_none() {
            return true;
        }
        return c.call_operator_loc().is_none()
            && ast::is_operator_method(c.name().as_slice())
            && c.arguments()
                .is_some_and(|a| a.arguments().iter().all(|n| tight(&n, depth + 1)));
    }
    if let Some(n) = n.as_defined_node() {
        return n.lparen_loc().is_some();
    }
    if let Some(n) = n.as_yield_node() {
        return n.arguments().is_none() || n.lparen_loc().is_some();
    }
    false
}

fn rewrite(
    condition: &Node<'_>,
    inner: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    let loc = condition.location();
    let range = loc.start_offset()..loc.end_offset();
    // Newlines or semicolons hidden by the removed delimiters can terminate
    // the predicate, e.g. `(ready\n  ) ? a : b` or `(ready;) ? a : b`.
    if range.len() > 16 * 1024
        || super::correction::contains_comment(file, range.clone())
        || !file.bytes[range.start + 1..inner.location().start_offset()]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t'))
        || !file.bytes[inner.location().end_offset()..range.end - 1]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t'))
    {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    if file.contains_heredoc(root, condition) || file.heredoc_pending_at(root, range.start) {
        return None;
    }
    // Grouping can immediately follow a keyword (e.g. `return(true) ?`).
    // Preserve its lexical boundary instead of creating `returntrue`, which
    // still parses but changes control flow into an unrelated method call.
    let needs_separator = range
        .start
        .checked_sub(1)
        .and_then(|i| file.bytes.get(i))
        .is_some_and(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'?' | b'!') || *b >= 0x80
        })
        && !file.bytes[range.start + 1].is_ascii_whitespace();
    let mut edits = vec![
        Correction {
            range: range.start..range.start + 1,
            replacement: if needs_separator {
                " ".into()
            } else {
                String::new()
            },
        },
        Correction {
            range: range.end - 1..range.end,
            replacement: if file
                .bytes
                .get(range.end)
                .is_some_and(u8::is_ascii_whitespace)
            {
                String::new()
            } else {
                " ".into()
            },
        },
    ];
    // Command arguments would otherwise absorb the ternary. Make their binding
    // explicit while preserving argument bytes, evaluation order and count.
    let command = if let Some(c) = inner.as_call_node() {
        let name = c.name().as_slice();
        if c.equal_loc().is_none()
            && c.block().is_none()
            && c.opening_loc().is_none()
            && (c.call_operator_loc().is_some()
                || name.first().is_some_and(u8::is_ascii_alphabetic))
            && c.arguments().is_some()
            && c.receiver().is_none_or(|r| tight(&r, 0))
        {
            let args = c.arguments()?;
            Some((
                c.message_loc()?.end_offset(),
                args.arguments().first()?.location().start_offset(),
                args.arguments().iter().last()?.location().end_offset(),
            ))
        } else {
            None
        }
    } else {
        inner
            .as_defined_node()
            .filter(|d| d.lparen_loc().is_none())
            .map(|d| {
                (
                    d.keyword_loc().end_offset(),
                    d.value().location().start_offset(),
                    d.value().location().end_offset(),
                )
            })
    };
    if let Some((selector_end, start, end)) = command {
        if !file.bytes[selector_end..start]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t'))
        {
            return None;
        }
        edits.push(Correction {
            range: selector_end..start,
            replacement: "(".into(),
        });
        edits.push(Correction {
            range: end..end,
            replacement: ")".into(),
        });
    } else if !tight(inner, 0) {
        return None;
    }
    #[cfg(test)]
    REWRITE_BYTES.with(|n| n.set(n.get() + range.len()));
    let edit = super::correction::combine(file, edits)?;
    super::negated_if::finish(edit, cx, file)
}

#[cfg(test)]
thread_local! { static REWRITE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    //! RuboCop ternary_parentheses_spec.rb examples (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{
        cops::testing::{assert_correction, correction_offenses},
        pipeline::lint_str,
    };

    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Style/TernaryParentheses")
            .collect()
    }

    #[test]
    fn pinned_detection_and_canonical_corrections() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/ternary_parentheses_pinned.json"
        ))
        .unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 45);
        let mut corrections = 0;
        for case in cases.as_array().unwrap() {
            let source = case["source"].as_str().unwrap();
            // The pin includes two invalid top-level yield examples. Test them
            // in a method as well, without changing Ruby's parser contract.
            let invalid = ruby_prism::parse(source.as_bytes()).errors().count() > 0;
            let wrapped = format!("def f\n{source}end\n");
            let before = if invalid { &wrapped } else { source };
            let found = offenses(before);
            let positions: Vec<_> = case["positions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p[0].as_u64().unwrap() as u32 + u32::from(invalid),
                        p[1].as_u64().unwrap() as u32,
                    )
                })
                .collect();
            assert_eq!(
                found.iter().map(|o| (o.line, o.col)).collect::<Vec<_>>(),
                positions,
                "{case}"
            );
            for (o, msg) in found.iter().zip(case["messages"].as_array().unwrap()) {
                assert_eq!(o.message, msg.as_str().unwrap());
            }
            if let Some(after) = case["correction"].as_str() {
                corrections += 1;
                let wrapped_after = format!("def f\n{after}end\n");
                assert_correction(
                    &TernaryParentheses,
                    before,
                    if invalid { &wrapped_after } else { after },
                );
            } else {
                assert!(
                    correction_offenses(&TernaryParentheses, before)
                        .iter()
                        .all(|o| o.correction.is_none()),
                    "{case}"
                );
            }
        }
        assert_eq!(corrections, 27);
    }

    #[test]
    fn accepts_assignments_modifiers_patterns_and_lookalikes() {
        for source in [
            "(x = work) ? x : fallback",
            "(@x = work) ? @x : fallback",
            "(X = work) ? a : b",
            "(obj.x = work) ? a : b",
            "(obj[0] = work) ? a : b",
            "(a,b = work) ? a : b",
            "(work if ready) ? a : b",
            "(work unless ready) ? a : b",
            "(object.value=(value)) ? a : b",
            "(obj in Integer) ? a : b",
            "begin; work; end ? a : b",
            "work ? (a) : (b)",
            "if (ready); a; else; b; end",
            "(foo..bar).include?(baz) ? a : b",
            "(\n  ready\n) ? a : b",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }

    #[test]
    fn keyword_adjacent_predicates_keep_token_separation() {
        for keyword in ["return", "next", "break"] {
            for predicate in ["true", "false", "nil", "1", "ready?", "(true)"] {
                let before =
                    format!("def f; loop {{ {keyword}({predicate}) ? :yes : :no }}; end\n");
                let after = format!("def f; loop {{ {keyword} {predicate} ? :yes : :no }}; end\n");
                assert_correction(&TernaryParentheses, &before, &after);
            }
        }
    }

    #[test]
    fn synthetic_corrections_preserve_the_normalized_main_ast() {
        let mut corrected = 0;
        for predicate in [
            "foo",
            "foo?",
            "foo bar",
            "foo *bar",
            "foo bar: 1",
            "obj&.foo bar, baz",
            "obj.< 10",
            "foo(bar)",
            "(foo bar)",
            "foo && bar",
            "foo || bar = 1",
            "foo and bar",
            "foo or bar",
            "not foo",
            "!foo bar",
            "defined? foo",
            "defined?(foo)",
            "yield",
            "yield 1",
            "yield(1)",
            "super",
            "super 1",
            "a = 1",
            "a += 1",
            "a ||= 1",
            "a &&= 1",
            "foo; bar",
            ";foo",
            "foo;",
            "foo rescue bar",
            "foo ? bar : baz",
            "foo if bar",
            "foo unless bar",
            "foo while bar",
            "begin; foo; rescue; bar; ensure; baz; end",
            "foo { bar }",
            "-> { foo }",
            "[foo]",
            "{a: foo}",
            "'foo'",
            "1..2",
            "1..",
            "foo in Integer",
            "__LINE__",
            "__ENCODING__",
            "nil",
            "true",
            "false",
            "self",
            "@a",
            "A::B",
            "foo ||\n bar",
            "\nfoo",
            "foo\n  ",
            "foo # keep\n  ",
            "foo &bar",
        ] {
            for template in [
                format!("def f; a = ({predicate}) ? x : y; end\n"),
                format!("def f; consume(({predicate}) ? x : y); end\n"),
                format!("def f; ({predicate})? x : (guard ? y : z); end\n"),
                format!("def f; return({predicate}) ? x : y; end\n"),
                format!("def f; loop {{ next({predicate}) ? x : y }}; end\n"),
                format!("def f; loop {{ break({predicate}) ? x : y }}; end\n"),
                format!("def f; not({predicate}) ? x : y; end\n"),
                format!("def f; defined?({predicate}) ? x : y; end\n"),
            ] {
                let parsed = ruby_prism::parse(template.as_bytes());
                if parsed.errors().count() != 0 {
                    continue;
                }
                let found = correction_offenses(&TernaryParentheses, &template);
                let applied = crate::corrections::apply(template.as_bytes(), &found).unwrap();
                if applied.bytes == template.as_bytes() {
                    continue;
                }
                corrected += 1;
                let reparsed = ruby_prism::parse(&applied.bytes);
                assert_eq!(
                    reparsed.errors().count(),
                    0,
                    "{template} -> {}",
                    String::from_utf8_lossy(&applied.bytes)
                );
                assert_eq!(
                    ast::fingerprint_ignoring_grouping(&parsed.node(), template.as_bytes()),
                    ast::fingerprint_ignoring_grouping(&reparsed.node(), &applied.bytes),
                    "{template} -> {}",
                    String::from_utf8_lossy(&applied.bytes)
                );
            }
        }
        assert!(corrected >= 75, "{corrected}");
    }

    #[test]
    fn correction_work_is_lazy_and_bounded() {
        let source = "result = (ready) ? yes : no\n".repeat(3000);
        REWRITE_BYTES.with(|n| n.set(0));
        assert_eq!(offenses(&source).len(), 3000);
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 0));
        assert_eq!(
            correction_offenses(&TernaryParentheses, &source).len(),
            3000
        );
        REWRITE_BYTES.with(|n| assert_eq!(n.get(), 7 * 3000));
        // The pipeline rejects depth >64 before dispatch. Exercise this
        // helper's independent bound directly on Prism's main parse.
        let deep = vec!["ready"; 100].join(" && ");
        let parsed = ruby_prism::parse(deep.as_bytes());
        let expression = parsed
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .first()
            .unwrap();
        assert!(!tight(&expression, 0));
        let huge = format!("(ready{}) ? yes : no", " ".repeat(17000));
        assert!(
            correction_offenses(&TernaryParentheses, &huge)[0]
                .correction
                .is_none()
        );
    }

    #[test]
    fn source_envelopes_and_literal_interiors_are_verbatim() {
        assert_correction(
            &TernaryParentheses,
            "\u{feff}# coding: utf-8\r\n(\"é\r\n  value\") ? __LINE__ : false\r\n__END__\r\npayload\x00\r\n",
            "\u{feff}# coding: utf-8\r\n\"é\r\n  value\" ? __LINE__ : false\r\n__END__\r\npayload\x00\r\n",
        );
        for source in [
            "(DATA.pos) ? a : b\n__END__\npayload\n",
            "(method(:f).source_location) ? a : b\n",
            "(ready) ? a : b # rubocop:disable Lint/Debugger\n",
            "(<<~H) ? a : b\nbody\nH\n",
        ] {
            let found = correction_offenses(&TernaryParentheses, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn branch_boundaries_guards_and_wrappers_are_untouched() {
        for (source, after) in [
            (
                "if guard\n  (check) ? first : second\nelsif other\n  work\nelse\n  fallback\nend\n",
                "if guard\n  check ? first : second\nelsif other\n  work\nelse\n  fallback\nend\n",
            ),
            (
                "def f; (check) ? (return first) : (return second); end\n",
                "def f; check ? (return first) : (return second); end\n",
            ),
            (
                "begin; (check) ? first : second; rescue; recover; ensure; cleanup; end\n",
                "begin; check ? first : second; rescue; recover; ensure; cleanup; end\n",
            ),
            (
                "(check) ? (nested) ? first : second : fallback\n",
                "check ? nested ? first : second : fallback\n",
            ),
            (
                "((value = work)) ? value : fallback\n",
                "(value = work) ? value : fallback\n",
            ),
        ] {
            assert_correction(&TernaryParentheses, source, after);
        }
        for source in [
            "(begin; check; rescue; recover; ensure; cleanup; end) ? a : b",
            "(foo { work }) ? a : b",
            "(1..2) ? a : b",
        ] {
            let found = correction_offenses(&TernaryParentheses, source);
            assert_eq!(found.len(), 1);
            assert!(found[0].correction.is_none(), "{source}");
        }
    }

    #[test]
    fn explicit_setter_calls_are_not_safe_assignment_syntax() {
        let found = offenses("(object.[]=(0, value)) ? a : b");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (1, 1));
        assert_eq!(found[0].message, "Omit parentheses for ternary conditions.");
    }

    #[test]
    fn retains_low_precedence_and_control_flow_without_hiding_offenses() {
        for source in [
            "(a and b) ? x : y",
            "(a or b) ? x : y",
            "(not a) ? x : y",
            "(a += 1) ? x : y",
            "(a ||= b) ? x : y",
            "(a && b = work) ? x : y",
            "(a ? b : c) ? x : y",
            "(one; two) ? x : y",
            "(a rescue b) ? x : y",
            "(!foo bar) ? x : y",
            "(foo # keep\n  ) ? x : y",
            "() ? x : y",
            "(foo\n  ) ? x : y",
            "(\nfoo) ? x : y",
            "(1..) ? x : y",
            "(foo;) ? x : y",
            "answer = (;foo) ? x : y",
        ] {
            let found = correction_offenses(&TernaryParentheses, source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].correction.is_none(), "{source}");
        }
    }
}
