//! `Layout/HeredocArgumentClosingParenthesis` (opt-in) — the closing
//! parenthesis of a call passing a heredoc argument goes on the heredoc's
//! opening line.
use ruby_prism::{Location, Node};

use super::heredoc::{call_type, has_literal_block};
use crate::{
    cops::{
        Cop, NodeContext,
        naming::heredoc::{Heredoc, strip},
        walk::{NodeVisitor, walk},
    },
    offense::{Correction, Offense},
    source::SourceFile,
};

pub struct HeredocArgumentClosingParenthesis;

const MSG: &str = "Put the closing parenthesis for a method call with a HEREDOC parameter \
                   on the same line as the HEREDOC opening.";

impl Cop for HeredocArgumentClosingParenthesis {
    fn name(&self) -> &'static str {
        "Layout/HeredocArgumentClosingParenthesis"
    }

    fn safe_fixes(&self) -> bool {
        true
    }

    fn autocorrect_incompatible_with(&self) -> &'static [&'static str] {
        &["Style/TrailingCommaInArguments"]
    }

    fn check_node<'pr>(
        &self,
        node: &Node<'pr>,
        cx: &NodeContext<'_, 'pr>,
        file: &SourceFile,
        out: &mut Vec<Offense>,
    ) {
        // `on_send`/`on_csend`.
        if call_type(node).is_none() {
            return;
        }
        let call = node.as_call_node().unwrap();
        let args = parser_arguments(node);
        // `extract_heredoc_argument`. Hash pairs and a single-line receiver
        // chain never stop the climb, so it starts at this call.
        let Some(heredoc) = args.iter().find_map(|arg| extract_heredoc(arg, file)) else {
            return;
        };
        let opening_line = file.line_of(heredoc.opening.start_offset());
        let ancestors: Vec<&Node<'pr>> = cx.ancestors().collect();
        let Some(outermost) = outermost_send_on_same_line(node, &ancestors, opening_line, file)
        else {
            return;
        };
        let outer = outermost.as_call_node().unwrap();
        let Some(close) = outer.closing_loc() else {
            return;
        };
        if end_keyword_before_closing_parenthesis(node, &ancestors)
            || subsequent_closing_parentheses_in_same_line(outermost, &close, file)
            || argument_between_heredoc_end_and_closing_parenthesis(&call, &args, file)
        {
            return;
        }
        let correction = if cx.corrections_requested() {
            correction(outermost, &close, file)
        } else {
            None
        };
        let (line, col) = file.line_col(close.start_offset());
        out.push(Offense {
            correction,
            cop: self.name(),
            severity: self.severity(),
            message: MSG.into(),
            line,
            end_line: line,
            col,
        });
    }
}

/// The parser-gem arguments of a call: its arguments and a block argument.
fn parser_arguments<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    let call = node.as_call_node().unwrap();
    let mut args: Vec<Node<'pr>> = call
        .arguments()
        .map(|a| a.arguments().iter().collect())
        .unwrap_or_default();
    if let Some(block) = call
        .block()
        .filter(|b| b.as_block_argument_node().is_some())
    {
        args.push(block);
    }
    args
}

/// `extract_heredoc`: a heredoc argument, the heredoc receiver of a
/// single-line `send`, or (recursively) a heredoc hash value.
fn extract_heredoc<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Heredoc<'pr>> {
    if let Some(heredoc) = Heredoc::of(node, file) {
        return Some(heredoc);
    }
    if let Some(call) = node.as_call_node()
        && call_type(node) == Some(super::heredoc::CallType::Send)
        && !has_literal_block(node)
        && let Some(receiver) = call.receiver()
        && let Some(heredoc) = Heredoc::of(&receiver, file)
        && closing_text_end(&heredoc, file) > node.location().end_offset()
    {
        return Some(heredoc);
    }
    let elements = match node {
        Node::KeywordHashNode { .. } => node.as_keyword_hash_node()?.elements(),
        Node::HashNode { .. } => node.as_hash_node()?.elements(),
        _ => return None,
    };
    elements
        .iter()
        .filter_map(|element| element.as_assoc_node())
        .find_map(|pair| extract_heredoc(&pair.value(), file))
}

/// `heredoc_end.end_pos`: the closing line without its line break.
fn closing_text_end(heredoc: &Heredoc<'_>, file: &SourceFile) -> usize {
    let closing = &heredoc.closing;
    let text = &file.bytes[closing.start_offset()..closing.end_offset()];
    let trailing = text
        .iter()
        .rev()
        .take_while(|&&b| super::heredoc::ruby_space(b))
        .count();
    closing.end_offset() - trailing
}

/// `outermost_send_on_same_line`: climb from the call holding the heredoc
/// argument to the first parenthesized `send`/`csend` holding the path as an
/// argument whose `)` is not on the heredoc's opening line. Leaving a block or
/// lambda body gives up.
fn outermost_send_on_same_line<'a, 'pr>(
    node: &'a Node<'pr>,
    ancestors: &[&'a Node<'pr>],
    opening_line: u32,
    file: &SourceFile,
) -> Option<&'a Node<'pr>> {
    let missing = |call: &ruby_prism::CallNode<'_>| {
        call.opening_loc().is_some_and(|l| l.as_slice() == b"(")
            && call
                .closing_loc()
                .is_some_and(|l| file.line_of(l.start_offset()) != opening_line)
    };
    if missing(&node.as_call_node()?) {
        return Some(node);
    }
    let mut previous = node;
    for &current in ancestors {
        if current.as_block_node().is_some() || current.as_lambda_node().is_some() {
            return None;
        }
        if let Some(call) = current.as_call_node()
            && call_type(current).is_some()
        {
            let argument = previous.as_arguments_node().is_some()
                || previous.as_block_argument_node().is_some();
            if argument && missing(&call) {
                return Some(current);
            }
        }
        previous = current;
    }
    None
}

/// `loc_is?(:end, 'end')` for the parser-gem ancestors of the call: Prism
/// ancestors, plus literal `do ... end` blocks of the call and of enclosing
/// calls (a parser `block` wraps its call).
fn end_keyword_before_closing_parenthesis(node: &Node<'_>, ancestors: &[&Node<'_>]) -> bool {
    std::iter::once(node)
        .chain(ancestors.iter().copied())
        .any(|n| has_end_keyword(n) || block_of(n).is_some_and(|b| has_end_keyword(&b)))
}

fn block_of<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    let block = match node {
        Node::CallNode { .. } => node.as_call_node()?.block(),
        Node::SuperNode { .. } => node.as_super_node()?.block(),
        Node::ForwardingSuperNode { .. } => node
            .as_forwarding_super_node()?
            .block()
            .map(|b| b.as_node()),
        _ => None,
    }?;
    block.as_block_node().is_some().then_some(block)
}

fn is_end(loc: Option<Location<'_>>) -> bool {
    loc.is_some_and(|l| l.as_slice() == b"end")
}

fn has_end_keyword(node: &Node<'_>) -> bool {
    match node {
        Node::IfNode { .. } => is_end(node.as_if_node().unwrap().end_keyword_loc()),
        Node::UnlessNode { .. } => is_end(node.as_unless_node().unwrap().end_keyword_loc()),
        Node::WhileNode { .. } => is_end(node.as_while_node().unwrap().closing_loc()),
        Node::UntilNode { .. } => is_end(node.as_until_node().unwrap().closing_loc()),
        Node::CaseNode { .. } => is_end(Some(node.as_case_node().unwrap().end_keyword_loc())),
        Node::CaseMatchNode { .. } => {
            is_end(Some(node.as_case_match_node().unwrap().end_keyword_loc()))
        }
        Node::ForNode { .. } => is_end(Some(node.as_for_node().unwrap().end_keyword_loc())),
        Node::DefNode { .. } => is_end(node.as_def_node().unwrap().end_keyword_loc()),
        Node::ClassNode { .. } => is_end(Some(node.as_class_node().unwrap().end_keyword_loc())),
        Node::ModuleNode { .. } => is_end(Some(node.as_module_node().unwrap().end_keyword_loc())),
        Node::SingletonClassNode { .. } => is_end(Some(
            node.as_singleton_class_node().unwrap().end_keyword_loc(),
        )),
        Node::BeginNode { .. } => is_end(node.as_begin_node().unwrap().end_keyword_loc()),
        Node::BlockNode { .. } => is_end(Some(node.as_block_node().unwrap().closing_loc())),
        Node::LambdaNode { .. } => is_end(Some(node.as_lambda_node().unwrap().closing_loc())),
        _ => false,
    }
}

/// The parser-gem `loc.end` of an argument, where it has one.
fn end_loc<'pr>(node: &Node<'pr>, file: &SourceFile) -> Option<Location<'pr>> {
    if Heredoc::of(node, file).is_some() {
        return None;
    }
    if let Some(block) = block_of(node) {
        return Some(block.as_block_node()?.closing_loc());
    }
    match node {
        Node::CallNode { .. } => {
            let call = node.as_call_node()?;
            if let Some(block) = call.block().and_then(|b| b.as_block_node()) {
                return Some(block.closing_loc());
            }
            call.closing_loc()
        }
        Node::ArrayNode { .. } => node.as_array_node()?.closing_loc(),
        Node::HashNode { .. } => Some(node.as_hash_node()?.closing_loc()),
        Node::StringNode { .. } => node.as_string_node()?.closing_loc(),
        Node::InterpolatedStringNode { .. } => node.as_interpolated_string_node()?.closing_loc(),
        Node::XStringNode { .. } => Some(node.as_x_string_node()?.closing_loc()),
        Node::InterpolatedXStringNode { .. } => {
            Some(node.as_interpolated_x_string_node()?.closing_loc())
        }
        Node::SymbolNode { .. } => node.as_symbol_node()?.closing_loc(),
        Node::InterpolatedSymbolNode { .. } => node.as_interpolated_symbol_node()?.closing_loc(),
        Node::RegularExpressionNode { .. } => {
            Some(node.as_regular_expression_node()?.closing_loc())
        }
        Node::InterpolatedRegularExpressionNode { .. } => Some(
            node.as_interpolated_regular_expression_node()?
                .closing_loc(),
        ),
        Node::ParenthesesNode { .. } => Some(node.as_parentheses_node()?.closing_loc()),
        Node::LambdaNode { .. } => Some(node.as_lambda_node()?.closing_loc()),
        Node::SuperNode { .. } => node.as_super_node()?.rparen_loc(),
        Node::YieldNode { .. } => node.as_yield_node()?.rparen_loc(),
        Node::DefinedNode { .. } => node.as_defined_node()?.rparen_loc(),
        _ => end_keyword(node).filter(|l| l.as_slice() == b"end"),
    }
}

fn end_keyword<'pr>(node: &Node<'pr>) -> Option<Location<'pr>> {
    match node {
        Node::IfNode { .. } => node.as_if_node()?.end_keyword_loc(),
        Node::UnlessNode { .. } => node.as_unless_node()?.end_keyword_loc(),
        Node::WhileNode { .. } => node.as_while_node()?.closing_loc(),
        Node::UntilNode { .. } => node.as_until_node()?.closing_loc(),
        Node::CaseNode { .. } => Some(node.as_case_node()?.end_keyword_loc()),
        Node::CaseMatchNode { .. } => Some(node.as_case_match_node()?.end_keyword_loc()),
        Node::ForNode { .. } => Some(node.as_for_node()?.end_keyword_loc()),
        Node::DefNode { .. } => node.as_def_node()?.end_keyword_loc(),
        Node::ClassNode { .. } => Some(node.as_class_node()?.end_keyword_loc()),
        Node::ModuleNode { .. } => Some(node.as_module_node()?.end_keyword_loc()),
        Node::SingletonClassNode { .. } => Some(node.as_singleton_class_node()?.end_keyword_loc()),
        Node::BeginNode { .. } => node.as_begin_node()?.end_keyword_loc(),
        _ => None,
    }
}

/// `subsequent_closing_parentheses_in_same_line?`: the outermost `)` directly
/// follows its last argument's closing delimiter on the same line.
fn subsequent_closing_parentheses_in_same_line(
    outermost: &Node<'_>,
    close: &Location<'_>,
    file: &SourceFile,
) -> bool {
    let Some(last) = parser_arguments(outermost).pop() else {
        return false;
    };
    let Some(end) = end_loc(&last, file) else {
        return false;
    };
    let (end_line, end_col) = file.line_col(end.start_offset());
    let (line, col) = file.line_col(close.start_offset());
    line == end_line && col == end_col + 1
}

/// `exist_argument_between_heredoc_end_and_closing_parentheses?` (true also
/// for an unparenthesized call).
fn argument_between_heredoc_end_and_closing_parenthesis(
    call: &ruby_prism::CallNode<'_>,
    args: &[Node<'_>],
    file: &SourceFile,
) -> bool {
    let Some(close) = call.closing_loc().filter(|_| call.opening_loc().is_some()) else {
        return true;
    };
    struct Ends<'f> {
        file: &'f SourceFile,
        max: Option<usize>,
    }
    impl<'pr> NodeVisitor<'pr> for Ends<'_> {
        fn enter(&mut self, node: Node<'pr>) {
            if let Some(h) = Heredoc::of(&node, self.file) {
                let end = closing_text_end(&h, self.file);
                self.max = Some(self.max.map_or(end, |m| m.max(end)));
            }
        }
    }
    let mut ends = Ends { file, max: None };
    for arg in args {
        walk(&mut ends, arg);
    }
    let Some(end) = ends.max else {
        return false;
    };
    end < close.start_offset() && !strip(&file.bytes[end..close.start_offset()]).is_empty()
}

/// Upstream's `autocorrect`: move `)` (and an external trailing comma) right
/// after the last argument, drop an internal trailing comma, and remove the
/// old `)` with its line when that line holds nothing else. One atomic edit.
fn correction(outermost: &Node<'_>, close: &Location<'_>, file: &SourceFile) -> Option<Correction> {
    let bytes = &file.bytes;
    let last = parser_arguments(outermost).pop()?;
    let last_end = last.location().end_offset();
    let end_pos = close.end_offset();
    let close_line = file.line_of(close.start_offset());
    let line_start = file.line_start(close_line);
    let line_text = &bytes[line_start..line_start + file.line_text(close_line).len()];
    // `/^ *\) {0,20},{0,1} *$/`
    let safe_line = {
        let rest = &line_text[line_text.iter().take_while(|&&b| b == b' ').count()..];
        rest.first() == Some(&b')') && {
            let rest = &rest[1..];
            let spaces = rest.iter().take_while(|&&b| b == b' ').count();
            let rest = &rest[spaces..];
            let rest = rest.strip_prefix(b",").unwrap_or(rest);
            spaces <= 20 && rest.iter().all(|&b| b == b' ')
        }
    };
    let mut remove_start = if safe_line && line_start > 0 {
        line_start - 1
    } else {
        close.start_offset()
    };
    if safe_line && remove_start > 0 && bytes[remove_start - 1] == b'\r' {
        remove_start -= 1;
    }
    let mut remove_end = end_pos + usize::from(bytes.get(end_pos) == Some(&b','));
    // External trailing comma: up to 20 spaces after `)`, then a comma.
    let spaces = bytes[end_pos..]
        .iter()
        .take(20)
        .take_while(|&&b| b == b' ')
        .count();
    let external = bytes.get(end_pos + spaces) == Some(&b',');
    if external {
        remove_end = remove_end.max(end_pos + spaces + 1);
    }
    // Internal trailing comma: a comma before the first line break after the
    // last argument. Only a comma after blanks is punctuation; upstream's
    // first comma could sit in a comment, and deleting through it would turn
    // commented-out code live.
    let between = &bytes[last_end..close.start_offset()];
    let blanks = between
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t'))
        .count();
    let internal = if between.get(blanks) == Some(&b',') {
        blanks + 1
    } else {
        0
    };
    if last_end + internal > remove_start {
        return None;
    }
    let mut replacement = Vec::new();
    replacement.push(b')');
    if external {
        replacement.push(b',');
    }
    replacement.extend_from_slice(&bytes[last_end + internal..remove_start]);
    Some(Correction {
        range: last_end..remove_end,
        replacement: String::from_utf8(replacement).ok()?,
    })
}

#[cfg(test)]
mod tests;
