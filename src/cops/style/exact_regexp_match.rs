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
        let Some((subject, text, whole)) = candidate(&call, file) else {
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
                .then(|| correct(&call, &subject, &text, whole, &prefer, cx, file))
                .flatten(),
        });
    }
}

/// Only match? is match-state-free. Other operators alter $~ even when their
/// result is discarded; unknown subjects can override either compared method.
fn correct(
    call: &CallNode<'_>,
    subject: &Node<'_>,
    text: &str,
    whole: bool,
    prefer: &str,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Option<Correction> {
    if !whole || !text.is_ascii() || !literal_predicate(call) {
        return None;
    }
    let root = cx.ancestors().next_back()?;
    let node = call.as_node();
    let start = node.location().start_offset();
    let facts = file.exact_regexp_safety(root);
    let range = start..ast::source_end(&node);
    if facts.changed
        || facts.first_effect.is_some_and(|end| end <= start)
        || super::correction::contains_comment(file, range.clone())
        || super::correction::touches_directive(file, range.clone())
        || file.contains_heredoc(root, subject)
        || file.heredoc_pending_at(root, start)
        || file.bytes[range.clone()].contains(&b'\n')
    {
        return None;
    }
    let parent = cx.ancestors().find(|n| n.as_arguments_node().is_none());
    let group = parent.and_then(|p| p.as_call_node()).is_some_and(|p| {
        p.receiver()
            .is_some_and(|r| r.location().start_offset() == start)
            || ast::is_operator_method(p.name().as_slice()) && p.name().as_slice() != b"[]"
    });
    super::literal_spelling::safe_edit(
        file,
        cx,
        Correction {
            range,
            replacement: if group {
                format!("({prefer})")
            } else {
                prefer.into()
            },
        },
    )
}
fn literal_predicate(call: &CallNode<'_>) -> bool {
    call.name().as_slice() == b"match?"
        && call.block().is_none()
        && call
            .receiver()
            .and_then(|n| n.as_string_node())
            .is_some_and(|s| s.unescaped().is_ascii())
}

/// Pattern spelling is observable through Regexp#source/inspect/equality even
/// when matching is unchanged. Accept only a discarded top-level literal or a
/// direct argument to a proven core String#match? (which cannot expose $~).
/// Reuse the lazy main-AST dispatch proof; no per-candidate tree walk/reparse.
/// Implicit last-line matches are NOT discarded literals: $~.regexp escapes.
pub(super) fn spelling_value_is_unobserved(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> bool {
    if node.as_regular_expression_node().is_none() {
        return false;
    }
    let mut ancestors = cx.ancestors();
    let Some(parent) = ancestors.next() else {
        return false;
    };
    if parent.as_statements_node().is_some() {
        return ancestors
            .next()
            .is_some_and(|n| n.as_program_node().is_some());
    }
    if parent.as_arguments_node().is_none() {
        return false;
    }
    let Some(call) = ancestors.next().and_then(|n| n.as_call_node()) else {
        return false;
    };
    if !literal_predicate(&call) || ast::call_arguments(&call).len() != 1 {
        return false;
    }
    let Some(root) = cx.ancestors().next_back() else {
        return false;
    };
    let facts = file.exact_regexp_safety(root);
    !facts.changed
        && facts
            .first_effect
            .is_none_or(|end| end > node.location().start_offset())
}

/// Lazy one-walk dispatch proof, shared across candidates. No local type
/// inference, deferred scopes, declarations, interpolation or unknown effects.
#[derive(Default)]
pub(crate) struct Safety {
    changed: bool,
    first_effect: Option<usize>,
    #[cfg(test)]
    pub(crate) visits: usize,
}
impl Safety {
    pub(crate) fn build(root: &Node<'_>) -> Self {
        let mut facts = Self::default();
        crate::cops::walk::walk(
            &mut SafetyBuilder {
                facts: &mut facts,
                predicates: Vec::new(),
            },
            root,
        );
        facts
    }
}
struct SafetyBuilder<'a> {
    facts: &'a mut Safety,
    // One frame per ancestor. Modifier predicates execute before the earlier
    // body, so effects in their source ranges must invalidate that body too.
    predicates: Vec<Option<(std::ops::Range<usize>, usize)>>,
}
impl<'pr> crate::cops::walk::NodeVisitor<'pr> for SafetyBuilder<'_> {
    fn enter(&mut self, node: Node<'pr>) {
        #[cfg(test)]
        {
            self.facts.visits += 1;
        }
        let conditional = if let Some(n) = node.as_if_node() {
            Some((n.predicate(), n.statements()))
        } else {
            node.as_unless_node()
                .map(|n| (n.predicate(), n.statements()))
        };
        self.predicates
            .push(conditional.and_then(|(predicate, statements)| {
                let loc = predicate.location();
                statements
                    .filter(|s| s.location().start_offset() < loc.start_offset())
                    .map(|_| {
                        (
                            loc.start_offset()..loc.end_offset(),
                            node.location().start_offset(),
                        )
                    })
            }));
        if let Some(call) = node.as_call_node() {
            let args = ast::call_arguments(&call);
            if !literal_predicate(&call)
                || args.len() != 1
                || args[0].as_regular_expression_node().is_none()
            {
                let loc = call.location();
                let effect = self.predicates.iter().flatten().fold(
                    loc.end_offset(),
                    |offset, (predicate, body_start)| {
                        if predicate.contains(&loc.start_offset())
                            && loc.end_offset() <= predicate.end
                        {
                            offset.min(*body_start)
                        } else {
                            offset
                        }
                    },
                );
                self.facts.first_effect = Some(
                    self.facts
                        .first_effect
                        .map_or(effect, |old| old.min(effect)),
                );
            }
            self.facts.changed |= call.block().is_some();
        } else {
            self.facts.changed |= !matches!(
                node,
                Node::ProgramNode { .. }
                    | Node::StatementsNode { .. }
                    | Node::ArgumentsNode { .. }
                    | Node::StringNode { .. }
                    | Node::RegularExpressionNode { .. }
                    | Node::ParenthesesNode { .. }
                    | Node::LocalVariableReadNode { .. }
                    | Node::LocalVariableWriteNode { .. }
                    | Node::ArrayNode { .. }
                    | Node::TrueNode { .. }
                    | Node::FalseNode { .. }
                    | Node::NilNode { .. }
                    | Node::IfNode { .. }
                    | Node::UnlessNode { .. }
                    | Node::ElseNode { .. }
                    | Node::AndNode { .. }
                    | Node::OrNode { .. }
                    | Node::SourceLineNode { .. }
                    | Node::GlobalVariableReadNode { .. }
                    | Node::BackReferenceReadNode { .. }
                    | Node::NumberedReferenceReadNode { .. }
            );
        }
    }
    fn leave(&mut self) {
        self.predicates.pop();
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
    fn canonical_literal_match_predicate_corrections() {
        use crate::cops::testing::assert_correction;
        for (before, after) in [
            (r"'string'.match?(/\Astring\z/)", "'string' == 'string'"),
            (
                r"'foo/bar'.match?(/\Afoo\/bar\z/)",
                "'foo/bar' == 'foo/bar'",
            ),
            (
                r#""foo'bar"&.match?(/\Afoo'bar\z/)"#,
                r#""foo'bar" == 'foo\'bar'"#,
            ),
            (r"!'string'.match?(/\Astring\z/)", "!('string' == 'string')"),
            (
                r"'string'.match?(/\Astring\z/).to_s",
                "('string' == 'string').to_s",
            ),
            (r"result = 'x'.match?(/\Ax\z/)", "result = 'x' == 'x'"),
            (
                r"a = 'x'.match?(/\Ax\z/) if true; p a",
                "a = 'x' == 'x' if true; p a",
            ),
            (
                r"a = 'x'.match?(/\Ax\z/) unless false; p a",
                "a = 'x' == 'x' unless false; p a",
            ),
        ] {
            assert_correction(&ExactRegexpMatch, before, after);
        }
        // A later modifier effect must not invalidate an earlier eager call.
        assert_correction(
            &ExactRegexpMatch,
            r"a = 'x'.match?(/\Ax\z/); b = 'x'.match?(/\Ax\z/) if ''.class.undef_method('==')",
            r"a = 'x' == 'x'; b = 'x'.match?(/\Ax\z/) if ''.class.undef_method('==')",
        );
    }
    #[test]
    fn risky_dispatch_result_values_state_and_unsupported_literal_edits_are_withheld() {
        use crate::cops::testing::assert_correction;
        for s in [
            r"string.match?(/\Astring\z/)",
            r"nil&.match?(/\Astring\z/)",
            r"'x' =~ /\Ax\z/",
            r"'x'.match(/\Ax\z/)",
            r"/\Ax\z/ === 'x'",
            r"'x' !~ /\Ax\z/",
            r"if 'x' =~ /\Ax\z/; p $~; end",
            r"'x'.match?(/\Ax\zbar/)",
            r"'café'.match?(/\Acafé\z/)",
            r#""\xFF".match?(/\Ax\z/)"#,
            r"'x'.match?(/\Ax\z/) { side_effect }",
            "class String; def ==(x); false; end; end\n'x'.match?(/\\Ax\\z/)",
            "require 'anything'\n'x'.match?(/\\Ax\\z/)",
            "def f; 'x'.match?(/\\Ax\\z/); end",
        ] {
            assert_correction(&ExactRegexpMatch, s, s);
        }
    }
    #[test]
    fn invalid_pattern_does_not_hide_following_valid_pattern() {
        let found = offenses("string =~ /^\\P$/\nstring.match(/\\Astring\\z/)\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].col), (2, 1));
    }
}
