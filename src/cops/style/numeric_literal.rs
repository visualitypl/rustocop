//! Pinned parser-adapter ranges shared by decimal grouping and base prefixes.
use crate::{
    cops::{NodeContext, ast},
    source::SourceFile,
};
use ruby_prism::{Location, Node};

/// Rational/imaginary payloads are not standalone int/float nodes upstream.
/// Fold exactly one directly enclosing prefix-minus, not explicit `.-@` sends.
pub(super) fn location<'pr>(
    node: &Node<'pr>,
    cx: &NodeContext<'_, 'pr>,
    floats: bool,
) -> Option<Location<'pr>> {
    if !(node.as_integer_node().is_some() || (floats && node.as_float_node().is_some()))
        || cx
            .parent()
            .is_some_and(|p| p.as_rational_node().is_some() || p.as_imaginary_node().is_some())
    {
        return None;
    }
    Some(
        cx.parent()
            .filter(|p| {
                ast::negated_numeric_literal(p)
                    .is_some_and(|n| n.location().start_offset() == node.location().start_offset())
            })
            .unwrap_or(node)
            .location(),
    )
}

/// RuboCop IntegerNode's source-based integer part (not a numeric conversion).
/// In particular, E/e also delimit hexadecimal spellings at the pinned version.
pub(super) fn integer_part<'s>(source: &'s [u8], file: &SourceFile) -> &'s [u8] {
    let source = if matches!(source.first(), Some(b'+' | b'-')) {
        &source[1..]
    } else {
        source
    };
    let mut end = 0;
    while end < source.len() {
        if matches!(source[end], b'e' | b'E' | b'.') {
            break;
        }
        // Folded signs can include encoded comments: an ASCII-looking trail
        // byte of a multibyte character is not an exponent delimiter.
        end += file.encoded_character_width(&source[end..]);
    }
    &source[..end]
}
