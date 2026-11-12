//! The parser-gem `block`/`numblock`/`itblock` node of the pinned Prism
//! translation, for the block layout cops: a Prism `BlockNode`
//! together with the call (or `super`) it is attached to, or a
//! `LambdaNode` (`->` is the block's `send`). Only the facts the cops read:
//! the block expression's start (`node.source_range`, which starts at the
//! call), `loc.begin`/`loc.end`, `node.arguments` and the extent of
//! `node.body`.
use std::ops::Range;

use ruby_prism::Node;

use crate::cops::NodeContext;

/// A parser `block`, `numblock` or `itblock` node.
pub(crate) struct ParserBlock<'pr> {
    /// `node.source_range.begin_pos`: the call's (or `->`'s) start.
    pub start: usize,
    /// `node.loc.begin`: `{` or `do`.
    pub opening: Range<usize>,
    /// `node.loc.end`: `}` or `end`.
    pub closing: Range<usize>,
    /// The Prism parameters: `BlockParametersNode` (`|...|`, or a lambda's
    /// `(...)`), `NumberedParametersNode` or `ItParametersNode`.
    pub parameters: Option<Node<'pr>>,
    /// The Prism body: `StatementsNode`, or an implicit `BeginNode` holding
    /// `rescue`/`else`/`ensure` clauses.
    pub body: Option<Node<'pr>>,
    /// A `->` literal.
    pub lambda: bool,
}

impl<'pr> ParserBlock<'pr> {
    /// The parser block `node` stands for, if any. A `BlockNode`'s parent in
    /// `cx` is the call (or `super`) it is attached to.
    pub(crate) fn of(node: &Node<'pr>, cx: &NodeContext<'_, 'pr>) -> Option<Self> {
        if let Some(block) = node.as_block_node() {
            let parent = cx.parent()?;
            let opening = block.opening_loc();
            let closing = block.closing_loc();
            return Some(Self {
                start: parent.location().start_offset(),
                opening: opening.start_offset()..opening.end_offset(),
                closing: closing.start_offset()..closing.end_offset(),
                parameters: block.parameters(),
                body: block.body(),
                lambda: false,
            });
        }
        let lambda = node.as_lambda_node()?;
        let opening = lambda.opening_loc();
        let closing = lambda.closing_loc();
        Some(Self {
            start: node.location().start_offset(),
            opening: opening.start_offset()..opening.end_offset(),
            closing: closing.start_offset()..closing.end_offset(),
            parameters: lambda.parameters(),
            body: lambda.body(),
            lambda: true,
        })
    }

    /// `node.arguments` when `node.arguments?`: written block parameters
    /// (positional, keyword or block-local), never numbered or `it`
    /// parameters, and never empty `||` or `()`.
    pub(crate) fn arguments(&self) -> Option<Node<'pr>> {
        let parameters = self.parameters.as_ref()?.as_block_parameters_node()?;
        (parameters.parameters().is_some() || parameters.locals().iter().next().is_some())
            .then(|| parameters.as_node())
    }

    /// `node.body.source_range`: the body's parser expression.
    pub(crate) fn body_range(&self) -> Option<Range<usize>> {
        body_range(self.body.as_ref()?)
    }

    /// The end of `node.children.compact.last.source_range`: the body, else
    /// the written parameters. `None` where the parser's last child is an
    /// `args` node without a location (no parameters, no body).
    pub(crate) fn last_child_end(&self) -> Option<usize> {
        if let Some(body) = self.body_range() {
            return Some(body.end);
        }
        let parameters = self.parameters.as_ref()?.as_block_parameters_node()?;
        Some(parameters.location().end_offset())
    }
}

/// The line break to insert on the line holding `offset`: `\r\n` when that
/// line ends in one (as `Style/Semicolon` keeps a CRLF file's endings),
/// upstream's `\n` otherwise.
pub(crate) fn line_break(file: &crate::source::SourceFile, offset: usize) -> &'static str {
    let next = file.line_start(file.line_of(offset) + 1);
    if file.bytes[..next].ends_with(b"\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn range(node: &Node<'_>) -> Range<usize> {
    let location = node.location();
    location.start_offset()..location.end_offset()
}

/// The parser expression of a block body. An implicit `BeginNode` is the
/// parser's `rescue`/`ensure` node, whose map (`eh_keyword_map`) starts at
/// the statements (or the first clause's keyword) and ends at the last
/// clause's body (or keyword).
fn body_range(body: &Node<'_>) -> Option<Range<usize>> {
    let Some(begin) = body.as_begin_node() else {
        return Some(range(body));
    };
    if begin.begin_keyword_loc().is_some() {
        return Some(range(body));
    }
    let statements = begin.statements().map(|s| range(&s.as_node()));
    let rescue = begin.rescue_clause();
    let ensure = begin.ensure_clause();
    let start = statements
        .as_ref()
        .map(|s| s.start)
        .or_else(|| rescue.as_ref().map(|r| r.keyword_loc().start_offset()))
        .or_else(|| {
            ensure
                .as_ref()
                .map(|e| e.ensure_keyword_loc().start_offset())
        })?;
    let end = if let Some(ensure) = &ensure {
        ensure.statements().map_or_else(
            || ensure.ensure_keyword_loc().end_offset(),
            |s| s.location().end_offset(),
        )
    } else if let Some(other) = begin.else_clause() {
        other.statements().map_or_else(
            || other.else_keyword_loc().end_offset(),
            |s| s.location().end_offset(),
        )
    } else if let Some(mut clause) = rescue {
        while let Some(next) = clause.subsequent() {
            clause = next;
        }
        // `rescue_body_map`: the body, `then`, the variable, the exception
        // list, else the keyword.
        clause
            .statements()
            .map(|s| s.location().end_offset())
            .or_else(|| clause.then_keyword_loc().map(|t| t.end_offset()))
            .or_else(|| clause.reference().map(|r| r.location().end_offset()))
            .or_else(|| {
                clause
                    .exceptions()
                    .iter()
                    .last()
                    .map(|e| e.location().end_offset())
            })
            .unwrap_or_else(|| clause.keyword_loc().end_offset())
    } else {
        statements.as_ref()?.end
    };
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cops::{Cop, NodeContext};
    use crate::{offense::Offense, source::SourceFile};

    /// Reports each parser block's start, body and last child as text.
    struct Probe;
    impl Cop for Probe {
        fn name(&self) -> &'static str {
            "Layout/ParserBlockProbe"
        }
        fn check_node<'pr>(
            &self,
            node: &Node<'pr>,
            cx: &NodeContext<'_, 'pr>,
            file: &SourceFile,
            out: &mut Vec<Offense>,
        ) {
            let Some(block) = ParserBlock::of(node, cx) else {
                return;
            };
            let text = |r: Range<usize>| String::from_utf8_lossy(&file.bytes[r]).into_owned();
            out.push(Offense {
                correction: None,
                cop: self.name(),
                severity: self.severity(),
                message: format!(
                    "{}|{}|{}|{}",
                    text(block.start..block.opening.start),
                    block.body_range().map(text).unwrap_or_default(),
                    block
                        .last_child_end()
                        .map(|end| text(end..block.closing.end))
                        .unwrap_or_else(|| "-".into()),
                    block
                        .arguments()
                        .map(|a| text(range(&a)))
                        .unwrap_or_default(),
                ),
                line: 1,
                end_line: 1,
                col: 1,
            });
        }
    }

    fn probe(source: &str) -> Vec<String> {
        crate::pipeline::lint_str_with(&[&Probe], source)
            .into_iter()
            .map(|o| o.message)
            .collect()
    }

    #[test]
    fn blocks_start_at_their_call_and_lambdas_at_the_arrow() {
        assert_eq!(probe("a.b(1) { |x| x }"), ["a.b(1) |x| }||x|"]);
        assert_eq!(probe("x = -> (y) do y end"), ["-> (y) |y| end|(y)"]);
        assert_eq!(probe("super do _1 end"), ["super |_1| end|"]);
        assert_eq!(probe("foo { it }"), ["foo |it| }|"]);
        assert_eq!(probe("foo { || }"), ["foo || }|"]);
        assert_eq!(probe("foo { |;a| }"), ["foo || }||;a|"]);
        assert_eq!(probe("foo {\n}"), ["foo ||-|"]);
    }

    #[test]
    fn implicit_rescue_and_ensure_bodies_follow_the_parser_map() {
        for (source, body) in [
            (
                "foo do\n  a\nrescue E => e\n  b end",
                "a\nrescue E => e\n  b",
            ),
            ("foo do\n  a\nrescue E => e; end", "a\nrescue E => e"),
            ("foo do\n  a\nrescue E, F; end", "a\nrescue E, F"),
            ("foo do\n  a\nrescue; end", "a\nrescue"),
            ("foo do\n  a\nrescue then end", "a\nrescue then"),
            (
                "foo do\nrescue\n  b\nelse\n  c end",
                "rescue\n  b\nelse\n  c",
            ),
            ("foo do\n  a\nrescue\nelse end", "a\nrescue\nelse"),
            ("foo do\n  a\nensure\n  b end", "a\nensure\n  b"),
            ("foo do\nensure end", "ensure"),
            ("foo do\n  a\nrescue\nensure end", "a\nrescue\nensure"),
            (
                "foo do begin\n  a\nrescue\nend end",
                "begin\n  a\nrescue\nend",
            ),
        ] {
            let found = probe(source);
            assert_eq!(found.len(), 1, "{source}");
            let body_text = found[0].split('|').nth(1).unwrap();
            assert_eq!(body_text, body, "{source}");
        }
    }
}
