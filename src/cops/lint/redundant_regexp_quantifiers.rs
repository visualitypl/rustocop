//! `Lint/RedundantRegexpQuantifiers` — mergeable greedy quantifiers in passive groups.
use super::duplicate_regexp_character_class_element::{
    bracket_control_escape_end, capture_condition_end, capture_reference_token, char_width,
    escape_end, interval_end, quantifier_view,
};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::Node;
use std::ops::Range;

pub struct RedundantRegexpQuantifiers;
impl Cop for RedundantRegexpQuantifiers {
    fn name(&self) -> &'static str {
        "Lint/RedundantRegexpQuantifiers"
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
        // Pinned RegexpNode#interpolation? tests only direct begin children,
        // even constant/empty #{...}. Shorthand variables are not begin nodes:
        // the existing View preserves their raw-source and position quirks.
        let parts = node
            .as_interpolated_regular_expression_node()
            .map(|n| n.parts())
            .or_else(|| {
                node.as_interpolated_match_last_line_node()
                    .map(|n| n.parts())
            });
        if parts.as_ref().is_some_and(|parts| {
            parts
                .iter()
                .any(|p| p.as_embedded_statements_node().is_some())
        }) {
            return;
        }
        let Some(mut regexp) = crate::cops::style::regexp_literal::Regexp::new(node) else {
            return;
        };
        if let Some(parts) = parts {
            // Keep the main-parse multiline emission facts aligned with this
            // consumer's additional shorthand source fragments.
            regexp.fragment_line_counts = parts
                .iter()
                .filter_map(|part| {
                    part.as_string_node()
                        .map(|n| n.unescaped().split_inclusive(|b| *b == b'\n').count())
                        .or_else(|| part.as_embedded_variable_node().map(|_| 1))
                })
                .collect();
            // Feed every direct non-begin child's source to the existing View,
            // including adjacent/trailing shorthand variables. Their locations
            // exclude '#', exactly like the pinned AST's variable child source.
            // This projection is local to this cop; older consumers are unchanged.
            regexp.fragments = parts
                .iter()
                .filter_map(|part| {
                    part.as_string_node().map(|n| n.content_loc()).or_else(|| {
                        part.as_embedded_variable_node()
                            .map(|n| n.variable().location())
                    })
                })
                .collect();
        }
        let Some(view) = quantifier_view(&regexp, file) else {
            return;
        };
        let tree = expressions(&view.bytes, regexp.closing.as_slice().contains(&b'x'));
        let correctable = cx.corrections_requested();
        let mut seen = vec![false; tree.len()];
        for (i, group) in tree.iter().enumerate() {
            if seen[i] || !group.redundant() || group.alternation {
                continue;
            }
            let Some(outer) = &group.quantifier else {
                continue;
            };
            let Some(outer_kind) = outer.kind else {
                continue;
            };
            for (j, child) in tree.iter().enumerate().take(group.end).skip(i + 1) {
                seen[j] = true;
                if !child.redundant()
                    && !matches!(child.kind, Kind::Terminal | Kind::Literal | Kind::Free)
                {
                    break;
                }
                let Some(inner) = &child.quantifier else {
                    if child.alternation {
                        break;
                    }
                    continue;
                };
                let Some(inner_kind) = inner.kind else {
                    if child.alternation {
                        break;
                    }
                    continue;
                };
                let replacement = if outer_kind == inner_kind {
                    inner_kind
                } else {
                    '*'
                };
                let range = view.starts[inner.range.start]..view.ends[outer.range.end - 1];
                let (line, col) = view.positions[inner.range.start];
                // RuboCop drops the outer quantifier and replaces the inner one.
                let correction = correctable
                    .then(|| {
                        // One atomic envelope preserves all intervening group tokens
                        // and comments. Nested/overlapping pairs defer to the shared
                        // planner rather than applying half a correction.
                        let mut bytes = vec![replacement as u8];
                        bytes.extend_from_slice(
                            &file.bytes
                                [view.ends[inner.range.end - 1]..view.starts[outer.range.start]],
                        );
                        Some(Correction {
                            range: range.clone(),
                            replacement: String::from_utf8(bytes).ok()?,
                        })
                    })
                    .flatten();
                out.push(Offense {
                    cop: self.name(), severity: self.severity(), line, col,
                    end_line: view.positions[outer.range.end - 1].0,
                    message: format!("Replace redundant quantifiers `{}` and `{}` with a single `{replacement}`.",
                        std::str::from_utf8(&view.bytes[inner.range.clone()]).unwrap(),
                        std::str::from_utf8(&view.bytes[outer.range.clone()]).unwrap()),
                    correction,
                });
                if child.alternation {
                    break;
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Passive,
    Barrier,
    Terminal,
    Literal,
    Free,
    Comment,
    OptionsSwitch,
}
struct Expression {
    kind: Kind,
    // Arena is preorder; end is the exclusive subtree boundary. No recursive
    // traversals or per-group rescans, including deeply nested passive chains.
    end: usize,
    children: usize,
    direct: Vec<usize>,
    alternation: bool,
    last: Option<usize>,
    literal_start: usize,
    literal_end: usize,
    quantifier: Option<Quantifier>,
}
impl Expression {
    fn redundant(&self) -> bool {
        self.kind == Kind::Passive && (self.children == 1 || self.alternation)
    }
}
struct Quantifier {
    range: Range<usize>,
    kind: Option<char>,
}
fn push(tree: &mut Vec<Expression>, parent: usize, kind: Kind, start: usize, end: usize) -> usize {
    let id = tree.len();
    if kind != Kind::Free {
        tree[parent].children += 1;
        if kind != Kind::Comment {
            tree[parent].last = Some(id);
        }
    }
    tree[parent].direct.push(id);
    tree.push(Expression {
        kind,
        end: id + 1,
        children: 0,
        direct: vec![],
        alternation: false,
        last: None,
        literal_start: start,
        literal_end: end,
        quantifier: None,
    });
    id
}
fn normalized(bytes: &[u8]) -> Option<char> {
    match bytes {
        b"*" => Some('*'),
        b"+" => Some('+'),
        b"?" => Some('?'),
        _ => {
            let body = bytes.strip_prefix(b"{")?.strip_suffix(b"}")?;
            let comma = body.iter().position(|b| *b == b',')?;
            let number = |s: &[u8]| {
                if s.is_empty() {
                    Some(0)
                } else {
                    std::str::from_utf8(s).ok()?.parse::<u64>().ok()
                }
            };
            let min = number(&body[..comma])?;
            let max = &body[comma + 1..];
            match (min, max.is_empty(), number(max)?) {
                (0, true, _) => Some('*'),
                (1, true, _) => Some('+'),
                (0, false, 1) => Some('?'),
                _ => None,
            }
        }
    }
}

// A second linear token pass builds only the expression facts this rule needs.
// Successful-tree validation, source projection and escape boundaries are shared
// with the landed regexp consumers; this does not construct or execute regexps.
fn expressions(bytes: &[u8], mut extended: bool) -> Vec<Expression> {
    let mut tree = vec![Expression {
        kind: Kind::Barrier,
        end: 0,
        children: 0,
        direct: vec![],
        alternation: false,
        last: None,
        literal_start: 0,
        literal_end: 0,
        quantifier: None,
    }];
    let mut stack = vec![(0, None)];
    let mut i = 0;
    let mut captures = 0;
    let mut missing_reference_delimiters = [false; 2];
    while i < bytes.len() {
        let parent = stack.last().unwrap().0;
        let start = i;
        let b = bytes[i];
        if extended && (b.is_ascii_whitespace() || b == b'#') {
            i += if b == b'#' {
                bytes[i..]
                    .iter()
                    .position(|b| *b == b'\n')
                    .unwrap_or(bytes.len() - i)
            } else {
                1
            };
            push(&mut tree, parent, Kind::Free, start, i);
        } else if matches!(b, b'*' | b'+' | b'?') || interval_end(bytes, i).is_some() {
            i = interval_end(bytes, i).unwrap_or(i + 1);
            let mut kind = normalized(&bytes[start..i]);
            if b != b'{' && matches!(bytes.get(i), Some(b'?' | b'+')) {
                i += 1;
                kind = None;
            }
            if let Some(mut last) = tree[parent].last {
                if tree[last].kind == Kind::OptionsSwitch {
                    return vec![];
                }
                // Literal runs are split by the scanner immediately before a
                // quantifier: `ab+` is two children, not one quantified run.
                if tree[last].kind == Kind::Literal {
                    let begin = tree[last].literal_start;
                    let end = tree[last].literal_end;
                    let mut final_char = begin;
                    let mut at = begin;
                    while at < end {
                        final_char = at;
                        at += char_width(bytes, at);
                    }
                    if final_char > begin {
                        tree[last].literal_end = final_char;
                        last = push(&mut tree, parent, Kind::Literal, final_char, end);
                    }
                }
                if tree[last].quantifier.is_some() {
                    // regexp_parser wraps chained quantifiers in implicit
                    // passive groups. Keep arena links O(1), flatten once below.
                    let child = tree.len();
                    let wrapped = std::mem::replace(
                        &mut tree[last],
                        Expression {
                            kind: Kind::Passive,
                            end: 0,
                            children: 1,
                            direct: vec![child],
                            alternation: false,
                            last: Some(child),
                            literal_start: 0,
                            literal_end: 0,
                            quantifier: None,
                        },
                    );
                    tree.push(wrapped);
                }
                tree[last].quantifier = Some(Quantifier {
                    range: start..i,
                    kind,
                });
            }
        } else if b == b'(' {
            let old = extended;
            let mut kind = Kind::Barrier;
            let mut restore = None;
            if bytes[i..].starts_with(b"(?#") {
                i += 3 + bytes[i + 3..].iter().position(|b| *b == b')').unwrap() + 1;
                push(&mut tree, parent, Kind::Comment, start, i);
                continue;
            }
            if bytes.get(i + 1) != Some(&b'?') {
                captures += 1;
            }
            i += 1;
            if bytes.get(i) == Some(&b'?') {
                i += 1;
                let options = i;
                let mut enabled = true;
                while bytes.get(i).is_some_and(|b| b"imxadu-".contains(b)) {
                    match bytes[i] {
                        b'-' => enabled = false,
                        b'x' => extended = enabled,
                        _ => {}
                    }
                    i += 1;
                }
                if i > options {
                    if bytes[i] == b')' {
                        i += 1;
                        push(&mut tree, parent, Kind::OptionsSwitch, start, i);
                        continue;
                    }
                    restore = Some(old);
                    i += 1; // scoped option ':'
                } else {
                    match bytes[i] {
                        b':' => {
                            kind = Kind::Passive;
                            i += 1;
                        }
                        b'<' if !matches!(bytes.get(i + 1), Some(b'=' | b'!')) => {
                            i += bytes[i..].iter().position(|b| *b == b'>').unwrap() + 1;
                        }
                        b'\'' => {
                            i += 1 + bytes[i + 1..].iter().position(|b| *b == b'\'').unwrap() + 1;
                        }
                        b'(' => {
                            i = capture_condition_end(bytes, i + 1).unwrap() + 1;
                        }
                        b'<' => i += 2,
                        _ => i += 1,
                    }
                }
            }
            let id = push(&mut tree, parent, kind, start, i);
            stack.push((id, restore));
        } else if b == b')' {
            let (id, restore) = stack.pop().unwrap();
            tree[id].end = tree.len();
            if let Some(old) = restore {
                extended = old;
            }
            i += 1;
        } else if b == b'|' {
            // An alternation is a nonterminal barrier even with just one
            // branch expression. It also prevents a surrounding passive group
            // from being mistaken for a one-child chain.
            tree[parent].alternation = true;
            push(&mut tree, parent, Kind::Barrier, i, i + 1);
            i += 1;
        } else if b == b'[' {
            let mut depth = 1;
            i += 1;
            while depth > 0 {
                if bytes[i..].starts_with(b"[:") {
                    i += 2 + bytes[i + 2..].windows(2).position(|b| b == b":]").unwrap() + 2;
                } else if bytes[i] == b'\\' {
                    i = escape_end(bytes, i, true).unwrap();
                } else {
                    match bytes[i] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        _ => {}
                    }
                    i += char_width(bytes, i);
                }
            }
            push(&mut tree, parent, Kind::Terminal, start, i);
        } else if b == b'\\' {
            i = if matches!(bytes.get(i + 1), Some(b'1'..=b'9')) {
                let mut end = i + 2;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
                let digits = &bytes[i + 1..end];
                if digits.len() == 1
                    || digits.iter().all(|b| matches!(b, b'0'..=b'7'))
                    || std::str::from_utf8(digits)
                        .unwrap()
                        .parse::<usize>()
                        .is_ok_and(|n| n <= captures)
                {
                    end
                } else {
                    i + 2
                }
            } else if matches!(bytes.get(i + 1), Some(b'k' | b'g'))
                && !capture_reference_token(bytes, i, &mut missing_reference_delimiters)
            {
                i + 2
            } else if matches!(bytes.get(i + 1), Some(b'c' | b'C' | b'M')) {
                bracket_control_escape_end(bytes, i).unwrap()
            } else if bytes.get(i + 1) == Some(&b'0') {
                let mut end = i + 2;
                // Octal escapes contain at most three digits, including the
                // leading zero. Any remaining digits are literal children.
                while end < i + 4 && bytes.get(end).is_some_and(|b| matches!(b, b'0'..=b'7')) {
                    end += 1;
                }
                end
            } else {
                escape_end(bytes, i, false).unwrap()
            };
            // Codepoint lists expand to individual terminal expressions. The
            // quantifier belongs to the last one, not the complete list.
            let count = if bytes[start..i].starts_with(b"\\u{") {
                bytes[start + 3..i - 1]
                    .split(u8::is_ascii_whitespace)
                    .count()
            } else {
                1
            };
            for _ in 0..count {
                push(&mut tree, parent, Kind::Terminal, start, i);
            }
        } else {
            i += char_width(bytes, i);
            let kind = if b".^$".contains(&b) {
                Kind::Terminal
            } else {
                Kind::Literal
            };
            if kind == Kind::Literal
                && tree[parent].last.is_some_and(|last| {
                    tree[last].kind == kind
                        && tree[last].quantifier.is_none()
                        && tree[last].literal_end == start
                })
            {
                let last = tree[parent].last.unwrap();
                tree[last].literal_end = i;
            } else {
                push(&mut tree, parent, kind, start, i);
            }
        }
    }
    let mut arena: Vec<_> = tree.into_iter().map(Some).collect();
    let mut ordered: Vec<Expression> = vec![];
    let mut stack = vec![(0, false)];
    while let Some((id, exit)) = stack.pop() {
        if exit {
            ordered[id].end = ordered.len();
            continue;
        }
        let mut expr = arena[id].take().unwrap();
        stack.push((ordered.len(), true));
        for child in expr.direct.drain(..).rev() {
            stack.push((child, false));
        }
        ordered.push(expr);
    }
    ordered
}

#[cfg(test)]
mod tests {
    //! Ported from RuboCop's lint/redundant_regexp_quantifiers_spec.rb
    //! (MIT, Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(source: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/RedundantRegexpQuantifiers")
            .collect()
    }
    #[test]
    fn complete_reference_sources_and_controls() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/redundant_regexp_quantifiers.json"
        ))
        .unwrap();
        for row in rows {
            let source = row["source"].as_str().unwrap();
            // Isolate this source-parser matrix from inherited consumers (in
            // particular the older reference-overflow consumer).
            let found = crate::pipeline::lint_str_with(&[&RedundantRegexpQuantifiers], source);
            let actual: Vec<_> = found
                .iter()
                .map(|o| serde_json::json!([o.line, o.col, o.end_line, o.message]))
                .collect();
            assert_eq!(serde_json::json!(actual), row["findings"], "{source}");
        }
    }
    #[test]
    fn deep_and_wide_patterns_use_explicit_linear_stacks() {
        let source = format!("/{}a+{}/", "(?:".repeat(8000), ")+".repeat(8000));
        let tree = expressions(&source.as_bytes()[1..source.len() - 1], false);
        assert_eq!(tree.len(), 8002);
        assert_eq!(tree[0].end, tree.len());
        let source = format!("/(?:{}a+)+/", "b".repeat(100_000));
        assert!(crate::pipeline::lint_str_with(&[&RedundantRegexpQuantifiers], &source).is_empty());
        let source = format!("/(?:a+)+{}/", r"\k<missing".repeat(10_000));
        assert_eq!(
            crate::pipeline::lint_str_with(&[&RedundantRegexpQuantifiers], &source).len(),
            1
        );
    }
    #[test]
    fn greedy_combinations_messages_and_locations() {
        for inner in ["+", "*", "?"] {
            for outer in ["+", "*", "?"] {
                let source = format!("# keep\nfoo = /(?:a{inner}){outer}/\n");
                let found = offenses(&source);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 12, 2));
                let replacement = if inner == outer { inner } else { "*" };
                assert_eq!(
                    found[0].message,
                    format!(
                        "Replace redundant quantifiers `{inner}` and `{outer}` with a single `{replacement}`."
                    )
                );
                assert!(found[0].correction.is_none());
            }
        }
    }
    #[test]
    fn no_offense_lookalikes_captures_and_non_greedy_quantifiers() {
        for pattern in [
            "(?:ab+)+",
            "(?:a|b+)+",
            "(?:a+|b)+",
            r"(?:\d\D+)+",
            "(a+)+",
            "(?:(a+))+",
            "(?:(a)+)+",
            "(?:a++)+",
            "(?:a+)++",
            "(?:a+?)+",
            "(?:a+)+?",
            "(?:a{3,4})+",
            "(?:a+){3,4}",
            "(?:a*#{interpolation})?",
        ] {
            assert!(
                offenses(&format!("foo = /{pattern}/")).is_empty(),
                "{pattern}"
            );
        }
    }
    #[test]
    fn true_begin_children_even_empty_or_constant_skip_the_whole_outer_literal() {
        for part in [
            "#{value}",
            "#{1}",
            "#{}",
            "#{\"\"}",
            "#{nil}",
            "#{raise 'never execute'}",
        ] {
            for pattern in [
                format!("(?:a+)+{part}"),
                format!("{part}(?:a+)+"),
                format!("(?:a+)+ #@value {part} (?:b+)+"),
            ] {
                for literal in [format!("/{pattern}/"), format!("%r{{{pattern}}}x")] {
                    for source in [literal.clone(), format!("if {literal}; :ok; end")] {
                        assert!(
                            crate::pipeline::lint_str_with_corrections(
                                &[&RedundantRegexpQuantifiers],
                                &source
                            )
                            .is_empty(),
                            "{source}"
                        );
                    }
                }
            }
        }
        // Escaped interpolation spelling is static, even with a percent
        // delimiter; a source substring check would incorrectly suppress it.
        let found = offenses(r"%r{(?:a+)+ \#{1} (?:b+)+x}");
        assert_eq!(found.iter().map(|o| o.col).collect::<Vec<_>>(), [8, 22]);
    }
    #[test]
    fn zero_prefixed_octal_escapes_stop_after_three_digits() {
        for octal in [r"\0000", r"\012345", r"\0777", r"\0000000"] {
            for source in [
                format!("/(?:{octal}+)+/"),
                format!("%r{{(?:{octal}+)+}}"),
                format!("/(?: {octal}+ )+/x"),
            ] {
                assert!(offenses(&source).is_empty(), "{source}");
            }
        }
        for octal in [r"\0", r"\00", r"\000", r"\012", r"\077"] {
            let source = format!("/(?:{octal}+)+/");
            let found = offenses(&source);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!((found[0].line, found[0].col), (1, 5 + octal.len() as u32));
            assert_eq!(
                found[0].message,
                "Replace redundant quantifiers `+` and `+` with a single `+`."
            );
            assert!(found[0].correction.is_none());
        }
    }
    #[test]
    fn normalized_sets_extended_and_nested_groups() {
        for (source, cols) in [
            ("foo = /(?:a+)+(?:b+)+/", vec![12, 19]),
            ("foo = /(?:a{1,}){,1}/", vec![12]),
            ("foo = /(?:[abc]+)+/", vec![16]),
            ("foo = /(?: a ? ) + /x", vec![14]),
            ("foo = /(?:(?:(?:(?:a)?))+)/", vec![22]),
            ("foo = /(?:(?:a?)+)+/", vec![15, 17]),
            ("foo = /(?:(?:a+)?)+/", vec![15, 17]),
            ("foo = /(?:(?:a+)+)?/", vec![15, 17]),
        ] {
            let found = offenses(source);
            assert_eq!(
                found.iter().map(|o| o.col).collect::<Vec<_>>(),
                cols,
                "{source}"
            );
        }
    }
}
