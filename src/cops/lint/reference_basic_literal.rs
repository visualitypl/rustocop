//! Pinned main-parse str/sym shape for the three basic-literal consumers.
//! Prism's static value is not enough: the adapter can emit zero/multiple str
//! children. This mirrors its child emission, never parses or decodes candidates.
use ruby_prism::{Node, StringNode};

use crate::{cops::ast, source::SourceFile};

pub(crate) fn is_basic_literal(node: &Node<'_>, file: &SourceFile) -> bool {
    if let Some(string) = node.as_string_node() {
        let opening = string.opening_loc();
        let opening = opening.as_ref().map_or(b"".as_slice(), |l| l.as_slice());
        // Delimiter-less word-array elements are emitted as str children;
        // character literals bypass composition entirely.
        return opening.is_empty() || opening == b"?" || string_children(node, file) == 1;
    }
    if node.as_interpolated_string_node().is_some() {
        return string_children(node, file) == 1;
    }
    if let Some(symbol) = node.as_symbol_node() {
        let Some(closing) = symbol.closing_loc() else {
            return true; // bare symbols and percent-symbol-array elements
        };
        if closing.as_slice().ends_with(b":") {
            return true; // pair_quoted bypasses ordinary symbol composition
        }
        let raw = symbol.value_loc();
        let raw = raw.as_ref().map_or(symbol.unescaped(), |l| l.as_slice());
        if raw.is_empty() {
            return false;
        }
        let opening = symbol.opening_loc();
        let opening = opening.as_ref().map_or(b"".as_slice(), |l| l.as_slice());
        return emitted_lines(raw, symbol.unescaped(), opening, file).len() == 1;
    }
    ast::is_basic_literal(node)
}

pub(super) fn is_dstr(node: &Node<'_>, file: &SourceFile) -> bool {
    matches!(
        node,
        Node::StringNode { .. } | Node::InterpolatedStringNode { .. }
    ) && !is_basic_literal(node, file)
}

fn string_children(node: &Node<'_>, file: &SourceFile) -> usize {
    let (opening, strings) = if let Some(string) = node.as_string_node() {
        (string.opening_loc(), vec![string])
    } else if let Some(string) = node.as_interpolated_string_node() {
        let opening = string.opening_loc();
        // Ordinary interpolation/concatenation is dstr. Static continued
        // squiggly heredocs alone can fold to one str in the adapter.
        if !opening
            .as_ref()
            .is_some_and(|l| l.as_slice().starts_with(b"<<"))
        {
            return 0;
        }
        let Some(strings) = string
            .parts()
            .iter()
            .map(|p| p.as_string_node())
            .collect::<Option<Vec<_>>>()
        else {
            return 0;
        };
        (opening, strings)
    } else {
        return 0;
    };
    let opening = opening.as_ref().map_or(b"".as_slice(), |l| l.as_slice());
    let heredoc = opening.starts_with(b"<<");
    let mut count = 0;
    let mut previous_newline = true;
    for string in strings {
        if !heredoc && opening.starts_with(b"%") && string.unescaped().is_empty() {
            continue; // visit_string_node's explicit empty-percent branch
        }
        for newline in string_lines(&string, opening, file) {
            if !heredoc || previous_newline {
                count += 1;
            }
            previous_newline = newline;
        }
    }
    count
}

fn string_lines(string: &StringNode<'_>, opening: &[u8], file: &SourceFile) -> Vec<bool> {
    let raw = string.content_loc();
    let raw = raw.as_slice();
    if opening.starts_with(b"<<") && string.unescaped().is_empty() {
        return Vec::new(); // visit_heredoc drops empty str children
    }
    emitted_lines(raw, string.unescaped(), opening, file)
}

/// Each emitted child's trailing-newline flag. The adapter iterates *decoded
/// lines*, emitting only at indices calculated from escaped physical lines.
/// For example `"a\\n\\\n"` has one decoded line but emission index 1, hence
/// emits NO children, despite its nonempty value. Payload alignment alone cannot
/// express this. Keep the existing source/escape-byte projection for encodings.
fn emitted_lines(raw: &[u8], value: &[u8], opening: &[u8], file: &SourceFile) -> Vec<bool> {
    if !raw.contains(&b'\n') {
        return vec![value.ends_with(b"\n")];
    }
    let single =
        opening.ends_with(b"'") || opening.starts_with(b"%q") || opening.starts_with(b"%s");
    let value = ast::require_literal::parts(raw, value, single, file).concat();
    let decoded: Vec<_> = value.split_inclusive(|&b| b == b'\n').collect();
    let escaped = file.match_pattern_source(raw);
    if single {
        // The non-interpolating branch emits once per physical source line,
        // even if that decoded line is empty.
        return escaped
            .split_inclusive(|&b| b == b'\n')
            .enumerate()
            .map(|(i, _)| decoded.get(i).is_some_and(|l| l.ends_with(b"\n")))
            .collect();
    }
    let mut emitted = Vec::new();
    let mut index = 0;
    let mut escaped_newlines = 0;
    let mut lines = escaped.split_inclusive(|&b| b == b'\n').peekable();
    while let Some(line) = lines.next() {
        // Match the adapter's /(\\*)n/ scan, not decoded newline counts:
        // Unicode/hex/octal escapes do not participate in this bookkeeping.
        let mut slashes = 0;
        let mut count = 0;
        for &byte in line {
            if byte == b'\\' {
                slashes += 1;
            } else {
                if byte == b'n' && slashes % 2 == 1 {
                    count += 1;
                }
                slashes = 0;
            }
        }
        if !line.ends_with(b"\n") && count > 0 {
            count -= 1;
        }
        escaped_newlines += count;
        let tail = line.strip_suffix(b"\n").unwrap_or(line);
        let tail = tail.strip_suffix(b"\r").unwrap_or(tail);
        let continued = line.ends_with(b"\n")
            && tail.iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
        if continued && lines.peek().is_some() {
            continue;
        }
        index += escaped_newlines;
        if let Some(line) = decoded.get(index) {
            emitted.push(line.ends_with(b"\n"));
        }
        index += 1;
        escaped_newlines = 0;
    }
    emitted
}
