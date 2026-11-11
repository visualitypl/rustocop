//! Shared facts for the layout heredoc cops: the pinned
//! `Heredoc` mixin's indentation measure and the Prism translation's
//! classification of calls into parser-gem `send`/`csend` nodes.
use ruby_prism::Node;

/// Ruby's `/\s/`: space, `\t`, `\n`, `\v`, `\f`, `\r`.
pub(super) fn ruby_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

/// `Heredoc#indent_level`: the shortest leading `/\s*/` run over the lines of
/// `text`, ignoring runs that reach a newline (blank lines). Zero when every
/// line is blank.
pub(super) fn indent_level(text: &[u8]) -> usize {
    text.split_inclusive(|&b| b == b'\n')
        .filter_map(|line| {
            let run = line.iter().take_while(|&&b| ruby_space(b)).count();
            (run == 0 || line[run - 1] != b'\n').then_some(run)
        })
        .min()
        .unwrap_or(0)
}

/// The parser-gem type of a Prism call under the pinned translation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CallType {
    Send,
    Csend,
}

/// `index`, `indexasgn`, `match_with_lvasgn` and negative numeric literals are
/// translated from calls but are not `call_type?` nodes.
pub(super) fn call_type(node: &Node<'_>) -> Option<CallType> {
    let call = node.as_call_node()?;
    if call.call_operator_loc().is_none() {
        match call.name().as_slice() {
            b"[]" | b"[]=" => return None,
            b"=~"
                if call
                    .receiver()
                    .is_some_and(|r| r.as_regular_expression_node().is_some()) =>
            {
                return None;
            }
            b"-@"
                if call.receiver().is_some_and(|r| {
                    r.as_integer_node().is_some()
                        || r.as_float_node().is_some()
                        || r.as_rational_node().is_some()
                        || r.as_imaginary_node().is_some()
                }) =>
            {
                return None;
            }
            _ => {}
        }
    }
    Some(if call.is_safe_navigation() {
        CallType::Csend
    } else {
        CallType::Send
    })
}

/// How a node hangs from its parser-gem parent call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Role {
    Receiver,
    Argument,
}

fn same(a: &Node<'_>, b: &Node<'_>) -> bool {
    let (a, b) = (a.location(), b.location());
    a.start_offset() == b.start_offset() && a.end_offset() == b.end_offset()
}

/// The node's parser-gem parent when that is a `send`/`csend` holding it as
/// receiver or argument. `ancestors` are innermost first. A call with a literal
/// block is the child of a `block` node, never of another call.
pub(super) fn call_parent<'pr>(
    node: &Node<'pr>,
    ancestors: &[&Node<'pr>],
) -> Option<(usize, CallType, Role)> {
    let (index, role) = match ancestors.first()? {
        parent if parent.as_arguments_node().is_some() => (1, Role::Argument),
        parent
            if parent
                .as_call_node()
                .and_then(|c| c.receiver())
                .is_some_and(|r| same(&r, node)) =>
        {
            (0, Role::Receiver)
        }
        _ => return None,
    };
    let call = ancestors.get(index)?;
    let kind = call_type(call)?;
    if role == Role::Argument
        && !call
            .as_call_node()
            .and_then(|c| c.arguments())
            .is_some_and(|a| a.arguments().iter().any(|arg| same(&arg, node)))
    {
        return None;
    }
    Some((index, kind, role))
}

/// Whether a call's parser node is wrapped by a literal `block`.
pub(super) fn has_literal_block(node: &Node<'_>) -> bool {
    node.as_call_node()
        .and_then(|c| c.block())
        .is_some_and(|b| b.as_block_node().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indent_level_matches_the_mixin() {
        assert_eq!(indent_level(b"  a\n    b\n"), 2);
        assert_eq!(indent_level(b"  a\n\n \n"), 2);
        assert_eq!(indent_level(b"\n   \n"), 0);
        assert_eq!(indent_level(b"\t a\n  b\n"), 2);
        assert_eq!(indent_level(b"  a\r\n   \r\n"), 2);
        assert_eq!(indent_level(b"a"), 0);
        assert_eq!(indent_level(b"   "), 3);
        assert_eq!(indent_level(b""), 0);
    }
}
