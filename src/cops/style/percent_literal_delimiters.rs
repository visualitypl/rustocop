//! `Style/PercentLiteralDelimiters` — fixed `()`, word/symbol arrays `[]`, regexp `{}`.

use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{Location, Node};

pub struct PercentLiteralDelimiters;

impl Cop for PercentLiteralDelimiters {
    fn name(&self) -> &'static str {
        "Style/PercentLiteralDelimiters"
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
        let Some((opening, closing)) = delimiters(node) else {
            return;
        };
        let source = opening.as_slice();
        if !source.starts_with(b"%") {
            return;
        }
        let prefix_len = if matches!(
            source.get(1),
            Some(b'q' | b'Q' | b'w' | b'W' | b'i' | b'I' | b'r' | b's' | b'x')
        ) {
            2
        } else {
            1
        };
        let kind = &source[..prefix_len];
        // Upstream has on_sym, not on_dsym; empty %s literals and physical
        // multiline literals with another fragment become dsym upstream.
        if kind == b"%s" {
            let body = &file.bytes[opening.end_offset()..closing.start_offset()];
            if body.is_empty() || body[..body.len().saturating_sub(1)].contains(&b'\n') {
                return;
            }
        }
        let (left, right) = match kind {
            b"%w" | b"%W" | b"%i" | b"%I" => (b'[', b']'),
            b"%r" => (b'{', b'}'),
            _ => (b'(', b')'),
        };
        let used = source[prefix_len];
        if used == left {
            return;
        }
        if contains_literal(node, false, left, right) {
            return;
        }
        let used_close = match used {
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            b'<' => b'>',
            _ => used,
        };
        if matches!(kind, b"%w" | b"%i") && contains_literal(node, false, used, used_close) {
            return;
        }
        if matches!(kind, b"%q" | b"%s" | b"%r") {
            let body = &file.bytes[opening.end_offset()..closing.start_offset()];
            if escaped_delimiter(body, used, used_close) {
                return;
            }
        }
        let (line, col) = file.line_col(opening.start_offset());
        let message = format!(
            "`{}`-literals should be delimited by `{}` and `{}`.",
            String::from_utf8_lossy(kind),
            char::from(left),
            char::from(right)
        );
        let correction = cx
            .corrections_requested()
            .then(|| {
                // A single atomic envelope prevents a partial delimiter change when
                // another correction overlaps, or a body line disables the cop.
                // Non-UTF-8 interiors cannot be stored losslessly in Correction.
                // Ruby permits physical-newline delimiters. Replacing those
                // would remove code lines and can expose deferred heredoc text.
                if source.len() != prefix_len + 1 || matches!(used, b'\n' | b'\r') {
                    return None;
                }
                let delimiter_start = opening.end_offset() - 1;
                // A Q->q edit is proven against the OLD delimiters. If those
                // delimiters consume escapes, changing both case and delimiters
                // can retain a backslash (e.g. %Q=a\=b= -> %q(a\=b)). Include
                // the unchanged Q in the envelope so these edits must defer.
                let (start, replacement) = if kind == b"%Q"
                    && escaped_delimiter(
                        &file.bytes[opening.end_offset()..closing.start_offset()],
                        used,
                        used_close,
                    ) {
                    (delimiter_start - 1, format!("Q{}", char::from(left)))
                } else {
                    (delimiter_start, char::from(left).to_string())
                };
                super::correction::combine(
                    file,
                    vec![
                        Correction {
                            range: start..opening.end_offset(),
                            replacement,
                        },
                        Correction {
                            range: closing.start_offset()..closing.start_offset() + 1,
                            replacement: char::from(right).to_string(),
                        },
                    ],
                )
            })
            .flatten();
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.last_line_of(node.location()),
            correction,
        });
    }
}

fn escaped_delimiter(body: &[u8], left: u8, right: u8) -> bool {
    let mut i = 0;
    while i + 1 < body.len() {
        if body[i] == b'\\' {
            if body[i + 1] == left || body[i + 1] == right {
                return true;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    false
}

fn delimiters<'pr>(node: &Node<'pr>) -> Option<(Location<'pr>, Location<'pr>)> {
    match node {
        Node::StringNode { .. } => {
            let n = node.as_string_node()?;
            Some((n.opening_loc()?, n.closing_loc()?))
        }
        Node::InterpolatedStringNode { .. } => {
            let n = node.as_interpolated_string_node()?;
            Some((n.opening_loc()?, n.closing_loc()?))
        }
        Node::SymbolNode { .. } => {
            let n = node.as_symbol_node()?;
            Some((n.opening_loc()?, n.closing_loc()?))
        }
        Node::InterpolatedSymbolNode { .. } => {
            let n = node.as_interpolated_symbol_node()?;
            Some((n.opening_loc()?, n.closing_loc()?))
        }
        Node::RegularExpressionNode { .. } => {
            let n = node.as_regular_expression_node()?;
            Some((n.opening_loc(), n.closing_loc()))
        }
        Node::InterpolatedRegularExpressionNode { .. } => {
            let n = node.as_interpolated_regular_expression_node()?;
            Some((n.opening_loc(), n.closing_loc()))
        }
        Node::MatchLastLineNode { .. } => {
            let n = node.as_match_last_line_node()?;
            Some((n.opening_loc(), n.closing_loc()))
        }
        Node::InterpolatedMatchLastLineNode { .. } => {
            let n = node.as_interpolated_match_last_line_node()?;
            Some((n.opening_loc(), n.closing_loc()))
        }
        Node::XStringNode { .. } => {
            let n = node.as_x_string_node()?;
            Some((n.opening_loc(), n.closing_loc()))
        }
        Node::InterpolatedXStringNode { .. } => {
            let n = node.as_interpolated_x_string_node()?;
            Some((n.opening_loc(), n.closing_loc()))
        }
        Node::ArrayNode { .. } => {
            let n = node.as_array_node()?;
            Some((n.opening_loc()?, n.closing_loc()?))
        }
        _ => None,
    }
}

// RuboCop compares decoded scalar strings/symbols, but raw static fragments
// inside interpolating literals/arrays. Never descend into interpolation code:
// its delimiters cannot close the surrounding literal. Work is bounded by this
// literal's own static fragments, not its recursive subtree or the full source.
fn contains_literal(node: &Node<'_>, raw: bool, left: u8, right: u8) -> bool {
    let contains = |bytes: &[u8]| bytes.contains(&left) || bytes.contains(&right);
    match node {
        Node::StringNode { .. } => {
            let n = node.as_string_node().unwrap();
            contains(if raw {
                n.content_loc().as_slice()
            } else {
                n.unescaped()
            })
        }
        Node::SymbolNode { .. } => {
            let n = node.as_symbol_node().unwrap();
            contains(if raw {
                n.location().as_slice()
            } else {
                n.unescaped()
            })
        }
        Node::RegularExpressionNode { .. } => contains(
            node.as_regular_expression_node()
                .unwrap()
                .content_loc()
                .as_slice(),
        ),
        Node::MatchLastLineNode { .. } => contains(
            node.as_match_last_line_node()
                .unwrap()
                .content_loc()
                .as_slice(),
        ),
        Node::XStringNode { .. } => {
            contains(node.as_x_string_node().unwrap().content_loc().as_slice())
        }
        _ => {
            let parts = match node {
                Node::InterpolatedStringNode { .. } => {
                    node.as_interpolated_string_node().unwrap().parts()
                }
                Node::InterpolatedSymbolNode { .. } => {
                    node.as_interpolated_symbol_node().unwrap().parts()
                }
                Node::InterpolatedRegularExpressionNode { .. } => node
                    .as_interpolated_regular_expression_node()
                    .unwrap()
                    .parts(),
                Node::InterpolatedMatchLastLineNode { .. } => {
                    node.as_interpolated_match_last_line_node().unwrap().parts()
                }
                Node::InterpolatedXStringNode { .. } => {
                    node.as_interpolated_x_string_node().unwrap().parts()
                }
                Node::ArrayNode { .. } => node.as_array_node().unwrap().elements(),
                _ => return false,
            };
            parts
                .iter()
                .any(|part| contains_literal(&part, true, left, right))
        }
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported/adapted to fixed delimiters from RuboCop's
    //! `spec/rubocop/cop/style/percent_literal_delimiters_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    use crate::{cops::testing::assert_correction, pipeline::lint_str};

    #[test]
    fn newline_delimiters_remain_reported_without_changing_physical_lines() {
        for source in ["%Q\nhello\n", "%q\r\nhello\r\n", "%\nhello\n"] {
            let found =
                crate::cops::testing::correction_offenses(&PercentLiteralDelimiters, source);
            assert_eq!(found.len(), 1, "{source:?}");
            assert!(found[0].correction.is_none(), "{source:?}");
        }
        for (before, after) in [("%Q hello ", "%Q(hello)"), ("%Q\thello\t", "%Q(hello)")] {
            assert_correction(&PercentLiteralDelimiters, before, after);
        }
    }

    #[test]
    fn empty_symbols_are_upstream_dsym_not_sym_dispatch() {
        for source in ["%s[]", "%s!!", "p [%Q(), %s[]]"] {
            assert!(
                lint_str(source)
                    .into_iter()
                    .all(|o| o.cop != "Style/PercentLiteralDelimiters"),
                "{source}"
            );
        }
    }

    #[test]
    fn multiline_symbols_are_upstream_dsym_not_sym_dispatch() {
        for source in ["%s[a\nb]", "%s[a\r\nb]", "%s[\n\n]"] {
            assert!(
                lint_str(source)
                    .into_iter()
                    .all(|o| o.cop != "Style/PercentLiteralDelimiters"),
                "{source}"
            );
        }
        assert_correction(&PercentLiteralDelimiters, "%s[a\n]", "%s(a\n)");
    }

    #[test]
    fn all_types_report_exact_diagnostics_and_correct_both_delimiters() {
        for (kind, old, new, left, right) in [
            ("%", "%[string]", "%(string)", '(', ')'),
            ("%q", "%q[string]", "%q(string)", '(', ')'),
            ("%Q", "%Q[string]", "%Q(string)", '(', ')'),
            ("%s", "%s[symbol]", "%s(symbol)", '(', ')'),
            ("%x", "%x[command]", "%x(command)", '(', ')'),
            ("%w", "%w(some words)", "%w[some words]", '[', ']'),
            ("%W", "%W(some words)", "%W[some words]", '[', ']'),
            ("%i", "%i(some symbols)", "%i[some symbols]", '[', ']'),
            ("%I", "%I(some words)", "%I[some words]", '[', ']'),
            ("%r", "%r[.*]im", "%r{.*}im", '{', '}'),
            ("%w", "%w()", "%w[]", '[', ']'),
        ] {
            let found: Vec<_> = lint_str(&format!("# heading\n  {old}\n"))
                .into_iter()
                .filter(|o| o.cop == "Style/PercentLiteralDelimiters")
                .collect();
            assert_eq!(found.len(), 1, "{old}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(
                found[0].message,
                format!("`{kind}`-literals should be delimited by `{left}` and `{right}`.")
            );
            assert_correction(&PercentLiteralDelimiters, old, new);
        }
    }

    #[test]
    fn preferred_delimiters_inside_interpolation_do_not_block_correction() {
        for (before, after) in [
            ("%[#{(1)}]", "%(#{(1)})"),
            ("%Q[#{(1)}]", "%Q(#{(1)})"),
            ("%W(#{[1].first})", "%W[#{[1].first}]"),
            ("%I(#{[1].first})", "%I[#{[1].first}]"),
            ("%r[#{ {a: 1} }]i", "%r{#{ {a: 1} }}i"),
            ("%x[#{(1)}]", "%x(#{(1)})"),
            (r"%{\x80}", r"%(\x80)"),
        ] {
            assert_correction(&PercentLiteralDelimiters, before, after);
        }
    }

    #[test]
    fn pinned_no_offense_exceptions_and_non_percent_lookalikes() {
        for source in [
            "%(string)",
            "%q(string)",
            "%Q(string)",
            "%s(symbol)",
            "%x(command)",
            "%w[some words]",
            "%W[some words]",
            "%i[some symbols]",
            "%I[some symbols]",
            "%r{regexp}",
            "%[(string)]",
            "%q[(string)]",
            "%W([some] [words])",
            "%w([some] [words])",
            "%r[{regexp}]",
            "%s[(]",
            r"%w(\(some words\))",
            r"%w(only closing delimiter character\))",
            r"%w|\|not pairing delimiter|",
            r"%i(\(\) each)",
            r"%q!a\!b!",
            r"%s!a\!b!",
            r"%r!a\!b!",
            r"%q[a\]b]",
            r"%s[a\]b]",
            r"%r[a\]b]",
            "'hi'",
            "\"hi\"",
            "/regexp/",
            "[1,2]",
            "`command`",
            "<<~X\n%w(foo)\nX\n",
        ] {
            assert!(
                lint_str(source)
                    .into_iter()
                    .all(|o| o.cop != "Style/PercentLiteralDelimiters"),
                "{source}"
            );
        }
    }

    #[test]
    fn all_23_applicable_pinned_correction_calls_have_exact_bytes() {
        let mut pairs: Vec<(String, String)> = [
            ("%w(some words)\n", "%w[some words]\n"),
            ("%W(some words)\n", "%W[some words]\n"),
            ("%W(#{[1].first})\n", "%W[#{[1].first}]\n"),
            ("%i(some symbols)\n", "%i[some symbols]\n"),
            ("%I(some words)\n", "%I[some words]\n"),
            ("%I(#{[1].first})\n", "%I[#{[1].first}]\n"),
            ("%w(\nsome\nwords\n)\n", "%w[\nsome\nwords\n]\n"),
            (
                "  array = %w(\n    first\n    second\n  )\n",
                "  array = %w[\n    first\n    second\n  ]\n",
            ),
            (
                "  array = %w(\n    first\n  second\n)\n",
                "  array = %w[\n    first\n  second\n]\n",
            ),
        ]
        .into_iter()
        .map(|(a, b)| (a.into(), b.into()))
        .collect();
        // Shared escape-character examples: spec lines 447 and 459.
        for kind in ["%w", "%W", "%i"] {
            for body in ["\n", "\t"] {
                pairs.push((format!("{kind}{{{body}}}\n"), format!("{kind}[{body}]\n")));
            }
        }
        // Shared changeable/interpolating examples: lines 492,503,516,527.
        for kind in ["%W", "%I"] {
            for (before, after) in [
                ("(a\\\\)", "[a\\\\]"),
                ("(a(b)c)", "[a(b)c]"),
                ("!a\\!b!", "[a\\!b]"),
                ("(a\\)b)", "[a\\)b]"),
            ] {
                pairs.push((format!("{kind}{before}\n"), format!("{kind}{after}\n")));
            }
        }
        assert_eq!(pairs.len(), 23);
        for (before, after) in pairs {
            assert_correction(&PercentLiteralDelimiters, &before, &after);
        }
    }

    #[test]
    fn pinned_escapes_nested_delimiters_and_multiline_corrections() {
        for kind in ["%", "%q", "%Q", "%s", "%w", "%W", "%x", "%r", "%i", "%I"] {
            let (open, close) = match kind {
                "%w" | "%W" | "%i" | "%I" => ('[', ']'),
                "%r" => ('{', '}'),
                _ => ('(', ')'),
            };
            for body in ["", "\n", "\t", "a\\\\"] {
                if kind == "%s" && body.is_empty() {
                    continue; // Empty symbols are upstream dsym, outside on_sym.
                }
                assert_correction(
                    &PercentLiteralDelimiters,
                    &format!("{kind}!{body}!"),
                    &format!("{kind}{open}{body}{close}"),
                );
            }
        }
        for kind in ["%", "%Q", "%W", "%x", "%I"] {
            assert_correction(
                &PercentLiteralDelimiters,
                &format!(r"{kind}!a\!b!"),
                &format!(
                    r"{kind}{}a\!b{}",
                    if matches!(kind, "%W" | "%I") {
                        '['
                    } else {
                        '('
                    },
                    if matches!(kind, "%W" | "%I") {
                        ']'
                    } else {
                        ')'
                    }
                ),
            );
        }
        for (before, after) in [
            ("%w(\nsome\nwords\n)", "%w[\nsome\nwords\n]"),
            (
                "  array = %w(\n    first\n    second\n  )",
                "  array = %w[\n    first\n    second\n  ]",
            ),
            (
                "  array = %w(\n    first\n  second\n)",
                "  array = %w[\n    first\n  second\n]",
            ),
            ("%Q[a[b]c]", "%Q(a[b]c)"),
            ("%r[a(b)c]", "%r{a(b)c}"),
            ("%I(a(b)c)", "%I[a(b)c]"),
        ] {
            assert_correction(&PercentLiteralDelimiters, before, after);
        }
    }
}
