//! `Layout/SpaceAroundKeyword` — a keyword needs a blank before and after it.
//!
//! A port of RuboCop's cop (97f30c4, MIT). Each of upstream's callbacks
//! maps to Prism nodes, and the keyword ranges it checks are taken from the
//! parser gem's location maps:
//!
//! - `and`/`or` (the keyword spellings only): the operator;
//! - `block`/`numblock`/`itblock` (blocks of calls and `super`, and lambdas):
//!   a `do` opening, and the `end` closing a `do`;
//! - `break`, `next`, `return`, `yield`, `super`, `zsuper`, `defined?`,
//!   `BEGIN`, `END`, `ensure`, `when`, `in` clauses, `rescue` clauses and the
//!   `rescue` modifier, one-line `in` (`match_pattern_p`; Ruby 3.4's `=>` is
//!   never checked) and `not`: the keyword;
//! - `case`/`case ... in`: `case` and `else`;
//! - `for`, `while`, `until` (never the `begin ... end while` form): the
//!   `while`/`until` keyword, a `do`, and the `end` closing a `do`;
//! - `if`/`unless` (never a ternary; modifiers and pattern guards too): the
//!   keyword, a `then`, the `else` or next `elsif`, and the `end` (an `elsif`
//!   has no `end` of its own);
//! - an explicit `begin`: `begin` and `end`; a `rescue` with an `else`: the
//!   `else`.
//!
//! A keyword is reported (`Space before keyword `x` is missing.`) when the
//! character before it is not a Ruby `\s` or one of `( | { [ ; , * =`, and
//! the node is not preceded by an operator: climbing RuboCop's ancestors
//! (Prism's, viewed as the parser gem's: argument lists are transparent, a
//! block wraps its call), an `and`/`or`/range ancestor or an operator-method
//! `send` (`+`, `!`, `[]`, …) reached through plain sends exempts it. An
//! `end` is checked for the missing space before it only. Otherwise the
//! keyword is reported (`Space after keyword `x` is missing.`) when the
//! character after it is not a Ruby `\s` or one of `; , # \ ) } ] .`, the
//! end of the file, a `&.`, a `(` after `break`, `defined?`, `next`, `not`,
//! `rescue`, `super` or `yield`, a `[` after `super` or `yield`, or a `::`
//! after `super`. As upstream, one range is reported once: a keyword missing
//! both blanks is reported for the one before, and an `elsif` is checked as
//! the `else` of its parent first. `Layout/SpaceBeforeBlockBraces` is fixed
//! at `space`, so upstream's `no_space` exemption for a brace block right
//! after the keyword never applies.
//!
//! The correction inserts one space before or after the keyword. It is
//! withheld where the blank could change how Ruby lexes the code: before the
//! keyword unless the previous byte is an identifier or number character, a
//! non-ASCII byte, or one of `) ] } " ' ` ? ! :` (`foo &begin end` would
//! become a binary `&`); after `not`, `yield`, `super` and `defined?`, after
//! which Ruby reads the next token as a command argument once a blank is
//! seen, unless the next byte starts an identifier, number, string,
//! backtick, instance or global variable (or a brace block after `super`)
//! (`super-1` would become `super(-1)`). After every other keyword Ruby
//! starts an expression with or without the blank. Every edit is withheld
//! for the listed source observers.
use std::ops::Range;

use ruby_prism::{Location, Node};

use super::{
    space_after_colon::{observed, ruby_space},
    space_before_comment::bom_end,
};
use crate::{
    cops::{Cop, NodeContext},
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct SpaceAroundKeyword;

/// RuboCop's `OPERATOR_METHODS`.
const OPERATOR_METHODS: &[&[u8]] = &[
    b"|", b"^", b"&", b"<=>", b"==", b"===", b"=~", b">", b">=", b"<", b"<=", b"<<", b">>", b"+",
    b"-", b"*", b"/", b"%", b"**", b"~", b"+@", b"-@", b"!@", b"~@", b"[]", b"[]=", b"!", b"!=",
    b"!~", b"`",
];

/// How one keyword range is checked.
#[derive(Clone, Copy)]
enum Check {
    /// `check_keyword`, with the node's `preceded_by_operator?`.
    Keyword { operator: bool },
    /// `check_end`: the blank before an `end` only.
    End,
    /// An `elsif` as its parent's `else` (with the parent's
    /// `preceded_by_operator?`) and then as the `elsif` node's own keyword
    /// (never preceded by an operator: its parent is an `if`).
    Elsif { operator: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

fn keyword(location: Location<'_>) -> Range<usize> {
    location.start_offset()..location.end_offset()
}

fn is_keyword(location: &Option<Location<'_>>, text: &[u8]) -> bool {
    location.as_ref().is_some_and(|loc| loc.as_slice() == text)
}

/// RuboCop's `preceded_by_operator?` for the node `cx` describes.
fn preceded_by_operator(node: &Node<'_>, cx: &NodeContext<'_, '_>) -> bool {
    let mut ancestors = cx.ancestors().peekable();
    if node.as_block_node().is_some() {
        // The parser gem's block wraps its call: the call is not an ancestor.
        ancestors.next();
    }
    while let Some(ancestor) = ancestors.next() {
        match ancestor {
            Node::ArgumentsNode { .. } => {}
            Node::AndNode { .. } | Node::OrNode { .. } | Node::RangeNode { .. } => return true,
            Node::CallNode { .. } => {
                let call = ancestor.as_call_node().unwrap();
                if call.is_safe_navigation()
                    || ancestors
                        .peek()
                        .is_some_and(|parent| parent.as_match_write_node().is_some())
                {
                    // A `csend`, or a `match_with_lvasgn`.
                    return false;
                }
                if OPERATOR_METHODS.contains(&call.name().as_slice()) {
                    return true;
                }
                if call
                    .block()
                    .is_some_and(|block| block.as_block_node().is_some())
                {
                    // The parser gem's `block` is the send's parent.
                    return false;
                }
            }
            _ => return false,
        }
    }
    false
}

/// The keyword ranges `node` checks, in upstream's order.
fn checks(
    node: &Node<'_>,
    cx: &NodeContext<'_, '_>,
    file: &SourceFile,
) -> Vec<(Range<usize>, Check)> {
    let operator = || preceded_by_operator(node, cx);
    let plain = |location: Location<'_>| {
        (
            keyword(location),
            Check::Keyword {
                operator: operator(),
            },
        )
    };
    let unoperated =
        |location: Location<'_>| (keyword(location), Check::Keyword { operator: false });
    let mut out = Vec::new();
    match node {
        Node::AndNode { .. } => {
            let location = node.as_and_node().unwrap().operator_loc();
            if location.as_slice() == b"and" {
                out.push(plain(location));
            }
        }
        Node::OrNode { .. } => {
            let location = node.as_or_node().unwrap().operator_loc();
            if location.as_slice() == b"or" {
                out.push(plain(location));
            }
        }
        Node::BlockNode { .. } | Node::LambdaNode { .. } => {
            let (opening, closing) = if let Some(block) = node.as_block_node() {
                (block.opening_loc(), block.closing_loc())
            } else {
                let lambda = node.as_lambda_node().unwrap();
                (lambda.opening_loc(), lambda.closing_loc())
            };
            if opening.as_slice() == b"do" {
                out.push(plain(opening));
                out.push((keyword(closing), Check::End));
            }
        }
        Node::BreakNode { .. } => out.push(plain(node.as_break_node().unwrap().keyword_loc())),
        Node::NextNode { .. } => out.push(plain(node.as_next_node().unwrap().keyword_loc())),
        Node::ReturnNode { .. } => out.push(plain(node.as_return_node().unwrap().keyword_loc())),
        Node::YieldNode { .. } => out.push(plain(node.as_yield_node().unwrap().keyword_loc())),
        Node::SuperNode { .. } => out.push(plain(node.as_super_node().unwrap().keyword_loc())),
        Node::DefinedNode { .. } => out.push(plain(node.as_defined_node().unwrap().keyword_loc())),
        Node::ForwardingSuperNode { .. } => {
            let start = node.location().start_offset();
            if file.bytes.get(start..start + 5) == Some(b"super") {
                out.push((
                    start..start + 5,
                    Check::Keyword {
                        operator: operator(),
                    },
                ));
            }
        }
        Node::PreExecutionNode { .. } => {
            out.push(plain(node.as_pre_execution_node().unwrap().keyword_loc()));
        }
        Node::PostExecutionNode { .. } => {
            out.push(plain(node.as_post_execution_node().unwrap().keyword_loc()));
        }
        Node::CaseNode { .. } => {
            let case = node.as_case_node().unwrap();
            out.push(plain(case.case_keyword_loc()));
            if let Some(clause) = case.else_clause() {
                out.push(plain(clause.else_keyword_loc()));
            }
        }
        Node::CaseMatchNode { .. } => {
            let case = node.as_case_match_node().unwrap();
            out.push(plain(case.case_keyword_loc()));
            if let Some(clause) = case.else_clause() {
                out.push(plain(clause.else_keyword_loc()));
            }
        }
        Node::EnsureNode { .. } => {
            out.push(plain(node.as_ensure_node().unwrap().ensure_keyword_loc()));
        }
        Node::ForNode { .. } => {
            let for_loop = node.as_for_node().unwrap();
            if let Some(opening) = for_loop.do_keyword_loc() {
                out.push(plain(opening));
                out.push((keyword(for_loop.end_keyword_loc()), Check::End));
            }
        }
        Node::WhileNode { .. } | Node::UntilNode { .. } => {
            let (begin_modifier, word, opening, closing) = if let Some(node) = node.as_while_node()
            {
                (
                    node.is_begin_modifier(),
                    node.keyword_loc(),
                    node.do_keyword_loc(),
                    node.closing_loc(),
                )
            } else {
                let node = node.as_until_node().unwrap();
                (
                    node.is_begin_modifier(),
                    node.keyword_loc(),
                    node.do_keyword_loc(),
                    node.closing_loc(),
                )
            };
            if begin_modifier {
                // `while_post`/`until_post` have no callback.
                return out;
            }
            if let Some(opening) = opening {
                out.push(plain(opening));
                if let Some(closing) = closing {
                    out.push((keyword(closing), Check::End));
                }
            }
            out.push(plain(word));
        }
        Node::IfNode { .. } => {
            let conditional = node.as_if_node().unwrap();
            let Some(word) = conditional.if_keyword_loc() else {
                // A ternary.
                return out;
            };
            let elsif = word.as_slice() == b"elsif";
            if !elsif {
                out.push(plain(word));
            }
            match conditional.subsequent() {
                Some(Node::ElseNode { .. }) => {
                    let clause = conditional.subsequent().unwrap();
                    out.push(plain(clause.as_else_node().unwrap().else_keyword_loc()));
                }
                Some(Node::IfNode { .. }) => {
                    let clause = conditional.subsequent().unwrap();
                    if let Some(next) = clause.as_if_node().unwrap().if_keyword_loc() {
                        out.push((
                            keyword(next),
                            Check::Elsif {
                                operator: operator(),
                            },
                        ));
                    }
                }
                _ => {}
            }
            let then = conditional.then_keyword_loc();
            if is_keyword(&then, b"then") {
                out.push(plain(then.unwrap()));
            }
            if !elsif && let Some(closing) = conditional.end_keyword_loc() {
                out.push((keyword(closing), Check::End));
            }
        }
        Node::UnlessNode { .. } => {
            let conditional = node.as_unless_node().unwrap();
            out.push(plain(conditional.keyword_loc()));
            if let Some(clause) = conditional.else_clause() {
                out.push(plain(clause.else_keyword_loc()));
            }
            let then = conditional.then_keyword_loc();
            if is_keyword(&then, b"then") {
                out.push(plain(then.unwrap()));
            }
            if let Some(closing) = conditional.end_keyword_loc() {
                out.push((keyword(closing), Check::End));
            }
        }
        Node::InNode { .. } => out.push(plain(node.as_in_node().unwrap().in_loc())),
        Node::WhenNode { .. } => out.push(plain(node.as_when_node().unwrap().keyword_loc())),
        Node::MatchPredicateNode { .. } => {
            out.push(plain(
                node.as_match_predicate_node().unwrap().operator_loc(),
            ));
        }
        Node::BeginNode { .. } => {
            let begin = node.as_begin_node().unwrap();
            if let Some(word) = begin.begin_keyword_loc() {
                out.push(plain(word));
                if let Some(closing) = begin.end_keyword_loc() {
                    out.push((keyword(closing), Check::End));
                }
            }
            // `on_rescue`: the `rescue` node's ancestors are never sends.
            if begin.rescue_clause().is_some()
                && let Some(clause) = begin.else_clause()
            {
                out.push(unoperated(clause.else_keyword_loc()));
            }
        }
        // `on_resbody`: a `resbody`'s parent is its `rescue` node.
        Node::RescueNode { .. } => {
            out.push(unoperated(node.as_rescue_node().unwrap().keyword_loc()))
        }
        Node::RescueModifierNode { .. } => {
            out.push(unoperated(
                node.as_rescue_modifier_node().unwrap().keyword_loc(),
            ));
        }
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            let selector = call.message_loc();
            if call.name().as_slice() == b"!" && is_keyword(&selector, b"not") {
                out.push(plain(selector.unwrap()));
            }
        }
        _ => {}
    }
    out
}

/// Whether the character before `range` needs no blank.
fn space_before_missing(file: &SourceFile, range: &Range<usize>) -> bool {
    range.start > bom_end(file)
        && !matches!(
            file.bytes[range.start - 1],
            b'(' | b'|' | b'{' | b'[' | b';' | b',' | b'*' | b'='
        )
        && !ruby_space(file.bytes[range.start - 1])
}

fn space_after_missing(file: &SourceFile, range: &Range<usize>) -> bool {
    let text = &file.bytes[range.clone()];
    let rest = &file.bytes[range.end..];
    let Some(&next) = rest.first() else {
        return false;
    };
    if (next == b'[' && matches!(text, b"super" | b"yield"))
        || (next == b'('
            && matches!(
                text,
                b"break" | b"defined?" | b"next" | b"not" | b"rescue" | b"super" | b"yield"
            ))
        || rest.starts_with(b"&.")
        || (text == b"super" && rest.starts_with(b"::"))
    {
        return false;
    }
    !(ruby_space(next) || matches!(next, b';' | b',' | b'#' | b'\\' | b')' | b'}' | b']' | b'.'))
}

/// Whether a blank inserted on `side` of the keyword keeps Ruby's tokens.
fn insertion_safe(file: &SourceFile, range: &Range<usize>, side: Side) -> bool {
    match side {
        Side::Before => {
            let byte = file.bytes[range.start - 1];
            byte.is_ascii_alphanumeric()
                || byte >= 0x80
                || matches!(
                    byte,
                    b'_' | b')' | b']' | b'}' | b'"' | b'\'' | b'`' | b'?' | b'!' | b':'
                )
        }
        Side::After => {
            let text = &file.bytes[range.clone()];
            if !matches!(text, b"not" | b"yield" | b"super" | b"defined?") {
                return true;
            }
            let byte = file.bytes[range.end];
            byte.is_ascii_alphanumeric()
                || byte >= 0x80
                || matches!(byte, b'_' | b'"' | b'\'' | b'`' | b'@' | b'$')
                || (byte == b'{' && text == b"super")
        }
    }
}

/// The side reported for one range, if any.
fn side(file: &SourceFile, range: &Range<usize>, check: Check) -> Option<Side> {
    let before = space_before_missing(file, range);
    match check {
        Check::End => before.then_some(Side::Before),
        Check::Keyword { operator } => {
            if before && !operator {
                Some(Side::Before)
            } else {
                space_after_missing(file, range).then_some(Side::After)
            }
        }
        Check::Elsif { operator } => {
            if before && !operator {
                Some(Side::Before)
            } else if space_after_missing(file, range) {
                Some(Side::After)
            } else {
                before.then_some(Side::Before)
            }
        }
    }
}

impl Cop for SpaceAroundKeyword {
    fn name(&self) -> &'static str {
        "Layout/SpaceAroundKeyword"
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
        for (range, check) in checks(node, cx, file) {
            let Some(side) = side(file, &range, check) else {
                continue;
            };
            let text = String::from_utf8_lossy(&file.bytes[range.clone()]);
            let message = match side {
                Side::Before => format!("Space before keyword `{text}` is missing."),
                Side::After => format!("Space after keyword `{text}` is missing."),
            };
            let at = match side {
                Side::Before => range.start,
                Side::After => range.end,
            };
            let correction = (cx.corrections_requested()
                && insertion_safe(file, &range, side)
                && !observed(cx, file))
            .then(|| Correction {
                range: at..at,
                replacement: " ".into(),
            });
            let (line, col) = file.line_col(range.start);
            out.push(Offense {
                correction,
                cop: self.name(),
                severity: self.severity(),
                message,
                line,
                end_line: line,
                col,
            });
        }
    }
}

#[cfg(test)]
mod tests;
