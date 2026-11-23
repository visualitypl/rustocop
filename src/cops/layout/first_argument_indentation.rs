//! `Layout/FirstArgumentIndentation` — fixed
//! `special_for_inner_method_call_in_parentheses`: a first argument after a
//! line break is indented one step (2) more than the previous code line, or
//! than the start of an inner call that is an argument of a parenthesized
//! call.
use crate::{
    cops::{Cop, NodeContext},
    offense::Offense,
    source::SourceFile,
};
use ruby_prism::Node;
pub struct FirstArgumentIndentation;
impl Cop for FirstArgumentIndentation {
    fn name(&self) -> &'static str {
        "Layout/FirstArgumentIndentation"
    }
    fn safe_fixes(&self) -> bool {
        true
    }
    fn wants_node(&self, node: &Node<'_>) -> bool {
        matches!(node, Node::CallNode { .. } | Node::SuperNode { .. })
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let Some(check) = evaluate(file, node, &ancestors) else {
            return;
        };
        let first = check.first.location();
        let range = first.start_offset()..first.end_offset();
        // Upstream `check_alignment`: an argument inside an earlier offense of
        // this cop (an enclosing call's first argument) is reported with the
        // generic message and left to a later pass.
        let nested = ancestors.iter().enumerate().any(|(i, ancestor)| {
            evaluate(file, ancestor, &ancestors[i + 1..]).is_some_and(|outer| {
                let loc = outer.first.location();
                loc.start_offset() <= range.start && range.end <= loc.end_offset()
            })
        });
        let message = if nested {
            "Bad indentation of the first argument.".to_owned()
        } else {
            format!(
                "Indent the first argument one step more than {}.",
                base_description(file, node, &check)
            )
        };
        let correction = (cx.corrections_requested() && !nested)
            .then(|| {
                let alignment = correction_target(file, node, &ancestors, &check);
                alignment.correction(file, cx.ancestors().next_back().unwrap_or(node))
            })
            .flatten();
        let (line, col) = file.line_col(range.start);
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message,
            line,
            col,
            end_line: file.last_line_of(first),
        });
    }
}
/// A reported first argument and its display-column shift.
struct Check<'pr> {
    first: Node<'pr>,
    delta: i64,
    special: bool,
}
/// Upstream `on_send`/`on_csend`/`on_super` at the fixed defaults
/// (`ArgumentAlignment: with_first_argument`, so no fixed-indentation gates).
/// `ancestors` are the node's Prism ancestors, innermost first.
fn evaluate<'pr>(
    file: &SourceFile,
    node: &Node<'pr>,
    ancestors: &[&Node<'pr>],
) -> Option<Check<'pr>> {
    let first = first_argument(node)?;
    let start = first.location().start_offset();
    if file.line_of(node.location().start_offset()) == file.line_of(start) {
        return None;
    }
    let special = special(node, ancestors);
    let base = if special {
        let text = strip(&file.bytes[node.location().start_offset()..start]);
        let newlines = text.iter().filter(|b| **b == b'\n').count() as u32;
        if newlines == 0 {
            super::alignment::display_column(file, node.location().start_offset())
        } else {
            previous_code_line(
                file,
                file.line_of(node.location().start_offset()) + newlines + 1,
            )?
        }
    } else {
        previous_code_line(file, file.line_of(start))?
    };
    if !super::alignment::starts_line(file, start) {
        return None;
    }
    let delta = (base + super::alignment::INDENT) as i64
        - super::alignment::display_column(file, start) as i64;
    (delta != 0).then_some(Check {
        first,
        delta,
        special,
    })
}
/// `node.first_argument` of a checked `send`/`csend`/`super` with
/// arguments: not a bare operator (`a + b`, `a&.+(b)`) or a setter. Prism keeps
/// a block pass outside the argument list; the parser does not.
fn first_argument<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let (arguments, block) = match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node()?;
            let dot = call
                .call_operator_loc()
                .is_some_and(|l| l.as_slice() == b".");
            if (crate::cops::ast::is_operator_method(call.name().as_slice()) && !dot)
                || call.is_attribute_write()
            {
                return None;
            }
            (call.arguments(), call.block())
        }
        Node::SuperNode { .. } => {
            let call = node.as_super_node()?;
            (call.arguments(), call.block())
        }
        _ => return None,
    };
    arguments
        .and_then(|a| a.arguments().iter().next())
        .or_else(|| block.filter(|b| b.as_block_argument_node().is_some()))
}
/// A literal block makes the parser's `block` node the call's parent.
fn has_literal_block(node: &Node<'_>) -> bool {
    match node {
        Node::CallNode { .. } => node
            .as_call_node()
            .and_then(|c| c.block())
            .is_some_and(|b| b.as_block_node().is_some()),
        Node::SuperNode { .. } => node
            .as_super_node()
            .and_then(|c| c.block())
            .is_some_and(|b| b.as_block_node().is_some()),
        _ => false,
    }
}
/// A parenthesized parser `send` (not `csend`, not `[]=`).
fn parenthesized_send(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|c| {
        !c.is_safe_navigation()
            && c.name().as_slice() != b"[]="
            && c.opening_loc().is_some_and(|l| l.as_slice() == b"(")
    })
}
/// `special_inner_call_indentation?`: the call is an argument of a
/// parenthesized `send` (its parser parent; a literal block, a block pass, a
/// splat, a pair or the receiver position are other parents).
fn special(node: &Node<'_>, ancestors: &[&Node<'_>]) -> bool {
    !has_literal_block(node)
        && ancestors
            .first()
            .is_some_and(|p| p.as_arguments_node().is_some())
        && ancestors.get(1).is_some_and(|p| {
            parenthesized_send(p) && node.location().start_offset() > p.location().start_offset()
        })
}
/// Ruby's `String#strip` (whitespace and NUL) of a byte range.
fn strip(text: &[u8]) -> &[u8] {
    let blank = |b: &u8| super::element_indentation::ruby_space(*b) || *b == 0;
    let start = text.iter().position(|b| !blank(b)).unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|b| !blank(b))
        .map_or(start, |i| i + 1);
    &text[start..end]
}
/// `previous_code_line(n) =~ /\S/`: the first non-blank line before `n` that
/// does not begin with a comment, as the index of its first non-space.
fn previous_code_line(file: &SourceFile, mut number: u32) -> Option<usize> {
    let comments = file.comments();
    let begins_with_comment = |n: u32| {
        let start = file.line_start(n);
        let end = file.line_start(n + 1);
        let i = comments.partition_point(|c| c.start < start);
        comments
            .get(i)
            .is_some_and(|c| c.start < end && super::alignment::starts_line(file, c.start))
    };
    let mut text: &[u8] = b"";
    while super::element_indentation::blank(text) || begins_with_comment(number) {
        number = number.checked_sub(1).filter(|n| *n > 0)?;
        text = super::element_indentation::line(file, number);
    }
    Some(super::element_indentation::line_indentation(file, number))
}
/// The message's base: the inner call's source up to its first argument, or
/// the previous line (noting a comment line in between).
fn base_description(file: &SourceFile, node: &Node<'_>, check: &Check<'_>) -> String {
    let text =
        strip(&file.bytes[node.location().start_offset()..check.first.location().start_offset()]);
    if !text.contains(&b'\n') && check.special {
        return format!("`{}`", String::from_utf8_lossy(text));
    }
    let last = text.rsplit(|b| *b == b'\n').next().unwrap_or_default();
    if strip(last).starts_with(b"#") {
        "the start of the previous line (not counting the comment)".into()
    } else {
        "the start of the previous line".into()
    }
}
/// Upstream `autocorrect`: shift the argument, or the whole top-level call
/// chain of an inner call when the argument sits further right than the call
/// starts and moving it alone would leave its closing line behind.
fn correction_target(
    file: &SourceFile,
    node: &Node<'_>,
    ancestors: &[&Node<'_>],
    check: &Check<'_>,
) -> super::alignment::Alignment {
    let first = check.first.location();
    let argument = super::alignment::Alignment {
        expression: first.start_offset()..first.end_offset(),
        delta: check.delta,
    };
    if check.delta >= 0
        || super::alignment::display_column(file, node.location().start_offset())
            >= check.delta.unsigned_abs() as usize
    {
        return argument;
    }
    // `find_top_level_send`: climb receivers of dotted parser sends.
    let mut top: &Node<'_> = node;
    let mut rest = ancestors;
    while !has_literal_block(top)
        && let Some((parent, tail)) = rest.split_first()
        && let Some(call) = parent.as_call_node()
        && !call.is_safe_navigation()
        && call.call_operator_loc().is_some()
        && call.receiver().is_some_and(|r| same(&r, top))
    {
        top = parent;
        rest = tail;
    }
    // `inner_call?`: the top call is an argument of a parenthesized send.
    let inner = !has_literal_block(top)
        && rest
            .first()
            .is_some_and(|p| p.as_arguments_node().is_some())
        && rest.get(1).is_some_and(|p| parenthesized_send(p));
    let closing = match top {
        Node::CallNode { .. } => top.as_call_node().and_then(|c| c.closing_loc()),
        Node::SuperNode { .. } => top.as_super_node().and_then(|c| c.rparen_loc()),
        _ => None,
    };
    let chain = inner
        && (!same(top, node)
            || closing.is_some_and(|l| {
                l.as_slice() == b")" && super::alignment::starts_line(file, l.start_offset())
            }));
    if !chain {
        return argument;
    }
    let loc = top.location();
    let mut start = loc.start_offset();
    let prefix = start - file.line_start(file.line_of(start));
    if !super::alignment::starts_line(file, start) || (prefix as i64) < -check.delta {
        // `AlignmentCorrector` removes the shift before the chain's first
        // token only when that many bytes there are blanks; otherwise its
        // first line stays. A mid-line chain's blanks are left too (a
        // conservative subset); only its later lines move.
        let line = file.line_start(file.line_of(start) + 1);
        start = line
            + file.bytes[line..]
                .iter()
                .take_while(|b| matches!(b, b' ' | b'\t' | b'\x0b'))
                .count();
    }
    super::alignment::Alignment {
        expression: start..loc.end_offset(),
        delta: check.delta,
    }
}
fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    let (a, b) = (a.location(), b.location());
    (a.start_offset(), a.end_offset()) == (b.start_offset(), b.end_offset())
}
#[cfg(test)]
mod tests;
