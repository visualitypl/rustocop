//! Local spelling proofs shared by the short string/word literal cops.
use crate::source::SourceFile;

/// Inspect syntax only at character starts from the main parse's encoding.
/// In Windows-31J, for example, a character's trailing byte can be `\\`.
pub(super) fn character_heads(
    file: &SourceFile,
    range: std::ops::Range<usize>,
) -> impl Iterator<Item = u8> + '_ {
    file.character_offsets(range).map(|i| file.bytes[i])
}

/// RuboCop's double_quotes_required?: single quotes or odd backslash runs
/// followed by anything except a double quote. Scan each static fragment once.
pub(super) fn double_quotes_required(bytes: impl Iterator<Item = u8>) -> bool {
    let mut bytes = bytes.peekable();
    while let Some(byte) = bytes.next() {
        if byte == b'\'' {
            return true;
        }
        if byte == b'\\' {
            let mut odd = true;
            while bytes.next_if_eq(&b'\\').is_some() {
                odd = !odd;
            }
            if odd && bytes.peek().is_none_or(|&b| b != b'"') {
                return true;
            }
        }
    }
    false
}

/// Decode only single-quote rules, for comparison with the MAIN parse payload.
/// A quote not escaped here would terminate the replacement, so reject it.
pub(super) fn single_quoted_bytes(raw: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'\'' => return None,
            b'\\' if raw.get(i + 1).is_some_and(|&b| b == b'\\' || b == b'\'') => {
                i += 1;
                result.push(raw[i]);
            }
            b'\r' if raw.get(i + 1) == Some(&b'\n') => {
                i += 1;
                result.push(b'\n');
            }
            b => result.push(b),
        }
        i += 1;
    }
    Some(result)
}
