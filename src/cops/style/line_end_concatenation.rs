//! `Style/LineEndConcatenation` — source-token adjacency, detection only.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{Location, Node};
use std::collections::{HashMap, HashSet};

/// Filled by the main preorder dispatch; operator calls read it on leave, once
/// both operands have been visited. No reparse or per-candidate subtree walk.
#[derive(Default)]
pub(crate) struct Boundaries {
    starts: HashMap<usize, usize>,
    ends: HashSet<usize>,
    // Pinned tSTRING_BEG/tSTRING_END balancing is not AST delimiter matching:
    // symbols, word arrays, commands and regexps end without a string begin.
    string_begins: Vec<usize>,
    followed_by_regexp_options: HashSet<usize>,
    // Queued heredoc bodies are physically here but lexed at their opening.
    skipped: HashMap<usize, usize>,
    skipped_ends: HashMap<usize, usize>,
}
impl Boundaries {
    fn record(&mut self, opening: Option<Location<'_>>, closing: Option<Location<'_>>) {
        let Some(closing) = closing.filter(|c| matches!(c.as_slice(), b"\"" | b"'")) else {
            return;
        };
        self.ends.insert(closing.end_offset());
        if let Some(opening) = opening.filter(|o| matches!(o.as_slice(), b"\"" | b"'")) {
            self.starts
                .insert(opening.start_offset(), closing.end_offset());
        }
    }
}

/// Only lexer-ignored bytes: comments are tokens and deliberately stop the
/// adjacency check. Each gap belongs to at most a constant number of candidates.
fn next_token(bytes: &[u8], offset: usize, boundaries: &Boundaries) -> usize {
    token_after_gap(bytes, offset, boundaries).0
}
fn token_after_gap(bytes: &[u8], mut offset: usize, boundaries: &Boundaries) -> (usize, bool) {
    let mut newline = false;
    loop {
        if let Some(&end) = boundaries.skipped.get(&offset) {
            offset = end;
            continue;
        }
        match bytes.get(offset) {
            Some(b'\n') => {
                newline = true;
                offset += 1;
            }
            Some(b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c') => offset += 1,
            Some(b'\\') if bytes.get(offset + 1) == Some(&b'\n') => offset += 2,
            Some(b'\\') if bytes.get(offset + 1..offset + 3) == Some(b"\r\n") => offset += 3,
            _ => return (offset, newline),
        }
    }
}

/// Delimiters that contribute string lexer tokens. Plain arrays and unquoted
/// symbols do not; regexp options follow their tSTRING_END as a separate token.
fn delimiters<'a>(node: &Node<'a>) -> Option<(Location<'a>, Location<'a>)> {
    let (opening, closing) = match node {
        Node::StringNode { .. } => {
            let n = node.as_string_node().unwrap();
            (n.opening_loc(), n.closing_loc())
        }
        Node::InterpolatedStringNode { .. } => {
            let n = node.as_interpolated_string_node().unwrap();
            (n.opening_loc(), n.closing_loc())
        }
        Node::SymbolNode { .. } => {
            let n = node.as_symbol_node().unwrap();
            (n.opening_loc(), n.closing_loc())
        }
        Node::InterpolatedSymbolNode { .. } => {
            let n = node.as_interpolated_symbol_node().unwrap();
            (n.opening_loc(), n.closing_loc())
        }
        Node::XStringNode { .. } => {
            let n = node.as_x_string_node().unwrap();
            (Some(n.opening_loc()), Some(n.closing_loc()))
        }
        Node::InterpolatedXStringNode { .. } => {
            let n = node.as_interpolated_x_string_node().unwrap();
            (Some(n.opening_loc()), Some(n.closing_loc()))
        }
        Node::ArrayNode { .. } => {
            let n = node.as_array_node().unwrap();
            (
                n.opening_loc().filter(|o| o.as_slice().starts_with(b"%")),
                n.closing_loc(),
            )
        }
        Node::RegularExpressionNode { .. } => {
            let n = node.as_regular_expression_node().unwrap();
            (Some(n.opening_loc()), Some(n.closing_loc()))
        }
        Node::InterpolatedRegularExpressionNode { .. } => {
            let n = node.as_interpolated_regular_expression_node().unwrap();
            (Some(n.opening_loc()), Some(n.closing_loc()))
        }
        _ => return None,
    };
    Some((opening?, closing?))
}

pub struct LineEndConcatenation;
impl Cop for LineEndConcatenation {
    fn name(&self) -> &'static str {
        "Style/LineEndConcatenation"
    }
    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        _out: &mut Vec<Offense>,
    ) {
        let Some((opening, closing)) = delimiters(node) else {
            return;
        };
        let mut boundaries = file.line_end_boundaries().lock().unwrap();
        if matches!(
            node,
            Node::StringNode { .. } | Node::InterpolatedStringNode { .. }
        ) {
            // Treat a simplified tSTRING as a balanced pair too: it has no
            // children, so the extra push/pop cannot affect enclosing matches.
            boundaries.string_begins.push(opening.start_offset());
        }
        if opening.as_slice().starts_with(b"<<") {
            let start = file.line_start(file.line_of(opening.start_offset()) + 1);
            let end = closing.end_offset();
            let end = *boundaries
                .skipped
                .entry(start)
                .and_modify(|old| *old = (*old).max(end))
                .or_insert(end);
            boundaries.skipped_ends.insert(end, start);
        }
        boundaries.record(Some(opening), Some(closing));
    }
    fn wants_leave_node(&self) -> bool {
        true
    }
    fn leave_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        if let Some((opening, closing)) = delimiters(node) {
            let mut boundaries = file.line_end_boundaries().lock().unwrap();
            if let Some(start) = boundaries.string_begins.pop() {
                if let Some(end) = boundaries.starts.get_mut(&start) {
                    *end = if opening.as_slice().starts_with(b"<<") {
                        opening.end_offset()
                    } else {
                        closing.end_offset()
                    };
                }
                if matches!(
                    node,
                    Node::RegularExpressionNode { .. }
                        | Node::InterpolatedRegularExpressionNode { .. }
                ) {
                    boundaries.followed_by_regexp_options.insert(start);
                }
            }
        }
        let Some(call) = node
            .as_call_node()
            .filter(|c| matches!(c.name().as_slice(), b"+" | b"<<"))
        else {
            return;
        };
        let Some(operator) = call
            .message_loc()
            .filter(|loc| matches!(loc.as_slice(), b"+" | b"<<"))
        else {
            return;
        };
        let bytes = &file.bytes;
        let boundaries = file.line_end_boundaries().lock().unwrap();
        let mut before = operator.start_offset();
        loop {
            if let Some(&start) = boundaries.skipped_ends.get(&before) {
                before = start;
            } else if before > 0
                && (bytes[before - 1].is_ascii_whitespace()
                    || bytes[before - 1] == b'\\'
                        && (bytes.get(before) == Some(&b'\n')
                            || bytes.get(before..before + 2) == Some(b"\r\n")))
            {
                before -= 1;
            } else {
                break;
            }
        }
        let successor = next_token(bytes, operator.end_offset(), &boundaries);
        let Some(&end) = boundaries.starts.get(&successor) else {
            return;
        };
        if !boundaries.ends.contains(&before)
            || file.line_of(operator.start_offset()) == file.line_of(successor)
        {
            return;
        }
        let (after, newline) = token_after_gap(bytes, end, &boundaries);
        // Match the pinned token policy, not general semantic precedence: **,
        // safe navigation and :: are intentionally NOT in its rejection list.
        let dot = bytes.get(after) == Some(&b'.') && bytes.get(after + 1) != Some(&b'.');
        let high = matches!(bytes.get(after), Some(b'%' | b'['))
            || (bytes.get(after) == Some(&b'*') && bytes.get(after + 1) != Some(&b'*'));
        if !boundaries.followed_by_regexp_options.contains(&successor)
            && (dot || (!newline && high))
        {
            return;
        }
        let (line, col) = file.line_col(operator.start_offset());
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            message: format!(
                "Use `\\` instead of `{}` to concatenate multiline strings.",
                String::from_utf8_lossy(operator.as_slice())
            ),
            line,
            col,
            end_line: line,
            correction: None,
        });
    }
}

#[cfg(test)]
mod tests {
    //! Examples ported from RuboCop's `style/line_end_concatenation_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).
    use crate::{offense::Offense, pipeline::lint_str};
    fn offenses(s: &str) -> Vec<Offense> {
        lint_str(s)
            .into_iter()
            .filter(|o| o.cop == "Style/LineEndConcatenation")
            .collect()
    }
    fn assert_at(s: &str, positions: &[(u32, u32, &str)]) {
        assert_eq!(ruby_prism::parse(s.as_bytes()).errors().count(), 0, "{s}");
        let found = offenses(s);
        assert_eq!(found.len(), positions.len(), "{s}: {found:?}");
        // Every matching upstream correction shape remains uncorrected even
        // when the pipeline explicitly requests edits.
        let correction_found =
            crate::cops::testing::correction_offenses(&super::LineEndConcatenation, s);
        assert_eq!(correction_found.len(), positions.len(), "{s}");
        assert!(correction_found.iter().all(|o| o.correction.is_none()));
        for (o, &(line, col, op)) in found.iter().zip(positions) {
            assert_eq!((o.line, o.col, o.end_line), (line, col, line), "{s}");
            assert_eq!(
                o.message,
                format!("Use `\\` instead of `{op}` to concatenate multiline strings.")
            );
            assert!(o.correction.is_none());
        }
    }
    #[test]
    fn pinned_literal_dynamic_and_continued_strings() {
        for op in ["+", "<<"] {
            assert_at(&format!("top = \"test\" {op}\n\"top\"\n"), &[(1, 14, op)]);
            assert_at(
                &format!("top = \"test#{{x}}\" {op}\n\"top\"\n"),
                &[(1, 18, op)],
            );
        }
        assert_at("top = \"test \" \\\n\"foo\" <<\n\"bar\"", &[(2, 7, "<<")]);
        assert_at("top = \"test\" +   \n\"top\"", &[(1, 14, "+")]);
        assert_at("top = \"test\" + \\\n\"top\"", &[(1, 14, "+")]);
    }
    #[test]
    fn pinned_chains_and_mixed_operators() {
        for op in ["+", "<<"] {
            assert_at(
                &format!("top = \"test#{{x}}\" {op}\n\"top\" {op}\n\"foo\""),
                &[(1, 18, op), (2, 7, op)],
            );
        }
        assert_at(
            "top = \"test#{x}\" <<\n\"top\" +\n\"foo\" <<\n\"bar\"",
            &[(1, 18, "<<"), (2, 7, "+"), (3, 7, "<<")],
        );
        assert_at(
            "top = \"test#{x}\" + # comment\n\"foo\" +\n%(bar) +\n\"baz\" +\n\"qux\"",
            &[(4, 7, "+")],
        );
        assert_at(
            "top = \"test#{x}\" <<\n\"top\" + # comment\n\"foo\" +\n\"bar\" +\n%(baz) +\n\"qux\"",
            &[(1, 18, "<<"), (3, 7, "+")],
        );
    }
    #[test]
    fn lexical_predecessors_are_not_a_string_type_proof() {
        for left in [
            "array << \"a\"",
            "%q\"a\"",
            "%w\"a\"",
            ":\"a\"",
            ":\"#{x}\"",
            "%x\"never executed\"",
        ] {
            let s = format!("{left} <<\n \"b\"");
            assert_at(&s, &[(1, left.len() as u32 + 2, "<<")]);
        }
        for left in [
            "%q(a)",
            "%(a)",
            "`never executed`",
            "?a",
            "__FILE__",
            "(\"a\")",
            "\"a\".freeze",
            "name",
            "[\"a\"]",
        ] {
            assert_at(&format!("{left} +\n \"b\""), &[]);
        }
        for right in [
            "%q(a)",
            "%q\"a\"",
            "`never executed`",
            "?a",
            "__FILE__",
            "(\"a\")",
            "name",
            ":\"a\"",
        ] {
            assert_at(&format!("\"a\" +\n {right}"), &[]);
        }
    }
    #[test]
    fn exact_pinned_precedence_list_and_nested_interpolation() {
        for suffix in ["** 2", "&.reverse", "::reverse", " + name", " << name"] {
            assert_at(&format!("\"a\" +\n \"b\"{suffix}"), &[(1, 5, "+")]);
        }
        for suffix in ["* 2", "% value", ".reverse", "[0]", ".+(other)"] {
            assert_at(&format!("\"a\" +\n \"b\"{suffix}"), &[]);
        }
        assert_at("\"a\" +\n \"#{\"#{value}\"}\".reverse", &[]);
        assert_at("\"a\" +\n \"#{\"#{value}\"}\"", &[(1, 5, "+")]);
        assert_at("\"a\" +\n \"b\" \\\n \"c\".reverse", &[(1, 5, "+")]);
        assert_at("\"#{\"a\" +\n \"b\"}\"", &[(1, 8, "+")]);
    }
    #[test]
    fn pinned_balance_stops_at_non_string_literal_end_tokens() {
        for op in ["+", "<<"] {
            for nested in ["%w[x]", "%W[#{x}]", "%i[x]", "%I[#{x}]", "`x`", "%x[x]"] {
                assert_at(
                    &format!("\"a\" {op}\n \"#{{{nested}}}\".reverse"),
                    &[(1, 5, op)],
                );
            }
            for nested in [":\"x\"", ":\"#{x}\"", "%s[x]", "`x`", "%w[x]"] {
                for suffix in [".reverse", "* 2", "% value", "[0]"] {
                    assert_at(&format!("\"a\" {op}\n \"#{{{nested}{suffix}}}\""), &[]);
                }
            }
            // A regexp's (even empty) options token, not its following dot,
            // immediately follows the unmatched tSTRING_END.
            for nested in ["/x/", "/#{x}/i", "%r[x]"] {
                assert_at(
                    &format!("\"a\" {op}\n \"#{{{nested}.reverse}}\".reverse"),
                    &[(1, 5, op)],
                );
            }
            // Ordinary string begins DO balance their ends, even percent strings.
            for nested in ["\"x\"", "\"#{x}\"", "%q[x]", "%Q[#{x}]"] {
                assert_at(&format!("\"a\" {op}\n \"#{{{nested}}}\".reverse"), &[]);
                assert_at(
                    &format!("\"a\" {op}\n \"#{{{nested}.reverse}}\""),
                    &[(1, 5, op)],
                );
            }
        }
        // An early match leaves the inner string's own end to close the outer
        // balance; do not simply select the first special literal in a subtree.
        assert_at(
            "\"a\" +\n \"#{\"#{:\"x\".reverse}\"}\".reverse",
            &[(1, 5, "+")],
        );
    }
    #[test]
    fn comments_parentheses_and_operator_source_are_boundaries() {
        for s in [
            "\"a\".+(\n \"b\")",
            "\"a\".<<(\n \"b\")",
            "\"a\" + (\n \"b\")",
            "(\"a\") +\n \"b\"",
            "\"a\" + # c\n \"b\"",
            "\"a\" +\n # c\n \"b\"",
            "# \"a\" +\n# \"b\"",
            "\"text +\\n more\"",
            "\"a\"\n +\n \"b\"",
            "\"a\" +\n=begin\nc\n=end\n \"b\"",
        ] {
            assert_at(s, &[]);
        }
        assert_at("\"a\" +\n \"b\" # c\n .reverse", &[(1, 5, "+")]);
    }
    #[test]
    fn heredoc_command_and_data_boundaries() {
        for s in [
            "\"a\" + <<~TXT\nb\nTXT\n",
            "(<<~TXT) +\na\nTXT\n \"b\"",
            "`a` +\n \"b\"",
            "\"a\" +\n `b`",
            "\"a\" + <<~`CMD`\nignored\nCMD\n",
            "__END__\n\"a\" +\n \"b\"",
        ] {
            assert_at(s, &[]);
        }
        assert_at(
            "consume(<<~TXT, \"a\" +\nbody\nTXT\n \"b\")\n",
            &[(1, 21, "+")],
        );
        assert_at("x = <<~TXT\n#{\"a\" +\n \"b\"}\nTXT\n", &[(2, 7, "+")]);
    }
    #[test]
    fn detection_only_even_when_corrections_are_requested() {
        use super::LineEndConcatenation;
        use crate::cops::{Cop, testing::correction_offenses};
        assert!(!LineEndConcatenation.safe_fixes());
        for s in [
            "\"a\" +\n \"b\"",
            "\"#{effect}\" <<\n \"b\"",
            "array << \"a\" <<\n \"b\"",
        ] {
            let found = correction_offenses(&LineEndConcatenation, s);
            assert_eq!(found.len(), 1, "{s}");
            assert!(found.iter().all(|o| o.correction.is_none()));
            assert_eq!(
                crate::corrections::apply(s.as_bytes(), &found)
                    .unwrap()
                    .bytes,
                s.as_bytes()
            );
        }
    }
    #[test]
    fn many_candidates_reuse_main_dispatch_boundaries() {
        use super::LineEndConcatenation;
        use crate::{cops::run_cops_with_corrections, source::SourceFile};
        let source = "\"a\" +\n \"#{%w[x]}\".reverse\n".repeat(2000);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        file.record_comments(&parsed);
        assert_eq!(parsed.errors().count(), 0);
        for corrections in [false, true] {
            let mut found = Vec::new();
            assert!(
                run_cops_with_corrections(
                    &[&LineEndConcatenation],
                    &parsed,
                    &file,
                    &mut found,
                    corrections
                )
                .is_empty()
            );
            assert_eq!(found.len(), 2000);
            assert!(found.iter().all(|o| o.correction.is_none()));
            let boundaries = file.line_end_boundaries().lock().unwrap();
            assert_eq!(boundaries.starts.len(), 4000);
            assert_eq!(boundaries.ends.len(), 4000);
            assert!(boundaries.string_begins.is_empty());
        }
    }
    #[test]
    fn queued_bodies_are_skipped_without_scanning_their_contents() {
        use super::LineEndConcatenation;
        use crate::{cops::run_cops_with_corrections, source::SourceFile};
        let header = format!(
            "consume(<<~A, <<~B, \"a\" +\n{}A\n{}B\n \"b\")\n",
            "unrelated + text\n".repeat(1000),
            "more text\n".repeat(1000)
        );
        let source = header.repeat(100);
        let file = SourceFile::new("x.rb".into(), source.into_bytes());
        let parsed = ruby_prism::parse(&file.bytes);
        file.record_comments(&parsed);
        assert_eq!(parsed.errors().count(), 0);
        let mut found = Vec::new();
        assert!(
            run_cops_with_corrections(&[&LineEndConcatenation], &parsed, &file, &mut found, false)
                .is_empty()
        );
        assert_eq!(found.len(), 100);
        let boundaries = file.line_end_boundaries().lock().unwrap();
        assert_eq!(boundaries.skipped.len(), 100);
        assert_eq!(boundaries.starts.len(), 200);
        assert_eq!(boundaries.ends.len(), 200);
    }
    #[test]
    fn distinguishes_range_statement_and_continued_operator_tokens() {
        for s in [
            "\"a\" +\n \"b\"..\"c\"",
            "\"a\" +\n \"b\"...\"c\"",
            "\"a\" +\n \"b\"\n [1]",
            "\"a\" +\n \"b\"\n %w[c]",
        ] {
            assert_at(s, &[(1, 5, "+")]);
        }
        assert_at("\"a\" \\\n +\n \"b\"", &[(2, 2, "+")]);
        assert_at("\"a\" \\\r\n +\r\n \"b\"", &[(2, 2, "+")]);
        for s in ["\"a\" +\n \"b\"\n .reverse", "\"a\" +\n \"b\" \\\n [0]"] {
            assert_at(s, &[]);
        }
    }
    #[test]
    fn pinned_no_offense_examples() {
        for s in [
            "top = \"test\" + \"top\"",
            "content = \"content\" +\n  \" \" * 3",
            "thing = 'a ' +\n  'gniht'.reverse",
            "output = 'value: ' +\n  '%d' % value",
            "'letter: ' +\n  'abcdefghij'[ix]",
            "x3a = 'x' +\n  \"#{'a' + \"#{3}\"}\".reverse",
            "top = \"test\" + # something\n\"top\"",
            "top = \"test\" +\n# something\n\"top\"",
            "top = %(test) +\n\"top\"",
            "top = __FILE__ +\n\"top\"",
        ] {
            assert_at(s, &[]);
        }
    }
}
