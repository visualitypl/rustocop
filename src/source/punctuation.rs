//! Minimal, AST-aware scan shared by separator and parenthesis-spacing cops.
//! No lexer or extra parse: literal/token exclusions come from the main AST.

use std::ops::Range;

use ruby_prism::Node;

use super::{SourceFile, literal_ranges};

pub(crate) struct Punctuation {
    pub offset: usize,
    pub before: Option<Range<usize>>,
    pub missing_after: bool,
    /// Extraneous horizontal whitespace just inside an ordinary parenthesis.
    pub inside_parens: Option<Range<usize>>,
}

pub(super) struct PunctuationData {
    pub tokens: Vec<Punctuation>,
    /// Real square-bracket tokens, for constant patterns whose delimiters may
    /// be parentheses around nested arrays. Collected in the same bounded scan.
    pub square_brackets: Vec<usize>,
}

pub(super) fn collect(file: &SourceFile, root: &Node<'_>) -> PunctuationData {
    let literal_ranges::PunctuationRanges {
        protected,
        interpolation_ends,
        argument_parens,
        multiline_string_ends,
        ..
    } = file.token_ranges(root);
    let contains = |ranges: &[Range<usize>], offset: usize| {
        let after = ranges.partition_point(|r| r.start <= offset);
        after > 0 && ranges[after - 1].contains(&offset)
    };
    let data_end = file.data_start().unwrap_or(file.bytes.len());
    // Ruby also stops at NUL, Ctrl-D and Ctrl-Z in code, but not when
    // those bytes are part of a literal or comment token.
    let end = memchr::memchr3_iter(0, 4, 26, &file.bytes[..data_end])
        .find(|&offset| !contains(protected, offset) && !contains(file.comments(), offset))
        .unwrap_or(data_end);
    let bytes = &file.bytes[..end];
    let mut square_brackets = Vec::new();
    let tokens = bytes
        .iter()
        .enumerate()
        .filter(|&(offset, byte)| {
            matches!(byte, b',' | b';' | b'(' | b')' | b'[' | b']')
                && !contains(protected, offset)
                && !contains(file.comments(), offset)
        })
        .filter_map(|(offset, byte)| {
            if matches!(byte, b'[' | b']') {
                square_brackets.push(offset);
                return None;
            }
            if matches!(byte, b'(' | b')') {
                let mut start = offset;
                let mut end = offset + 1;
                let blank = |i: usize| {
                    matches!(bytes[i], b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
                        && !contains(protected, i)
                };
                let range = if *byte == b'(' {
                    start += 1;
                    while end < bytes.len() && blank(end) {
                        end += 1;
                    }
                    // Comments and physical/continued newlines exempt the gap.
                    if matches!(bytes.get(end), None | Some(b'\n' | b'#'))
                        || bytes[end..].starts_with(b"\\\n")
                        || bytes[end..].starts_with(b"\\\r\n")
                    {
                        return None;
                    }
                    // tLPAREN_ARG is not an ordinary left paren in the pinned
                    // cop. Empty pairs still report via the right-paren rule.
                    if bytes.get(end) != Some(&b')')
                        && argument_parens.binary_search(&offset).is_ok()
                    {
                        return None;
                    }
                    start..end
                } else {
                    end = offset;
                    while start > 0 && blank(start - 1) {
                        start -= 1;
                    }
                    // Empty pairs are already reported at their opening gap.
                    // Leading indentation is not space inside parentheses.
                    if start == 0
                        || multiline_string_ends.binary_search(&start).is_ok()
                        || bytes[start - 1] == b'\n'
                        || (bytes[start - 1] == b'(' && !contains(protected, start - 1))
                    {
                        return None;
                    }
                    start..end
                };
                return (!range.is_empty()).then_some(Punctuation {
                    offset,
                    before: None,
                    missing_after: false,
                    inside_parens: Some(range),
                });
            }
            // Never remove a line break or indentation before a leading comma.
            // A whitespace percent delimiter can be the preceding token's end.
            let mut start = offset;
            while start > 0
                && matches!(bytes[start - 1], b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
                && !contains(protected, start - 1)
            {
                start -= 1;
            }
            let before =
                (start < offset && start > 0 && bytes[start - 1] != b'\n').then_some(start..offset);
            let missing_after = bytes.get(offset + 1).is_some_and(|next| {
                !next.is_ascii_whitespace()
                    && !matches!(next, b')' | b']' | b'|' | b';')
                    && interpolation_ends.binary_search(&(offset + 1)).is_err()
                    // A continuation has no adjacent token on this line.
                    && !bytes[offset + 1..].starts_with(b"\\\n")
                    && !bytes[offset + 1..].starts_with(b"\\\r\n")
            });
            Some(Punctuation {
                offset,
                before,
                missing_after,
                inside_parens: None,
            })
        })
        .collect();
    PunctuationData {
        tokens,
        square_brackets,
    }
}
