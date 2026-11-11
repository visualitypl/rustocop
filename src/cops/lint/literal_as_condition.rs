//! `Lint/LiteralAsCondition` — flags literals used as conditions, where the
//! branch taken is fixed.
//!
//! Matches RuboCop's semantics:
//!
//! - `if`/`elsif`/`unless`/ternary/modifier conditions that are literals,
//!   except when the branch the literal selects is empty and there is an
//!   `elsif`/`else` to fall back to (RuboCop registers no offense when it
//!   has no meaningful correction).
//! - `while`/`until` conditions (also modifier and `begin…end while`
//!   forms), except the idiomatic infinite loops `while true` and
//!   `until false`.
//! - A literal `case` subject (arrays only when all their elements are
//!   basic literals, never interpolated strings), and the `when` of a
//!   subject-less `case` whose conditions are all literals.
//! - A literal `case … in` subject, unless any pattern binds a variable;
//!   `!`/`&&`/`||`/parentheses in that subject are searched for literals.
//! - The left operand of `&&` when truthy and of `||` when falsey, anywhere.
//! - The operand of `!`/`not`, searched through `!`, `&&`/`||` and
//!   parentheses (a literal right under `&&` is left to the `&&` check).
//!
//! Offenses with the same range are reported once, as in RuboCop. Quoted
//! source retains whitespace (the reference buffer normalizes CRLF to LF). Pattern
//! guards (`in x if true`) are not conditions in RuboCop's AST and are left
//! alone. The offense is reported at the literal.

use ruby_prism::{CaseMatchNode, CaseNode, IfNode, Node, UnlessNode, Visit};

use crate::{
    cops::{Cop, NodeContext, ast::is_literal},
    offense::Offense,
    source::SourceFile,
};

/// The `Lint/LiteralAsCondition` cop.
pub struct LiteralAsCondition;

impl Cop for LiteralAsCondition {
    fn name(&self) -> &'static str {
        "Lint/LiteralAsCondition"
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        _cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let mut reporter = Reporter {
            cop: self,
            file,
            out,
        };
        match node {
            Node::AndNode { .. } => {
                let left = node.as_and_node().unwrap().left();
                if is_truthy_literal(&left) {
                    reporter.report(&left);
                }
            }
            Node::OrNode { .. } => {
                let left = node.as_or_node().unwrap().left();
                if is_falsey_literal(&left) {
                    reporter.report(&left);
                }
            }
            Node::IfNode { .. } => check_if(&node.as_if_node().unwrap(), &mut reporter),
            Node::UnlessNode { .. } => check_unless(&node.as_unless_node().unwrap(), &mut reporter),
            Node::WhileNode { .. } => {
                check_loop(
                    &node.as_while_node().unwrap().predicate(),
                    b"true",
                    &mut reporter,
                );
            }
            Node::UntilNode { .. } => {
                check_loop(
                    &node.as_until_node().unwrap().predicate(),
                    b"false",
                    &mut reporter,
                );
            }
            Node::CaseNode { .. } => check_case(&node.as_case_node().unwrap(), &mut reporter),
            Node::CaseMatchNode { .. } => {
                check_case_match(&node.as_case_match_node().unwrap(), &mut reporter);
            }
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if call.is_safe_navigation() || call.name().as_slice() != b"!" {
                    return;
                }
                let Some(operand) = call.receiver() else {
                    return;
                };
                if is_literal(&operand) {
                    reporter.report(&operand);
                } else {
                    reporter.search(&operand);
                }
            }
            _ => {}
        }
    }
}

/// Pushes offenses, once per range.
struct Reporter<'a> {
    cop: &'a LiteralAsCondition,
    file: &'a SourceFile,
    out: &'a mut Vec<Offense>,
}

impl Reporter<'_> {
    fn report(&mut self, literal: &Node<'_>) {
        let location = literal.location();
        self.report_range(location.start_offset(), location.end_offset());
    }

    fn report_range(&mut self, start: usize, end: usize) {
        let (line, col) = super::array_literal_in_regexp::reference_line_col(start, self.file);
        let projected = self.file.match_pattern_source(&self.file.bytes[start..end]);
        let source = projected
            .utf8_chunks()
            .map(|chunk| {
                let mut text = chunk.valid().replace("\r\n", "\n");
                for byte in chunk.invalid() {
                    text.push_str(&format!("\\x{byte:02X}"));
                }
                text
            })
            .collect::<String>();
        let message = format!("Literal `{source}` appeared as a condition.");
        // RuboCop reports a range once per cop; the same position and
        // message mean the same range.
        let cop = self.cop.name();
        if self
            .out
            .iter()
            .any(|o| o.cop == cop && o.line == line && o.col == col && o.message == message)
        {
            return;
        }
        self.out.push(Offense {
            correction: None,
            cop,
            severity: self.cop.severity(),
            message,
            line,
            end_line: self.file.line_of(end.saturating_sub(1).max(start)),
            col,
        });
    }

    /// RuboCop's `handle_node`: a literal is reported unless it is the
    /// direct operand of `&&`; `!`, `&&`, `||` and parentheses are searched.
    fn handle(&mut self, node: &Node<'_>, under_and: bool) {
        if is_literal(node) {
            if !under_and {
                self.report(node);
            }
        } else {
            self.search(node);
        }
    }

    /// RuboCop's `check_node`.
    fn search(&mut self, node: &Node<'_>) {
        match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                let prefix_bang = !call.is_safe_navigation()
                    && call.name().as_slice() == b"!"
                    && call
                        .message_loc()
                        .is_some_and(|message| message.as_slice() == b"!");
                if prefix_bang && let Some(operand) = call.receiver() {
                    self.handle(&operand, false);
                }
            }
            Node::AndNode { .. } => {
                let and = node.as_and_node().unwrap();
                self.handle(&and.left(), true);
                self.handle(&and.right(), true);
            }
            Node::OrNode { .. } => {
                let or = node.as_or_node().unwrap();
                self.handle(&or.left(), false);
                self.handle(&or.right(), false);
            }
            Node::ParenthesesNode { .. } => {
                let Some(body) = node.as_parentheses_node().unwrap().body() else {
                    return;
                };
                match body.as_statements_node() {
                    Some(statements) => {
                        let mut iter = statements.body().iter();
                        if let (Some(only), None) = (iter.next(), iter.next()) {
                            self.handle(&only, false);
                        }
                    }
                    None => self.handle(&body, false),
                }
            }
            _ => {}
        }
    }
}

fn is_truthy_literal(node: &Node<'_>) -> bool {
    is_literal(node) && !is_falsey_literal(node)
}

fn is_falsey_literal(node: &Node<'_>) -> bool {
    matches!(node, Node::FalseNode { .. } | Node::NilNode { .. })
}

/// `if`, `elsif`, ternary and modifier `if` (RuboCop's `on_if`).
fn check_if(node: &IfNode<'_>, reporter: &mut Reporter<'_>) {
    let keyword = node.if_keyword_loc();
    let keyword = keyword.as_ref().map(|keyword| keyword.as_slice());
    if node.end_keyword_loc().is_none()
        && keyword == Some(b"if".as_slice())
        && is_pattern_guard(node.location().start_offset(), reporter.file)
    {
        return;
    }
    let condition = node.predicate();
    if !is_literal(&condition) {
        return;
    }
    let else_branch = match node.subsequent() {
        None => false,
        Some(subsequent) => match subsequent.as_else_node() {
            Some(else_node) => else_node.statements().is_some(),
            // An `elsif`.
            None => true,
        },
    };
    let is_elsif = keyword == Some(b"elsif".as_slice());
    // A ternary has no `else` keyword in RuboCop's terms.
    let has_else = keyword.is_some() && node.subsequent().is_some();
    let taken = is_truthy_literal(&condition);
    let surviving = if taken {
        node.statements().is_some()
    } else {
        else_branch
    };
    if !surviving && (is_elsif || has_else) {
        return;
    }
    reporter.report(&condition);
}

/// `unless` and modifier `unless` (RuboCop's `on_if`).
fn check_unless(node: &UnlessNode<'_>, reporter: &mut Reporter<'_>) {
    if node.end_keyword_loc().is_none()
        && is_pattern_guard(node.location().start_offset(), reporter.file)
    {
        return;
    }
    let condition = node.predicate();
    if !is_literal(&condition) {
        return;
    }
    let else_clause = node.else_clause();
    let taken = is_falsey_literal(&condition);
    let surviving = if taken {
        node.statements().is_some()
    } else {
        else_clause
            .as_ref()
            .is_some_and(|else_clause| else_clause.statements().is_some())
    };
    if !surviving && else_clause.is_some() {
        return;
    }
    reporter.report(&condition);
}

/// Whether a modifier-form `if`/`unless` starting at `start` is a `case …
/// in` pattern guard: Prism wraps `in pattern if guard` in an `IfNode`,
/// RuboCop's AST in an `if_guard`. Only the `in` keyword can precede one
/// (not a method `.in`, a symbol `:in`, a comment ending in "in" or the
/// name in `def in`). Comments between `in` and the pattern are skipped.
fn is_pattern_guard(start: usize, file: &SourceFile) -> bool {
    let end = skip_back_blanks(start, file);
    if !word_ends_at(&file.bytes, end, b"in") {
        return false;
    }
    let before = skip_back_blanks(end - 2, file);
    !word_ends_at(&file.bytes, before, b"def")
}

/// The offset just past the last token before `offset`, skipping
/// whitespace, line continuations and comments.
fn skip_back_blanks(mut offset: usize, file: &SourceFile) -> usize {
    let bytes = &file.bytes;
    let comments = file.comments();
    loop {
        while offset > 0 && matches!(bytes[offset - 1], b' ' | b'\t' | b'\n' | b'\r' | b'\\') {
            offset -= 1;
        }
        let after = comments.partition_point(|comment| comment.start < offset);
        match after.checked_sub(1).map(|index| &comments[index]) {
            Some(comment) if comment.end >= offset => offset = comment.start,
            _ => return offset,
        }
    }
}

/// Whether the identifier-like token ending at `end` is exactly `word`
/// (not part of a longer name, a method call `.word`, a symbol `:word` or
/// a variable `@word`/`$word`).
fn word_ends_at(bytes: &[u8], end: usize, word: &[u8]) -> bool {
    if end < word.len() || &bytes[end - word.len()..end] != word {
        return false;
    }
    let start = end - word.len();
    !(start > 0
        && matches!(bytes[start - 1], b'.' | b':' | b'@' | b'$' | b'_' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | 0x80..))
}

/// `while`/`until` and their modifier and post forms.
fn check_loop(condition: &Node<'_>, idiomatic: &[u8], reporter: &mut Reporter<'_>) {
    if condition.location().as_slice() == idiomatic {
        return;
    }
    if is_literal(condition) {
        reporter.report(condition);
    }
}

fn check_case(node: &CaseNode<'_>, reporter: &mut Reporter<'_>) {
    match node.predicate() {
        Some(subject) => {
            if is_literal(&subject) {
                check_case_subject(&subject, reporter);
            }
        }
        None => {
            for when in node.conditions().iter() {
                let Some(when) = when.as_when_node() else {
                    continue;
                };
                let conditions = when.conditions();
                if !conditions.iter().all(|condition| is_literal(&condition)) {
                    continue;
                }
                let (Some(first), Some(last)) =
                    (conditions.iter().next(), conditions.iter().last())
                else {
                    continue;
                };
                reporter.report_range(
                    first.location().start_offset(),
                    last.location().end_offset(),
                );
            }
        }
    }
}

fn check_case_match(node: &CaseMatchNode<'_>, reporter: &mut Reporter<'_>) {
    let Some(subject) = node.predicate() else {
        return;
    };
    let mut finder = MatchVariableFinder {
        pattern_depth: 0,
        found: false,
    };
    finder.visit(&node.as_node());
    if finder.found {
        return;
    }
    check_case_subject(&subject, reporter);
}

/// RuboCop's `check_case`.
fn check_case_subject(subject: &Node<'_>, reporter: &mut Reporter<'_>) {
    if let Some(array) = subject.as_array_node()
        && !is_primitive_array(&array.as_node(), reporter.file)
    {
        return;
    }
    if is_reference_dstr(subject, reporter.file) {
        return;
    }
    reporter.handle(subject, false);
}

use super::reference_basic_literal::{is_basic_literal, is_dstr as is_reference_dstr};

/// An array of reference basic literals and such arrays. Physical multiline
/// strings are dstr, not basic literals, at every nesting level.
fn is_primitive_array(node: &Node<'_>, file: &SourceFile) -> bool {
    node.as_array_node().is_some_and(|array| {
        array
            .elements()
            .iter()
            .all(|element| is_basic_literal(&element, file) || is_primitive_array(&element, file))
    })
}

/// Finds a pattern variable (RuboCop's `match_var`) anywhere below a node.
struct MatchVariableFinder {
    /// How many patterns enclose the current node.
    pattern_depth: usize,
    found: bool,
}

impl MatchVariableFinder {
    fn visit_pattern<'pr>(&mut self, pattern: &Node<'pr>) {
        self.pattern_depth += 1;
        self.visit(pattern);
        self.pattern_depth -= 1;
    }
}

impl<'pr> Visit<'pr> for MatchVariableFinder {
    fn visit_in_node(&mut self, node: &ruby_prism::InNode<'pr>) {
        self.visit_pattern(&node.pattern());
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
    }

    fn visit_match_predicate_node(&mut self, node: &ruby_prism::MatchPredicateNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    fn visit_pinned_expression_node(&mut self, node: &ruby_prism::PinnedExpressionNode<'pr>) {
        let depth = std::mem::take(&mut self.pattern_depth);
        self.visit(&node.expression());
        self.pattern_depth = depth;
    }

    fn visit_local_variable_target_node(
        &mut self,
        _node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        if self.pattern_depth > 0 {
            self.found = true;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Offense and no-offense examples are ported from RuboCop's
    //! `spec/rubocop/cop/lint/literal_as_condition_spec.rb`
    //! (MIT License, Copyright (c) 2012-26 Bozhidar Batsov).

    use super::*;
    use crate::pipeline::lint_str;

    fn offenses(source: &str) -> Vec<Offense> {
        let result = ruby_prism::parse(source.as_bytes());
        assert_eq!(result.errors().count(), 0, "fixture must parse: {source:?}");
        lint_str(source)
            .into_iter()
            .filter(|o| o.cop == "Lint/LiteralAsCondition")
            .collect()
    }

    fn found(source: &str) -> Vec<(u32, u32, String)> {
        offenses(source)
            .into_iter()
            .map(|o| (o.line, o.col, o.message))
            .collect()
    }

    fn positions(source: &str) -> Vec<(u32, u32)> {
        offenses(source).iter().map(|o| (o.line, o.col)).collect()
    }

    fn at(line: u32, col: u32, literal: &str) -> (u32, u32, String) {
        (
            line,
            col,
            format!("Literal `{literal}` appeared as a condition."),
        )
    }

    const TRUTHY: [&str; 6] = ["1", "2.0", "[1]", "{}", ":sym", ":\"#{a}\""];
    const FALSEY: [&str; 2] = ["nil", "false"];

    /// Templates with `L` for the literal, and where it is reported.
    const TRUTHY_OFFENSES: &[(&str, &[(u32, u32)])] = &[
        ("if L\n  top\nend\n", &[(1, 4)]),
        ("if L\n  top\nelse\n  foo\nend\n", &[(1, 4)]),
        ("if condition\n  top\nelsif L\n  foo\nend\n", &[(3, 7)]),
        ("if L\n  x = 1\nelsif L\n  x = 2\nend\n", &[(1, 4), (3, 7)]),
        (
            "if condition\n  top\nelsif L\n  foo\nelse\n  bar\nend\n",
            &[(3, 7)],
        ),
        (
            "if condition\n  top # comment 1\nelsif L\n  foo # comment 2\nelse\n  bar\nend\n",
            &[(3, 7)],
        ),
        ("top if L\n", &[(1, 8)]),
        ("L ? top : bar\n", &[(1, 1)]),
        ("unless L\n  top\nend\n", &[(1, 8)]),
        ("top unless L\n", &[(1, 12)]),
        ("while L\n  top\nend\n", &[(1, 7)]),
        ("begin\n  top\nend while L\n", &[(3, 11)]),
        ("until L\n  top\nend\n", &[(1, 7)]),
        ("begin\n  top\nend until L\n", &[(3, 11)]),
        ("case L\nwhen x then top\nend\n", &[(1, 6)]),
        ("case\nwhen L then top\nend\n", &[(2, 6)]),
        ("case L\nin CONST then top\nend\n", &[(1, 6)]),
        ("if L && x\n  top\nend\n", &[(1, 4)]),
        ("if L && true\n  top\nend\n", &[(1, 4)]),
        ("if x && !(L && a) && y && z\n  top\nend\n", &[(1, 11)]),
        ("if !L\n  top\nend\n", &[(1, 5)]),
        ("!L\n", &[(1, 2)]),
        ("not(L)\n", &[(1, 5)]),
    ];

    const TRUTHY_NO_OFFENSES: &[&str] = &[
        "if L\nelse\n  foo\nend\n",
        "if L\nelsif condition\n  foo\nend\n",
        "if condition\n  top\nelsif L\nend\n",
        "unless L\n  top\nelse\nend\n",
        "case x\nwhen L then top\nend\n",
        "case L\nin x then top\nend\n",
        "case x\nin L then top\nend\n",
        "if x && L\n  top\nend\n",
        "if test(L)\n  top\nend\n",
        "if (a || L).something\n  top\nend\n",
        "case a || L\nwhen b\n  top\nend\n",
    ];

    const FALSEY_OFFENSES: &[(&str, &[(u32, u32)])] = &[
        ("if L\n  top\nend\n", &[(1, 4)]),
        ("if L\n  top\nelse\n  foo\nend\n", &[(1, 4)]),
        ("if L\n  top\nelsif condition\n  foo\nend\n", &[(1, 4)]),
        (
            "if condition\n  top\nelsif L\n  foo\nelse\n  bar\nend\n",
            &[(3, 7)],
        ),
        ("top if L\n", &[(1, 8)]),
        ("L ? top : bar\n", &[(1, 1)]),
        ("unless L\n  top\nend\n", &[(1, 8)]),
        ("top unless L\n", &[(1, 12)]),
        ("while L\n  top\nend\n", &[(1, 7)]),
        ("begin\n  top\nend while L\n", &[(3, 11)]),
        ("begin\n  top\n  foo\nend while L\n", &[(4, 11)]),
        ("case L\nwhen x\n  top\nend\n", &[(1, 6)]),
        ("if L || x\n  top\nend\n", &[(1, 4)]),
    ];

    const FALSEY_NO_OFFENSES: &[&str] = &[
        "if L\n  top\nelse\nend\n",
        "if condition\n  top\nelsif L\n  foo\nend\n",
        "unless L\nelse\n  foo\nend\n",
    ];

    fn check_templates(literals: &[&str], offending: &[(&str, &[(u32, u32)])], accepted: &[&str]) {
        for literal in literals {
            for (template, expected) in offending {
                let source = template.replace('L', literal);
                let found = found(&source);
                let wanted: Vec<_> = expected
                    .iter()
                    .map(|&(line, col)| at(line, col, literal))
                    .collect();
                assert_eq!(found, wanted, "for {source:?}");
            }
            for template in accepted {
                let source = template.replace('L', literal);
                assert!(offenses(&source).is_empty(), "should not flag {source:?}");
            }
        }
    }

    #[test]
    fn safe_navigation_negation_is_not_an_on_send_callback() {
        assert!(offenses("1&.!").is_empty());
        assert!(offenses("nil&.!").is_empty());
        assert!(offenses("!(1&.!)").is_empty());
        assert!(offenses("case 1&.!; in X; end").is_empty());
        assert_eq!(found("1.!"), [at(1, 1, "1")]);
    }

    #[test]
    fn flags_truthy_literals_as_rubocop_does() {
        check_templates(&TRUTHY, TRUTHY_OFFENSES, TRUTHY_NO_OFFENSES);
    }

    #[test]
    fn flags_falsey_literals_as_rubocop_does() {
        check_templates(&FALSEY, FALSEY_OFFENSES, FALSEY_NO_OFFENSES);
    }

    #[test]
    fn flags_nil_in_until() {
        assert_eq!(found("until nil\n  top\nend\n"), [at(1, 7, "nil")]);
        assert_eq!(found("begin\n  top\nend until nil\n"), [at(3, 11, "nil")]);
    }

    #[test]
    fn accepts_idiomatic_infinite_loops() {
        for source in [
            "while true\n  break if condition\nend\n",
            "begin\n  break if condition\nend while true\n",
            "until false\n  break if condition\nend\n",
            "begin\n  break if condition\nend until false\n",
            "loop_once while true\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn case_subjects_must_be_primitive_literals() {
        assert_eq!(
            found("case [1, 2, [3, 4]]\nwhen [1, 2, 5] then top\nend\n"),
            [at(1, 6, "[1, 2, [3, 4]]")]
        );
        assert_eq!(
            found("case [1, 2, [3, 4]]\nin [1, 2, 5] then top\nend\n"),
            [at(1, 6, "[1, 2, [3, 4]]")]
        );
        for source in [
            "case [1, 2, x]\nwhen [1, 2, 5] then top\nend\n",
            "case [1, 2, [x, 1]]\nwhen [1, 2, 5] then top\nend\n",
            "case \"#{x}\"\nwhen [1, 2, 5] then top\nend\n",
            "case [1, 2, x]\nin [1, 2, 5] then top\nend\n",
            "case [1, 2, [x, 1]]\nin [1, 2, 5] then top\nend\n",
            "case { a: 1, b: 2, c: 3 }\nin a: Integer => m\nend\n",
            "case \"#{x}\"\nin [1, 2, 5] then top\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn retains_multiline_literal_source_and_reference_newlines() {
        for literal in [
            "[1,\n2]",
            "'a\nb'",
            "[\n  1,\n\t2\n]",
            "[1, # original comment\n\t2]",
            "{a: 1,\n b: 2}",
            "%q(a\n\tb)",
            ":\"a\nb\"",
            "'a\\nb'",
            "'a\rb'",
        ] {
            for (template, col) in [
                ("if L; x;end\n", 4),
                ("!(L)\n", 3),
                ("!(x || L)\n", 8),
                ("case L;when x;end\n", 6),
                ("while L; x;end\n", 7),
            ] {
                for nl in ["\n", "\r\n"] {
                    let source = template.replace('L', literal).replace('\n', nl);
                    let actual = offenses(&source);
                    // The pinned adapter represents static physical multiline
                    // strings as dstr; case subjects exclude dstr, unlike !/if.
                    if template.starts_with("case") && matches!(literal, "'a\nb'" | "%q(a\n\tb)") {
                        assert!(actual.is_empty(), "{source:?}");
                        continue;
                    }
                    assert_eq!(
                        actual
                            .iter()
                            .map(|o| (o.line, o.col, o.message.clone()))
                            .collect::<Vec<_>>(),
                        [at(1, col, literal)],
                        "{source:?}"
                    );
                    assert_eq!(actual[0].end_line, 1 + literal.matches('\n').count() as u32);
                    assert!(actual[0].correction.is_none());
                }
            }
        }
    }

    #[test]
    fn case_arrays_exclude_physical_multiline_string_elements_recursively() {
        for literal in ["'a\nb'", "\"a\nb\"", "%q(a\nb)", "%Q(a\nb)"] {
            for array in [format!("[{literal}]"), format!("[1, [[{literal}]]]")] {
                for branch in ["when x", "in X"] {
                    for nl in ["\n", "\r\n"] {
                        let source = format!("case {array}; {branch}; end\n").replace('\n', nl);
                        assert!(found(&source).is_empty(), "{source:?}");
                    }
                }
            }
        }
        // Escaped newlines are not physical part boundaries.
        for literal in ["'a\\nb'", "\"a\\nb\"", "\"a\\\nb\""] {
            let array = format!("[1, [[{literal}]]]");
            for branch in ["when x", "in X"] {
                assert_eq!(
                    found(&format!("case {array}; {branch}; end\n")),
                    [at(1, 6, &array)]
                );
            }
        }
    }

    #[test]
    fn case_empty_percent_and_continuation_only_strings_are_reference_dstrs() {
        // All 24 shapes are native-valid but
        // the pinned adapter emits zero-child dstr, not a basic str literal.
        for literal in ["%q()", "%Q()", "%()", "\"\\\n\""] {
            for subject in [
                literal.to_string(),
                format!("[{literal}]"),
                format!("[1, [[{literal}]]]"),
            ] {
                for branch in ["when x", "in X"] {
                    for nl in ["\n", "\r\n"] {
                        let source = format!("case {subject}; {branch}; end\n").replace('\n', nl);
                        assert!(found(&source).is_empty(), "{source:?}");
                    }
                }
            }
            // dstr is still a literal in ordinary boolean contexts. Do not
            // suppress these findings as a side effect of case classification.
            assert_eq!(
                found(&format!("if {literal}; x; end\n")),
                [at(1, 4, literal)]
            );
        }
        for literal in ["''", "\"\"", "\"\\u{}\"", "\"a\\\nb\""] {
            for branch in ["when x", "in X"] {
                assert_eq!(
                    found(&format!("case {literal}; {branch}; end\n")),
                    [at(1, 6, literal)]
                );
            }
        }
    }

    #[test]
    fn case_continued_strings_follow_emitted_children_not_cooked_emptiness() {
        for literal in ["\"a\\n\\\n\"", "\"\\n\\\n\"", "%Q(a\\n\\\n)"] {
            for subject in [literal.to_string(), format!("[1, [[{literal}]]]")] {
                for branch in ["when x", "in X"] {
                    for nl in ["\n", "\r\n"] {
                        let source = format!("case {subject}; {branch}; end\n").replace('\n', nl);
                        assert!(found(&source).is_empty(), "{source:?}");
                    }
                }
            }
        }
        assert_eq!(found("case \"a\\n\"; when x; end"), [at(1, 6, "\"a\\n\"")]);
    }

    #[test]
    fn case_arrays_exclude_reference_dynamic_symbols_not_static_empty_values() {
        for literal in [":\"\"", ":''", "%s()", ":\"a\nb\"", ":'a\nb'", ":\"\\\n\""] {
            for array in [format!("[{literal}]"), format!("[1, [[{literal}]]]")] {
                for branch in ["when x", "in X"] {
                    assert!(
                        found(&format!("case {array}; {branch}; end")).is_empty(),
                        "{array:?}"
                    );
                }
            }
            // Unlike dstr, direct dsym subjects are still reported upstream.
            assert_eq!(
                found(&format!("case {literal}; when x; end")),
                [at(1, 6, literal)]
            );
        }
        for literal in [":x", ":\"x\"", ":\"\\u{}\"", ":\"a\\\nb\""] {
            let array = format!("[{literal}]");
            assert_eq!(
                found(&format!("case {array}; when x; end")),
                [at(1, 6, &array)]
            );
        }
    }

    #[test]
    fn case_continued_squiggly_heredocs_remain_single_strings() {
        for head in ["<<~X", "<<~\"X\""] {
            for indent in ["", "  ", "\t"] {
                for branch in ["when x", "in X"] {
                    for nl in ["\n", "\r\n"] {
                        let source =
                            format!("case {head}; {branch}; end\n{indent}a\\\n{indent}b\nX\n")
                                .replace('\n', nl);
                        assert_eq!(found(&source), [at(1, 6, head)], "{source:?}");
                    }
                }
            }
        }
        for head in ["<<~X", "<<~'X'"] {
            let source = format!("case {head}; when x; end\n  a\n  b\nX\n");
            assert!(found(&source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn subjectless_case_retains_multiline_condition_list_spacing() {
        assert_eq!(
            found("case\nwhen 1,\n\t:a, 'b\nc' then x\nend\n"),
            [at(2, 6, "1,\n\t:a, 'b\nc'")]
        );
        for source in [
            "if x; puts 'a\nb'; end\n",
            "case [x,\n2]; when x; end\n",
            "!(x && 'a\nb')\n",
            "case\nwhen x,\n1 then x\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn subjectless_case_reports_all_when_conditions() {
        assert_eq!(
            found("case\nwhen 1, :a then top\nwhen x, 2 then bar\nend\n"),
            [at(2, 6, "1, :a")]
        );
    }

    #[test]
    fn flags_if_without_surviving_branch_used_as_value() {
        assert_eq!(found("foo.bar =\n  if true\n  end\n"), [at(2, 6, "true")]);
        assert_eq!(found("x = if false\n  foo\nend\n"), [at(1, 8, "false")]);
        assert_eq!(found("if 42\nend\n"), [at(1, 4, "42")]);
    }

    #[test]
    fn flags_post_loops_with_loop_control_keywords() {
        assert_eq!(
            positions("begin\n  redo if foo\n  break\nend while nil\n"),
            [(4, 11)]
        );
        assert_eq!(
            positions("begin\n  next if foo\n  bar\nend until true\n"),
            [(4, 11)]
        );
        assert_eq!(
            positions("begin\n  items.each { |item| next if item }\nend while false\n"),
            [(3, 11)]
        );
    }

    #[test]
    fn flags_and_or_operands_anywhere() {
        for (source, position) in [
            ("def foo\n  puts 123 && return if bar?\nend\n", (2, 8)),
            ("def foo\n  puts nil || return if bar?\nend\n", (2, 8)),
            (
                "def clone(opts = nil || (return self))\n  super\nend\n",
                (1, 18),
            ),
            (
                "def foo(opts = 123 && (foo; return self))\n  super\nend\n",
                (1, 16),
            ),
            ("def foo\n  baz? if 123 && return\nend\n", (2, 11)),
            (
                "def foo\n  bar do\n    puts 123 && break if baz?\n  end\nend\n",
                (3, 10),
            ),
            (
                "def foo\n  bar do\n    puts 123 && next if baz?\n  end\nend\n",
                (3, 10),
            ),
            ("x = 1 && y\n", (1, 5)),
            ("x = (false or y)\n", (1, 6)),
        ] {
            assert_eq!(positions(source), [position], "for {source:?}");
        }
    }

    #[test]
    fn searches_negations_and_reports_each_range_once() {
        // `!(a || 1)`: the `||` operand is searched from the `!`.
        assert_eq!(found("!(a || 1)\n"), [at(1, 8, "1")]);
        // A direct `&&` operand is the `&&` check's business.
        assert!(found("!(a && 1)\n").is_empty());
        // Found by both `||` and `!`: reported once.
        assert_eq!(found("!(nil || x)\n"), [at(1, 3, "nil")]);
        assert_eq!(found("!!1\n"), [at(1, 3, "1")]);
        assert_eq!(found("case !1\nin X\nend\n"), [at(1, 7, "1")]);
        assert_eq!(found("case a || 1\nin X\nend\n"), [at(1, 11, "1")]);
    }

    #[test]
    fn accepts_case_match_with_pattern_variables_anywhere() {
        for source in [
            "case 1\nin Integer => n then n\nend\n",
            "case 1\nin [*rest] then rest\nend\n",
            "case 1\nin { a: } then a\nend\n",
            "case 1\nin X\n  y in [z]\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
        assert_eq!(found("x = 1\ncase 1\nin ^x then 2\nend\n"), [at(2, 6, "1")]);
    }

    #[test]
    fn ignores_pattern_guards() {
        for source in [
            "case x\nin Integer if true then 1\nend\n",
            "case x\nin Integer unless nil\nend\n",
            "case x\nin y if 1\nend\n",
            "case x\nin # comment\n  Integer if true\nend\n",
            "case x\nin # one\n  # two\n  Integer if true\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
        assert_eq!(
            found("case x\nin Integer\n  top if 1\nend\n"),
            [at(3, 10, "1")]
        );
        // Not the `in` keyword.
        for source in [
            "# log in\ntop if 1\n",
            "x.in\ntop if 1\n",
            "foo :in\ntop if 1\n",
            "login\ntop if 1\n",
        ] {
            assert_eq!(found(source), [at(2, 8, "1")], "for {source:?}");
        }
        // A method named `in`.
        assert_eq!(found("def in\n  top if 1\nend\n"), [at(2, 10, "1")]);
        assert_eq!(
            found("def in # comment\n  top if 1\nend\n"),
            [at(2, 10, "1")]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for source in [
            "if x\n  top\nend\n",
            "if (1)\n  top\nend\n",
            "while (true)\n  top\nend\n",
            "if 1..2\n  top\nend\n",
            "if /foo/\n  top\nend\n",
            "if __ENCODING__\n  top\nend\n",
            "x = foo || 1\n",
            "x = 1 || foo\n",
            "x = nil && foo\n",
            "foo(true)\n",
            "x = [1, 2]\n",
            "a.!\n",
            "case\nwhen x then 1\nend\n",
        ] {
            assert!(offenses(source).is_empty(), "should not flag {source:?}");
        }
    }

    #[test]
    fn honors_directives() {
        assert!(
            offenses("if 1 # rubocop:disable Lint/LiteralAsCondition\n  top\nend\n").is_empty()
        );
    }
}
