//! `Style/ExactRegexpMatch` — prefer equality for whole-string literal patterns.
use crate::{
    cops::{Cop, NodeContext, ast},
    offense::{Correction, Offense},
    source::SourceFile,
};
use ruby_prism::{CallNode, Node};
pub struct ExactRegexpMatch;
impl Cop for ExactRegexpMatch {
    fn name(&self) -> &'static str {
        "Style/ExactRegexpMatch"
    }
    fn safe_fixes(&self) -> bool {
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
        let Some((subject, text, _)) = candidate(&call, file) else {
            return;
        };
        let op = if call.name().as_slice() == b"!~" {
            "!="
        } else {
            "=="
        };
        let quoted = text.replace('\\', "\\\\").replace('\'', "\\'");
        let prefer = format!(
            "{} {op} '{quoted}'",
            String::from_utf8_lossy(subject.location().as_slice())
        );
        let start = node.location().start_offset();
        let end = ast::source_end(node);
        let (line, col) = file.line_col(start);
        out.push(Offense {
            cop: self.name(),
            severity: self.severity(),
            line,
            col,
            end_line: file.line_of(end.saturating_sub(1).max(start)),
            message: format!("Use `{prefer}`."),
            correction: cx
                .corrections_requested()
                .then(|| correct(&call, &prefer, cx))
                .flatten(),
        });
    }
}

/// RuboCop's corrector: only for a method that returns a Boolean, or a
/// result used for its truthiness alone; parenthesized as the receiver or
/// operand of another call.
fn correct(call: &CallNode<'_>, prefer: &str, cx: &NodeContext<'_, '_>) -> Option<Correction> {
    let node = call.as_node();
    let boolean =
        !call.is_safe_navigation() && matches!(call.name().as_slice(), b"===" | b"!~" | b"match?");
    let ancestors: Vec<_> = cx.ancestors().collect();
    if !boolean && !truthiness_only(&node, &ancestors) {
        return None;
    }
    let start = node.location().start_offset();
    let parent = cx.ancestors().find(|n| n.as_arguments_node().is_none());
    let group = parent.and_then(|p| p.as_call_node()).is_some_and(|p| {
        p.receiver()
            .is_some_and(|r| r.location().start_offset() == start)
            || ast::is_operator_method(p.name().as_slice()) && p.name().as_slice() != b"[]"
    });
    Some(Correction {
        range: start..ast::source_end(&node),
        replacement: if group {
            format!("({prefer})")
        } else {
            prefer.into()
        },
    })
}
/// RuboCop's `truthiness_only?`: the value is unused, or only decides a
/// condition (through `!`, `&&`, `||` and grouping).
fn truthiness_only(node: &Node<'_>, ancestors: &[&Node<'_>]) -> bool {
    if !super::correction::value_used(node, ancestors.iter().copied()) {
        return true;
    }
    let span = |n: &Node<'_>| (n.location().start_offset(), n.location().end_offset());
    let Some(index) = ancestors
        .iter()
        .position(|a| a.as_statements_node().is_none())
    else {
        return false;
    };
    let parent = ancestors[index];
    match parent {
        Node::ParenthesesNode { .. } | Node::AndNode { .. } | Node::OrNode { .. } => {
            truthiness_only(parent, &ancestors[index + 1..])
        }
        Node::IfNode { .. } => span(&parent.as_if_node().unwrap().predicate()) == span(node),
        Node::UnlessNode { .. } => {
            span(&parent.as_unless_node().unwrap().predicate()) == span(node)
        }
        Node::WhileNode { .. } => span(&parent.as_while_node().unwrap().predicate()) == span(node),
        Node::UntilNode { .. } => span(&parent.as_until_node().unwrap().predicate()) == span(node),
        _ => parent
            .as_call_node()
            .is_some_and(|c| c.name().as_slice() == b"!"),
    }
}
fn candidate<'pr>(call: &CallNode<'pr>, file: &SourceFile) -> Option<(Node<'pr>, String, bool)> {
    let receiver = call.receiver()?;
    let args = ast::call_arguments(call);
    if args.len() != 1 {
        return None;
    }
    let argument = args.into_iter().next()?;
    let (subject, regexp) = match call.name().as_slice() {
        b"=~" | b"!~" | b"match" | b"match?" => (receiver, argument),
        b"===" if !call.is_safe_navigation() => (argument, receiver),
        _ => return None,
    };
    let regexp = regexp.as_regular_expression_node()?;
    if regexp.closing_loc().as_slice().len() != 1 {
        return None;
    }
    let pattern = regexp.unescaped();
    let (text, whole) = literal_pattern(pattern, file)?;
    Some((subject, String::from_utf8_lossy(text).into_owned(), whole))
}

/// Recognize the pinned parser's initial bos/literal/eos tokens. Escapes are
/// separate tokens, not decoded literal text. Invalid interval braces are
/// ordinary literal characters. Work is linear in this regexp, never the file.
fn literal_pattern<'a>(pattern: &'a [u8], file: &SourceFile) -> Option<(&'a [u8], bool)> {
    let body = pattern.strip_prefix(b"\\A")?;
    let mut i = 0;
    while i < body.len() {
        let b = body[i];
        if matches!(
            b,
            b'\\' | b'.' | b'^' | b'$' | b'*' | b'+' | b'?' | b'[' | b'(' | b')' | b'|'
        ) || b == b'{' && interval(&body[i..])
        {
            break;
        }
        i += file.encoded_character_width(&body[i..]);
    }
    if i == 0 || !body[i..].starts_with(b"\\z") || top_level_alternation(body, file) {
        return None;
    }
    Some((&body[..i], i + 2 == body.len()))
}
fn interval(bytes: &[u8]) -> bool {
    let mut i = 1;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let first = i > 1;
    if bytes.get(i) == Some(&b',') {
        i += 1;
        let second_start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        return (first || i > second_start) && bytes.get(i) == Some(&b'}');
    }
    first && bytes.get(i) == Some(&b'}')
}
fn top_level_alternation(bytes: &[u8], file: &SourceFile) -> bool {
    let (mut escaped, mut extended) = (false, false);
    // Each nested class (including a POSIX [:name:] delimiter) owns its ].
    // State 0 is the start, 1 follows an initial ^, and 2 follows content;
    // a ] in either initial state is literal rather than a closing delimiter.
    let mut classes = Vec::new();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        i += file.encoded_character_width(&bytes[i..]);
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' {
            escaped = true;
            if let Some(state) = classes.last_mut() {
                *state = 2;
            }
            continue;
        }
        if let Some(state) = classes.last_mut() {
            match b {
                b']' if *state == 2 => {
                    classes.pop();
                }
                b'^' if *state == 0 => *state = 1,
                b'[' => {
                    *state = 2;
                    classes.push(0);
                }
                _ => *state = 2,
            }
            continue;
        }
        if b == b'#' && extended {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += file.encoded_character_width(&bytes[i..]);
            }
            continue;
        }
        match b {
            b'[' => classes.push(0),
            b'(' if bytes[i..].starts_with(b"?#") => {
                i += 2;
                while i < bytes.len() && bytes[i] != b')' {
                    i += file.encoded_character_width(&bytes[i..]);
                }
                i += usize::from(i < bytes.len());
            }
            b'(' => {
                // Inline option switches affect # comments and alternation at
                // the root as well as inside option-scoped groups.
                if bytes.get(i) == Some(&b'?') {
                    let (mut j, mut enabled, mut next) = (i + 1, true, extended);
                    while let Some(&option) = bytes.get(j) {
                        match option {
                            b'-' => enabled = false,
                            b'x' => next = enabled,
                            b'i' | b'm' | b'a' | b'd' | b'u' => {}
                            _ => break,
                        }
                        j += 1;
                    }
                    if j > i + 1 && matches!(bytes.get(j), Some(b')' | b':')) {
                        if bytes[j] == b':' {
                            groups.push(extended);
                        }
                        extended = next;
                        i = j + 1;
                        continue;
                    }
                }
                groups.push(extended);
            }
            b')' => {
                extended = groups.pop().unwrap_or(extended);
            }
            b'|' if groups.is_empty() => return true,
            _ => {}
        }
    }
    false
}
#[cfg(test)]
mod tests {
    //! Ported from RuboCop style/exact_regexp_match_spec.rb (MIT,
    //! Copyright (c) 2012-26 Bozhidar Batsov).
    use super::*;
    fn offenses(s: &str) -> Vec<Offense> {
        crate::pipeline::lint_str(s)
            .into_iter()
            .filter(|o| o.cop == ExactRegexpMatch.name())
            .collect()
    }
    #[test]
    fn pinned_methods_quotes_positions_and_messages() {
        for (s, subject, op, text) in [
            (r"string =~ /\Astring\z/", "string", "==", "string"),
            (r"/\Astring\z/ === string", "string", "==", "string"),
            (r"string.match(/\Astring\z/)", "string", "==", "string"),
            (r"string&.match(/\Astring\z/)", "string", "==", "string"),
            (r"string.match?(/\Astring\z/)", "string", "==", "string"),
            (r"string !~ /\Astring\z/", "string", "!=", "string"),
            (r"string =~ /\Afoo'bar\z/", "string", "==", r"foo\'bar"),
        ] {
            let found = offenses(&format!("# header\n  {s}\n"));
            assert_eq!(found.len(), 1, "{s}");
            assert_eq!((found[0].line, found[0].col, found[0].end_line), (2, 3, 2));
            assert_eq!(found[0].message, format!("Use `{subject} {op} '{text}'`."));
        }
    }
    #[test]
    fn pinned_no_offense_lookalikes() {
        for s in [
            r"string === /\Astring\z/",
            r"match(/\Astring\z/)",
            r"string =~ /\Astring#{interpolation}\z/",
            r"/\A0+\z/ === string",
            r"string =~ /\Astring.*\z/",
            "string =~ /^string$/",
            r"string =~ /\Astring\z/i",
            r"string =~ /^\P$/",
            r"string.match?(/\Astring\z/, 1)",
            r"string =~ /\A\z/",
        ] {
            assert!(offenses(s).is_empty(), "{s}");
        }
    }
    #[test]
    fn literal_tokens_delimiters_options_encodings_and_match_state_lookalikes() {
        for pattern in [
            r"/\Afoo\z/",
            r"%r{\Afoo/bar\z}",
            r"/\Afoo\/bar\z/",
            r"/\Afoo{bar}]-_ #!\z/",
            r"/\Acafé\z/",
            r"/\Afoo\zbar/",
            r"/\Afoo\z(?:x|y)/",
        ] {
            let s = format!("s.match?({pattern})");
            assert_eq!(offenses(&s).len(), 1, "{s}");
        }
        for pattern in [
            r"/\Afoo\z/i",
            r"/\Afoo\z/m",
            r"/\Afoo\z/x",
            r"/\Afoo\z/o",
            r"/\Afoo\z/n",
            r"/\Afoo\z/u",
            r"/\Afoo\z/e",
            r"/\Afoo\z/s",
            r"/\A(foo)\z/",
            r"/\A(?<x>foo)\z/",
            r"/\Afoo\1\z/",
            r"/\Afoo\k<x>\z/",
            r"/\Afoo\.bar\z/",
            r"/\Afoo\\bar\z/",
            r"/\Afoo\x41\z/",
            r"/\Afoo\u0041\z/",
            r"/\Afoo\nbar\z/",
            r"/\Afoo{1}\z/",
            r"/\Afoo{,2}\z/",
            r"/\Afoo[ab]\z/",
            r"/\Afoo\z|bar/",
            r"/\Afoo\Z/",
            r"/\Afoo#{x}\z/",
        ] {
            let s = format!("s.match?({pattern})");
            assert!(offenses(&s).is_empty(), "{s}");
        }
    }
    #[test]
    fn pinned_literal_interval_and_inline_extended_comment_tokens() {
        for pattern in [
            r"/\Afoo{,}\z/",
            r"/\Afoo\z(?x)#|comment/",
            r"/\Afoo\z(?#ignored|comment)/",
        ] {
            assert_eq!(
                offenses(&format!("s.match?({pattern})")).len(),
                1,
                "{pattern}"
            );
        }
        for pattern in [
            r"/\Afoo\z(?x)#ignored".to_string() + "\n|bar/",
            r"/\Afoo\z(?x:comment)|bar/".into(),
        ] {
            assert!(
                offenses(&format!("s.match?({pattern})")).is_empty(),
                "{pattern}"
            );
        }
    }
    #[test]
    fn posix_and_nested_classes_keep_alternation_at_the_correct_depth() {
        for pattern in [
            r"/\Afoo\z[[:alpha:]|bar]/",
            r"/\Afoo\z[[a-z]&&[^x]|bar]/",
            r"/\Afoo\z[\]|bar]/",
        ] {
            let found = offenses(&format!("s.match?({pattern})"));
            assert_eq!(found.len(), 1, "{pattern}");
            assert_eq!((found[0].line, found[0].col), (1, 1));
            assert_eq!(found[0].message, "Use `s == 'foo'`.");
        }
        for pattern in [
            r"/\Afoo\z[[:alpha:](]|bar/",
            r"/\Afoo\z[[a-z]&&[^x](]|bar/",
            r"/\Afoo\z[\](]|bar/",
        ] {
            assert!(
                offenses(&format!("s.match?({pattern})")).is_empty(),
                "{pattern}"
            );
        }
    }
    #[test]
    fn invalid_pattern_does_not_hide_following_valid_pattern() {
        let found = offenses("string =~ /^\\P$/\nstring.match(/\\Astring\\z/)\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 1));
    }
}
