//! `Layout/EmptyLinesAroundExceptionHandlingKeywords` — no blank line right
//! before or after `rescue`, `else` and `ensure`.
//!
//! Upstream's `on_def`/`on_defs`/`on_block`/`on_numblock`/`on_kwbegin` (no
//! `on_itblock`) at the fixed `no_empty_lines` style, with
//! `Layout/EmptyLinesAroundBlockBody` at its fixed `no_empty_lines`. The
//! parser body is a `rescue`/`ensure` node (Prism: an implicit or `begin`
//! BeginNode with clauses), or a rescue modifier that is the sole statement
//! of a `def`/block or the first statement of a `begin`.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::{BeginNode, Location, Node};

pub struct EmptyLinesAroundExceptionHandlingKeywords;

impl Cop for EmptyLinesAroundExceptionHandlingKeywords {
    fn name(&self) -> &'static str {
        "Layout/EmptyLinesAroundExceptionHandlingKeywords"
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
        let Some((owner_start, body, end)) = owner(node, cx) else {
            return;
        };
        let Some(body) = body else {
            return;
        };
        let keywords = body.keywords();
        if keywords.is_empty() {
            return;
        }
        let owner_line = file.line_of(owner_start);
        let end_line = end.map(|end| file.line_of(end.start_offset()));
        let mut lines: Vec<u32> = Vec::new();
        let start = out.len();
        for keyword in keywords {
            let line = file.line_of(keyword.start_offset());
            if line == owner_line {
                continue;
            }
            // Upstream reads `body.parent.loc.end`, which a (multi-line)
            // endless definition lacks: it raises and reports nothing.
            let Some(end_line) = end_line else {
                break;
            };
            if body.last_line(file) == end_line {
                continue;
            }
            let word = std::str::from_utf8(keyword.as_slice()).unwrap_or("rescue");
            for (blank, side) in [(line + 1, "after"), (line - 1, "before")] {
                // Upstream keeps one offense per range: the first message.
                if !file.line_text(blank).is_empty() || lines.contains(&blank) {
                    continue;
                }
                lines.push(blank);
                out.push(Offense {
                    correction: super::body_padding::line_removal(cx, file, blank),
                    cop: self.name(),
                    severity: self.severity(),
                    message: format!("Extra empty line detected {side} the `{word}`."),
                    line: blank,
                    end_line: blank,
                    col: 1,
                });
            }
        }
        super::body_padding::hold_for_listed_observers(cx, file, &mut out[start..]);
    }
}

/// The parser `rescue`/`ensure` body of a `def`, block or `begin`.
enum Body<'pr> {
    Clauses(BeginNode<'pr>),
    Modifier(ruby_prism::RescueModifierNode<'pr>),
}

/// `(owner start offset, body, end keyword)`: the owner's parser line is the
/// `def`/`begin` keyword, a lambda's `->`, or a block's call (the parser's
/// block node encloses its send).
fn owner<'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
) -> Option<(usize, Option<Body<'pr>>, Option<Location<'pr>>)> {
    if let Some(def) = node.as_def_node() {
        return Some((
            node.location().start_offset(),
            def.body().and_then(|b| sole_body(&b)),
            def.end_keyword_loc(),
        ));
    }
    if let Some(block) = node.as_block_node() {
        if block
            .parameters()
            .is_some_and(|p| p.as_it_parameters_node().is_some())
        {
            return None;
        }
        let call = cx.parent()?;
        return Some((
            call.location().start_offset(),
            block.body().and_then(|b| sole_body(&b)),
            Some(block.closing_loc()),
        ));
    }
    if let Some(lambda) = node.as_lambda_node() {
        if lambda
            .parameters()
            .is_some_and(|p| p.as_it_parameters_node().is_some())
        {
            return None;
        }
        return Some((
            node.location().start_offset(),
            lambda.body().and_then(|b| sole_body(&b)),
            Some(lambda.closing_loc()),
        ));
    }
    let begin = node.as_begin_node()?;
    let keyword = begin.begin_keyword_loc()?;
    let end = begin.end_keyword_loc();
    let body = if begin.rescue_clause().is_some() || begin.ensure_clause().is_some() {
        Some(Body::Clauses(begin))
    } else {
        // `kwbegin`'s first child.
        begin
            .statements()
            .and_then(|s| s.body().iter().next())
            .and_then(|first| first.as_rescue_modifier_node())
            .map(Body::Modifier)
    };
    Some((keyword.start_offset(), body, end))
}

/// A `def`/block body: an implicit BeginNode with clauses, or a sole rescue
/// modifier statement.
fn sole_body<'pr>(body: &Node<'pr>) -> Option<Body<'pr>> {
    if let Some(begin) = body.as_begin_node() {
        return (begin.rescue_clause().is_some() || begin.ensure_clause().is_some())
            .then_some(Body::Clauses(begin));
    }
    let statements = body.as_statements_node()?;
    let mut iter = statements.body().iter();
    let first = iter.next()?;
    if iter.next().is_some() {
        return None;
    }
    first.as_rescue_modifier_node().map(Body::Modifier)
}

impl<'pr> Body<'pr> {
    /// Upstream's `keyword_locations`: `ensure`, then the rescue part's
    /// `else` and each `rescue`.
    fn keywords(&self) -> Vec<Location<'pr>> {
        match self {
            Body::Modifier(modifier) => vec![modifier.keyword_loc()],
            Body::Clauses(begin) => {
                let mut keywords = Vec::new();
                if let Some(ensure) = begin.ensure_clause() {
                    keywords.push(ensure.ensure_keyword_loc());
                }
                if let Some(rescue) = begin.rescue_clause() {
                    if let Some(clause) = begin.else_clause() {
                        keywords.push(clause.else_keyword_loc());
                    }
                    let mut next = Some(rescue);
                    while let Some(clause) = next {
                        keywords.push(clause.keyword_loc());
                        next = clause.subsequent();
                    }
                }
                keywords
            }
        }
    }

    /// Upstream's last body line compared with the owner's `end` line: an
    /// `ensure` node's last line, or a `rescue` node's `else` keyword line,
    /// else its last `rescue` keyword line.
    fn last_line(&self, file: &SourceFile) -> u32 {
        match self {
            Body::Modifier(modifier) => file.line_of(modifier.keyword_loc().start_offset()),
            Body::Clauses(begin) => {
                if let Some(ensure) = begin.ensure_clause() {
                    return match ensure.statements() {
                        Some(statements) => file.last_line_of(statements.location()),
                        None => file.line_of(ensure.ensure_keyword_loc().start_offset()),
                    };
                }
                if let Some(clause) = begin.else_clause() {
                    return file.line_of(clause.else_keyword_loc().start_offset());
                }
                let mut last = begin.rescue_clause();
                while let Some(next) = last.as_ref().and_then(|c| c.subsequent()) {
                    last = Some(next);
                }
                last.map_or(0, |clause| {
                    file.line_of(clause.keyword_loc().start_offset())
                })
            }
        }
    }
}

#[cfg(test)]
mod tests;
