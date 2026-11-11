//! `Lint/UselessTimes` — literal Integer#times calls yielding at most once.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct UselessTimes;
impl Cop for UselessTimes {
    fn name(&self) -> &'static str {
        "Lint/UselessTimes"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // The pinned adapter projects compound writes into a zero-argument
        // getter send plus an assignment. Only the getter's range is reported.
        macro_rules! getter {
            ($access:ident) => {{
                let call = node.$access().unwrap();
                if call.is_safe_navigation() || call.read_name().as_slice() != b"times" {
                    return;
                }
                (
                    call.receiver(),
                    file.last_line_of(call.message_loc().unwrap()),
                )
            }};
        }
        let (receiver, end_line) = match node {
            Node::CallOperatorWriteNode { .. } => getter!(as_call_operator_write_node),
            Node::CallOrWriteNode { .. } => getter!(as_call_or_write_node),
            Node::CallAndWriteNode { .. } => getter!(as_call_and_write_node),
            _ => {
                let Some(call) = node.as_call_node() else {
                    return;
                };
                if call.is_safe_navigation()
                    || call.name().as_slice() != b"times"
                    || call.arguments().is_some_and(|a| !a.arguments().is_empty())
                {
                    return;
                }
                // A literal block is transparent to the send matcher; a block
                // pass must contain a static sym, not a dsym.
                if let Some(block) = call.block().filter(|b| b.as_block_node().is_none())
                    && !block
                        .as_block_argument_node()
                        .is_some_and(|b| b.expression().is_some_and(|e| static_symbol(&e)))
                {
                    return;
                }
                (call.receiver(), file.last_line_of(node.location()))
            }
        };
        let Some(receiver) = receiver else { return };
        let negative = ast::negated_numeric_literal(&receiver);
        let literal = negative.as_ref().unwrap_or(&receiver);
        let count = if let Some(integer) = literal.as_integer_node() {
            let value = integer.value();
            let (signed, digits) = value.to_u32_digits();
            let nonzero = digits.iter().any(|d| *d != 0);
            let signed = (signed ^ negative.is_some()) && nonzero;
            if !signed && (digits.len() > 1 || digits.first().is_some_and(|d| *d > 1)) {
                return;
            }
            let number = ast::unsigned_integer_decimal(digits);
            if signed { format!("-{number}") } else { number }
        } else if literal.as_source_line_node().is_some()
            && file.line_of(literal.location().start_offset()) == 1
        {
            "1".into()
        } else {
            return;
        };
        let (line, col) = super::array_literal_in_regexp::reference_line_col(
            node.location().start_offset(),
            file,
        );
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            // Ordinary/numbered/it blocks include the full block; compound
            // assignment getters stop at the selector, excluding the RHS.
            end_line,
            message: format!("Useless call to `{count}.times` detected."),
            correction: None,
        });
    }
}
// Prism represents empty quoted symbols as SymbolNode too. The pinned
// parser translator composes zero parts into dsym, one into sym, and multiple
// physical string parts into dsym. A nonempty Unicode escape such as \\u{}
// still emits one (empty-valued) part; don't reject by decoded value alone.
fn static_symbol(node: &Node<'_>) -> bool {
    let Some(symbol) = node.as_symbol_node() else {
        return false;
    };
    if symbol.closing_loc().is_none() {
        return true;
    }
    let Some(raw) = symbol.value_loc() else {
        return false;
    };
    let raw = raw.as_slice();
    if raw.is_empty() {
        return false;
    }
    if !raw.contains(&b'\n') {
        return true;
    }
    let single = symbol
        .opening_loc()
        .is_some_and(|l| l.as_slice() == b":'" || l.as_slice().starts_with(b"%s"));
    if single {
        return raw.split_inclusive(|b| *b == b'\n').count() == 1;
    }
    // Part-count-only projection of Prism's MIT-licensed parser compiler,
    // string_nodes_from_line_continuations. No escape values are reparsed:
    // use the main parse's decoded line count and the translator's emission
    // slots. Even a nonempty value can emit zero parts (e.g. "\\n\\<LF>").
    let decoded_lines = symbol.unescaped().split_inclusive(|b| *b == b'\n').count();
    let (mut slots, mut escaped_newlines, mut emitted, mut consumed) = (0, 0, 0, 0);
    for line in raw.split_inclusive(|b| *b == b'\n') {
        let (mut slashes, mut newlines) = (0, 0);
        for byte in line {
            if *byte == b'\\' {
                slashes += 1;
            } else {
                newlines += usize::from(*byte == b'n' && slashes % 2 == 1);
                slashes = 0;
            }
        }
        if !line.ends_with(b"\n") && newlines > 0 {
            newlines -= 1;
        }
        escaped_newlines += newlines;
        consumed += line.len();
        let continued = line.strip_suffix(b"\n").is_some_and(|body| {
            let body = body.strip_suffix(b"\r").unwrap_or(body);
            body.iter().rev().take_while(|b| **b == b'\\').count() % 2 == 1
        });
        if !continued || consumed == raw.len() {
            slots += escaped_newlines + 1;
            emitted += usize::from(slots <= decoded_lines);
            escaped_newlines = 0;
        }
    }
    emitted == 1
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/useless_times_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(source: &str) -> Vec<Offense> {
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/UselessTimes")
            .collect()
    }
    #[test]
    fn compound_assignments_report_only_the_getter_range() {
        for operator in ["+=", "||=", "&&="] {
            for (receiver, count) in [("0", "0"), ("1", "1"), ("-1", "-1")] {
                let source = format!("# getter\n  {receiver}.times {operator}\n    work\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    format!("Useless call to `{count}.times` detected.")
                );
                assert!(found[0].correction.is_none());
            }
            // A newline before the selector belongs to the getter; the RHS does not.
            let source = format!("0\n .times {operator}\n 1.times {{\n  work\n }}\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 2, "{source}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (1, 1, 2));
            assert_eq!((found[1].line, found[1].col, found[1].end_line), (3, 2, 5));
        }
    }
    #[test]
    fn compound_assignment_lookalikes_remain_ignored() {
        for operator in ["+=", "||=", "&&=", "="] {
            for receiver in ["2", "(0)", "0.0", "x"] {
                let source = format!("{receiver}.times {operator} 1\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
            for receiver in ["0", "1", "-1"] {
                let source = format!("{receiver}&.times {operator} 1\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        assert!(offenses("0.times = 1\n").is_empty());
        assert!(offenses("0.other += 1\n").is_empty());
        assert!(offenses("0[1] += 1\n").is_empty());
    }
    #[test]
    fn pinned_zero_negative_one_and_block_pass() {
        for count in ["0", "-1", "1"] {
            for suffix in [
                " { something }",
                " { |i| something(i) }",
                "(&:something)",
                "",
                " { next 42 }",
                " { break }",
                " { redo }",
                " { |i,j| work(i,j) }",
                " { |(a,b)| work(a,b) }",
                " { |*a| work(a) }",
                " { [1,2].each { next } }",
            ] {
                let source = format!("# example\n  {count}.times{suffix}\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
                assert_eq!(
                    found[0].message,
                    format!("Useless call to `{count}.times` detected.")
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn arbitrary_precision_and_adapter_literal_values() {
        for (literal, count) in [
            (
                "-0xffffffffffffffffffffffffffffffff",
                "-340282366920938463463374607431768211455",
            ),
            (
                "-999999999999999999999999999999999999",
                "-999999999999999999999999999999999999",
            ),
            ("--1", "-1"),
            ("--2", "-2"),
            ("---1", "-1"),
            ("-0", "0"),
            ("0b1", "1"),
            ("+1", "1"),
            ("__LINE__", "1"),
            ("-__LINE__", "1"),
        ] {
            let source = format!("{literal}.times {{ it }}");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                found[0].message,
                format!("Useless call to `{count}.times` detected.")
            );
        }
        for source in [
            "999999999999999999999999999999999999.times {}",
            "0.-@.times {}",
            "-(0).times {}",
            "\n__LINE__.times {}",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
    #[test]
    fn block_pass_requires_reference_static_sym_not_empty_or_multipart_dsym() {
        for symbol in [
            r#":"""#,
            ":''",
            "%s()",
            "%s[]",
            "%s''",
            ":\"\\\n\"",
            ":\"\\\r\n\"",
            ":\"\\\n\\\n\"",
            ":\"\\u{}\\\n\"",
            ":\"\\n\\\n\"",
            ":\"\\n\\\r\n\"",
            ":\"a\nb\"",
            ":'a\nb'",
            "%s(a\nb)",
            r##":"#{work}""##,
            r#":"work#{nil}""#,
            "(:work)",
            "(:'')",
        ] {
            for count in [
                "0",
                "1",
                "-1",
                "-999999999999999999999999999999999999",
                "--1",
            ] {
                let source = format!("consume({count}.times(&{symbol}))\n");
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
    }
    #[test]
    fn static_symbol_projection_preserves_unicode_empty_and_nonempty_controls() {
        for symbol in [
            ":work",
            r#":"\u{}""#,
            r#":"\u{ }""#,
            r#":" ""#,
            ":' '",
            "%s( )",
            r#":"\u{77 6f 72 6b}""#,
            r#":"work\n""#,
            r#":'\n'"#,
            r#"%s(\n)"#,
            ":'\\\n'",
            "%s(\\\n)",
            ":\"\\\nwork\"",
            ":\"work\n\"",
            ":\"\n\"",
            ":\"\\u{}\n\"",
        ] {
            let source = format!("# static sym\n  0.times(&{symbol})\n");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (2, 3), "{source}");
            assert_eq!(found[0].end_line, source.lines().count() as u32, "{source}");
            assert_eq!(found[0].message, "Useless call to `0.times` detected.");
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn pinned_multiline_and_inline_calls() {
        for (source, line, col, end) in [
            ("1.times.reverse_each do\n foo\nend", 1, 1, 1),
            ("def foo\n 1.times do\n end\nend", 2, 2, 3),
            (
                "def foo\n 1.times do |i|\n work(i)\n i += 1\n\n work(i)\n end\nend",
                2,
                2,
                7,
            ),
            ("1.times do |i|\n i,j = i*2, i*3\nend", 1, 1, 3),
            ("1.times do |i|\n j = 1\n work(j)\nend", 1, 1, 4),
            ("foo(0.times { do_something })", 1, 5, 1),
        ] {
            let found = offenses(source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(
                (found[0].line, found[0].col, found[0].end_line),
                (line, col, end)
            );
        }
    }
    #[test]
    fn ignores_noninteger_grouped_safe_navigation_and_argument_lookalikes() {
        for source in [
            "2.times { |i| puts i }",
            "2.times(&:something)",
            "0&.times {}",
            "(0).times {}",
            "0.0.times {}",
            "0r.times {}",
            "0i.times {}",
            "x.times {}",
            "times {}",
            "0.times(arg)",
            "0.times(&block)",
            "0.times(&nil)",
            "0.times(&:foo, 1)",
            "0.times(&:\"#{foo}\")",
            "1.times(1, &:foo)",
            "0.times!",
            "'0.times {}'",
            "# 0.times {}",
        ] {
            assert!(offenses(source).is_empty(), "{source}");
        }
    }
}
