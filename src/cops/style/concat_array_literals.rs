//! `Style/ConcatArrayLiterals` — unsafe correction (mutation/dispatch).
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense, single_line},
    source::SourceFile,
};
use ruby_prism::Node;
pub struct ConcatArrayLiterals;
impl Cop for ConcatArrayLiterals {
    fn name(&self) -> &'static str {
        "Style/ConcatArrayLiterals"
    }
    fn unsafe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        node.as_call_node().is_some()
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if call.name().as_slice() != b"concat" {
            return;
        }
        let args = ast::call_arguments(&call);
        if args.is_empty() || args.iter().any(|a| a.as_array_node().is_none()) {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let end = crate::cops::bundler::ordered_gem_node::send_end(&call);
        let current = single_line(&file.bytes[selector.start_offset()..end]);
        let mut elements = Vec::new();
        let mut dynamic = false;
        let mut percent_any = false;
        for arg in &args {
            let array = arg.as_array_node().unwrap();
            let percent = array
                .opening_loc()
                .is_some_and(|l| l.as_slice().starts_with(b"%"));
            percent_any |= percent;
            for element in array.elements().iter() {
                if percent {
                    if let Some(s) = element.as_string_node() {
                        let mut out = Vec::new();
                        crate::cops::lint::duplicate_match_pattern::inspect_source(
                            s.unescaped(),
                            file,
                            &mut out,
                        );
                        elements.push(String::from_utf8_lossy(&out).into_owned());
                    } else if let Some(s) = element.as_symbol_node() {
                        let out = crate::cops::lint::symbol_conversion::inspect_symbol(
                            s.unescaped(),
                            file,
                            false,
                        );
                        elements.push(String::from_utf8_lossy(&out).into_owned());
                    } else {
                        dynamic = true;
                    }
                } else {
                    elements
                        .push(String::from_utf8_lossy(element.location().as_slice()).into_owned());
                }
            }
        }
        let prefer = format!("push({})", elements.join(", "));
        let correction = if !cx.corrections_requested() || (percent_any && dynamic) {
            None
        } else if percent_any
            || args
                .iter()
                .any(|a| a.as_array_node().unwrap().elements().is_empty())
        {
            // rebuild_call, unless a comment inside would be lost.
            let comment = file
                .comments()
                .iter()
                .any(|c| selector.start_offset() <= c.start && c.end <= end);
            (!comment).then(|| Correction {
                range: selector.start_offset()..end,
                replacement: prefer.clone(),
            })
        } else {
            remove_brackets(&call, &args, &selector, end, file)
        };
        let message = if dynamic {
            format!(
                "Use `push` with elements as arguments without array brackets instead of `{current}`."
            )
        } else {
            format!(
                "Use `{}` instead of `{current}`.",
                single_line(prefer.as_bytes())
            )
        };
        let (line, col) = file.line_col(selector.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1)),
            correction,
        });
    }
}
/// Upstream's `remove_brackets`: `concat` becomes `push` and the brackets
/// go, unless something other than commas and whitespace precedes a `]`.
fn remove_brackets(
    call: &ruby_prism::CallNode<'_>,
    args: &[Node<'_>],
    selector: &ruby_prism::Location<'_>,
    end: usize,
    file: &SourceFile,
) -> Option<Correction> {
    let span = |l: &ruby_prism::Location<'_>| l.start_offset()..l.end_offset();
    let mut edits = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        let array = arg.as_array_node()?;
        let close = array.closing_loc()?;
        let closing = if index + 1 == args.len() {
            span(&close)
        } else {
            array.elements().iter().last()?.location().end_offset()..close.end_offset()
        };
        let text = &file.bytes[closing.clone()];
        if !(text.ends_with(b"]")
            && text[..text.len() - 1]
                .iter()
                .all(|b| b.is_ascii_whitespace() || *b == b','))
        {
            return None;
        }
        edits.push((span(&array.opening_loc()?), String::new()));
        edits.push((closing, String::new()));
    }
    let start = selector.start_offset();
    let single =
        file.line_of(call.location().start_offset()) == file.line_of(end.saturating_sub(1));
    if call.opening_loc().is_some() || single {
        edits.push((span(selector), "push".into()));
    } else {
        edits.push((start..args[0].location().start_offset(), "push(".into()));
        let at = args.last()?.as_array_node()?.closing_loc()?.end_offset();
        edits.push((at..at, ")".into()));
    }
    Correction::from_edits(&file.bytes, edits)
}
#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop concat_array_literals_spec.rb
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ConcatArrayLiterals.name())
            .collect()
    }
    #[test]
    fn pinned_arguments_positions_and_exact_messages() {
        for (s, col, current, prefer) in [
            ("arr.concat([item])", 5, "concat([item])", "push(item)"),
            ("arr&.concat([item])", 6, "concat([item])", "push(item)"),
            (
                "arr.concat([foo, bar])",
                5,
                "concat([foo, bar])",
                "push(foo, bar)",
            ),
            (
                "arr.concat([foo, bar], [baz])",
                5,
                "concat([foo, bar], [baz])",
                "push(foo, bar, baz)",
            ),
            (
                "arr.concat [foo], [bar]",
                5,
                "concat [foo], [bar]",
                "push(foo, bar)",
            ),
            (
                "arr.concat([foo,], [bar])",
                5,
                "concat([foo,], [bar])",
                "push(foo, bar)",
            ),
            ("arr.concat(%i[item])", 5, "concat(%i[item])", "push(:item)"),
            (
                "arr.concat(%w[item])",
                5,
                "concat(%w[item])",
                "push(\"item\")",
            ),
            ("arr.concat([], [b])", 5, "concat([], [b])", "push(b)"),
            (
                "arr.concat([a], [], [b])",
                5,
                "concat([a], [], [b])",
                "push(a, b)",
            ),
            (
                "arr.concat([], [b]) # comment",
                5,
                "concat([], [b])",
                "push(b)",
            ),
        ] {
            let o = offenses(&format!("# header\n{s}\n"));
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!((o[0].line, o[0].col, o[0].end_line), (2, col, 2), "{s}");
            assert_eq!(
                o[0].message,
                format!("Use `{prefer}` instead of `{current}`.")
            );
            assert!(o[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_dynamic_percent_array_messages() {
        for arg in ["%I[item #{foo}]", "%W[item #{foo}]", "%W[#{foo}]"] {
            let s = format!("arr.concat({arg})");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                format!(
                    "Use `push` with elements as arguments without array brackets instead of `concat({arg})`."
                )
            );
        }
    }
    #[test]
    fn pinned_multiline_comments_heredocs_and_empty_arrays() {
        use crate::cops::testing::correction_offenses;
        assert!(!ConcatArrayLiterals.safe_fixes());
        for (s, prefer, end) in [
            ("arr.concat([\n  foo,\n  bar\n])", "foo, bar", 4),
            ("arr.concat([\n foo,\n], [\n bar,\n])", "foo, bar", 5),
            ("arr.concat [\n foo,\n bar,\n]", "foo, bar", 4),
            ("arr.concat [foo,\n bar\n] if baz", "foo, bar", 3),
            ("arr&.concat [foo], [\n bar\n]", "foo, bar", 3),
            (
                "arr.concat([\n foo # comment\n], [\n bar\n])",
                "foo, bar",
                5,
            ),
            (
                "arr.concat([\n <<~TEXT\n  foo\n TEXT\n], [\n bar\n])",
                "<<~TEXT, bar",
                7,
            ),
            ("arr.concat([ # keep me\n], [\n 'b' # and me\n])", "'b'", 4),
            ("arr.concat(%w[a], [\n b # comment\n])", "\"a\", b", 3),
            ("arr.concat([], [], %w[])", "", 1),
            (
                "arr.concat([[a]], [*b], [effect()])",
                "[a], *b, effect()",
                1,
            ),
            ("concat([a]) {\n effect\n}", "a", 1),
        ] {
            let o = correction_offenses(&ConcatArrayLiterals, s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                (o[0].line, o[0].col, o[0].end_line),
                (
                    1,
                    if s.starts_with("concat") {
                        1
                    } else if s.starts_with("arr&.") {
                        6
                    } else {
                        5
                    },
                    end
                ),
                "{s}"
            );
            assert!(
                o[0].message
                    .starts_with(&format!("Use `push({prefer})` instead of `")),
                "{}",
                o[0].message
            );
        }
        // RuboCop 1.92 (pinned) `-A --only Style/ConcatArrayLiterals` output.
        for (s, fixed) in [
            (
                "arr.concat([\n  foo,\n  bar\n])",
                "arr.push(\n  foo,\n  bar\n)",
            ),
            (
                "arr.concat([\n foo,\n], [\n bar,\n])",
                "arr.push(\n foo, \n bar,\n)",
            ),
            (
                "arr.concat [\n foo,\n bar,\n]",
                "arr.push(\n foo,\n bar,\n)",
            ),
            (
                "arr.concat [foo,\n bar\n] if baz",
                "arr.push(foo,\n bar\n) if baz",
            ),
            ("arr&.concat [foo], [\n bar\n]", "arr&.push(foo, \n bar\n)"),
            (
                "arr.concat([\n foo # comment\n], [\n bar\n])",
                "arr.concat([\n foo # comment\n], [\n bar\n])",
            ),
            (
                "arr.concat([ # keep me\n], [\n 'b' # and me\n])",
                "arr.concat([ # keep me\n], [\n 'b' # and me\n])",
            ),
            (
                "arr.concat(%w[a], [\n b # comment\n])",
                "arr.concat(%w[a], [\n b # comment\n])",
            ),
            ("arr.concat([], [], %w[])", "arr.push()"),
            (
                "arr.concat([[a]], [*b], [effect()])",
                "arr.push([a], *b, effect())",
            ),
            ("concat([a]) {\n effect\n}", "push(a) {\n effect\n}"),
        ] {
            assert_eq!(
                crate::cops::testing::fix_until_stable(&ConcatArrayLiterals, s),
                fixed,
                "{s}"
            );
        }
    }
    #[test]
    fn decoded_percent_elements_are_diagnostics_not_lossy_edits() {
        for (arg, prefer) in [
            ("%w[a\\ b c\\]d]", "\"a b\", \"c]d\""),
            ("%i[a\\ b]", ":\"a b\""),
            ("%w[é]", "\"é\""),
            ("%W[a\\n]", "\"a\\n\""),
            ("%i[]", ""),
        ] {
            let s = format!("arr.concat({arg})");
            let o = offenses(&s);
            assert_eq!(o.len(), 1, "{s}");
            assert_eq!(
                o[0].message,
                format!("Use `push({prefer})` instead of `concat({arg})`.")
            );
        }
    }
    #[test]
    fn pinned_and_structural_no_offense_lookalikes() {
        for s in [
            "arr.concat(items)",
            "arr.concat([foo, bar], baz)",
            "arr.concat",
            "arr.push(item)",
            "arr << item",
            "arr.concat(*[[a]])",
            "arr.concat([a], &block)",
            "arr.concat(&block)",
            "arr.concat(([a]))",
            "def concat; end",
            "# arr.concat([a])\n'concat'",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
}
