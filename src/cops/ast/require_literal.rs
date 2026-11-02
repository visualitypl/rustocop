//! Pinned parser-AST literal payload alignment shared by the literal-reading cops.
//! Keep main-parse decoded escape bytes, but project source characters (also
//! unknown escaped glyphs) as Parser::Source::Buffer does. No candidate parsing.
use crate::source::SourceFile;

pub(crate) fn parts(raw: &[u8], value: &[u8], single: bool, file: &SourceFile) -> Vec<Vec<u8>> {
    let mut parts = vec![Vec::new()];
    let (mut r, mut v) = (0, 0);
    while r < raw.len() && v < value.len() {
        let out = parts.last_mut().unwrap();
        if raw[r] == b'\\' {
            // An unknown escaped source glyph loses its backslash in Prism,
            // but its payload still has the original encoding. The reference
            // buffer transcodes that glyph *before* parsing the escape. Align
            // against the main decoded bytes, then project only this source
            // character, never numeric/Unicode/control escape result bytes.
            if !single && r + 1 < raw.len() {
                let width = file.composed_character_chunk(&raw[r + 1..]).0.max(1);
                let end = (r + 1 + width).min(raw.len());
                let character = &raw[r + 1..end];
                let projected = file.match_pattern_source(character);
                // UTF8-MAC may compose an ASCII escape letter with its raw
                // combining suffix *before* parsing (e.g. \\e + acute -> é).
                // Align the one-byte main escape result and the unchanged raw
                // suffix, rather than decoding either again or changing flags.
                let composed_escape = character[0].is_ascii()
                    && projected.first().is_some_and(|b| !b.is_ascii())
                    && escape(&raw[r..], false, file) == (2, 1)
                    && value[v + 1..].starts_with(&character[1..]);
                if (!character[0].is_ascii() && value[v..].starts_with(character))
                    || composed_escape
                {
                    out.extend_from_slice(&projected);
                    r = end;
                    v += character.len();
                    continue;
                }
            }
            let (source_len, decoded_len) =
                if single && raw.get(r + 1) != Some(&b'\\') && value.get(v) == Some(&b'\\') {
                    (1, 1) // retained backslash, not an escaped %q delimiter
                } else {
                    escape(&raw[r..], single, file)
                };
            let end = (v + decoded_len).min(value.len());
            out.extend_from_slice(&value[v..end]);
            r += source_len;
            v = end;
        } else if raw[r..].starts_with(b"\r\n") || raw[r] == b'\n' {
            // Prism normalizes physical CRLF in the decoded value. The pinned
            // translator retains physical string-part boundaries, unlike an
            // escaped \\n (and unlike a removed continuation).
            r += if raw[r] == b'\r' { 2 } else { 1 };
            if value.get(v) == Some(&b'\n') {
                out.push(b'\n');
                v += 1;
            }
            parts.push(Vec::new());
        } else {
            let width = file
                .composed_character_chunk(&raw[r..])
                .0
                .max(1)
                .min(raw.len() - r);
            let character = &raw[r..r + width];
            if value[v..].starts_with(character) {
                out.extend_from_slice(&file.match_pattern_source(character));
                v += width;
            } else if !character.iter().all(|b| matches!(b, b' ' | b'\t')) {
                // A partial heredoc tab can expand to spaces in the main
                // decoded value; retain those main-parse bytes, not source
                // indentation. Ordinary characters still advance exactly.
                while value.get(v).is_some_and(|b| matches!(b, b' ' | b'\t')) {
                    out.push(value[v]);
                    v += 1;
                }
                if value[v..].starts_with(character) {
                    out.extend_from_slice(&file.match_pattern_source(character));
                    v += width;
                }
            }
            r += width;
        }
    }
    parts.last_mut().unwrap().extend_from_slice(&value[v..]);
    if parts.len() > 1 && parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

// Return source-token width and main-decoded byte width, not an escape value.
// Ordinary escape results come from Prism; source glyph projection is handled
// in parts. This is an alignment map, not a second unescaper.
pub(crate) fn escape(raw: &[u8], single: bool, file: &SourceFile) -> (usize, usize) {
    let Some(&next) = raw.get(1) else {
        return (1, 1);
    };
    if single {
        return match next {
            b'\\' | b'\'' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'<' | b'>' => (2, 1),
            _ => (1, 1), // the backslash is retained
        };
    }
    match next {
        b'\n' => (2, 0),
        b'\r' if raw.get(2) == Some(&b'\n') => (3, 0),
        b'x' => (
            2 + raw[2..]
                .iter()
                .take(2)
                .take_while(|b| b.is_ascii_hexdigit())
                .count(),
            1,
        ),
        b'0'..=b'7' => (
            1 + raw[1..]
                .iter()
                .take(3)
                .take_while(|b| matches!(b, b'0'..=b'7'))
                .count(),
            1,
        ),
        b'u' => {
            if raw.get(2) == Some(&b'{') {
                let end = raw.iter().position(|&b| b == b'}').unwrap_or(raw.len() - 1);
                let len = raw[3..end]
                    .split(u8::is_ascii_whitespace)
                    .filter(|s| !s.is_empty())
                    .map(unicode_width)
                    .sum();
                (end + 1, len)
            } else {
                (6.min(raw.len()), unicode_width(&raw[2..raw.len().min(6)]))
            }
        }
        b'c' => {
            let (n, _) = control_character(&raw[2..], file);
            (2 + n, 1)
        }
        b'C' | b'M' if raw.get(2) == Some(&b'-') => {
            let (n, _) = control_character(&raw[3..], file);
            (3 + n, 1)
        }
        _ => {
            let width = file.encoded_character_width(&raw[1..]).max(1);
            (1 + width, width)
        }
    }
}
fn control_character(raw: &[u8], file: &SourceFile) -> (usize, usize) {
    if raw.starts_with(b"\\") {
        escape(raw, false, file)
    } else {
        (file.encoded_character_width(raw).max(1), 1)
    }
}
fn unicode_width(hex: &[u8]) -> usize {
    std::str::from_utf8(hex)
        .ok()
        .and_then(|s| u32::from_str_radix(s, 16).ok())
        .and_then(char::from_u32)
        .map_or(1, char::len_utf8)
}
