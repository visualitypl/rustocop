//! The source encoding's character widths and Ruby/Onigmo blanks. The Rust Prism
//! wrapper exposes magic comments, not the parser's canonical encoding handle.
//! Reuse those and the main parse's comment locations; never parse again here.

use ruby_prism::ParseResult;
use ruby_prism_sys::pm_encoding_t;

#[path = "constant_lowercase.rs"]
mod constant_lowercase;
#[path = "constant_uppercase_change.rs"]
mod constant_uppercase_change;
#[path = "escape_alnum.rs"]
mod escape_alnum;
#[path = "source_inspection.rs"]
mod source_inspection;
#[path = "word_array_classes.rs"]
mod word_array_classes;

// Public Prism C API, not yet allowlisted by ruby-prism-sys 1.9's bindgen.
// Use the parser's own alias lookup and width callbacks, not another codec's
// approximation of Ruby character boundaries. Keep all FFI in this module.
unsafe extern "C" {
    fn pm_encoding_find(start: *const u8, end: *const u8) -> *const pm_encoding_t;
}

#[path = "heredoc_uppercase.rs"]
mod heredoc_uppercase;
#[path = "naming_uppercase.rs"]
mod naming_uppercase;

type CharacterWidth = unsafe extern "C" fn(*const u8, isize) -> usize;

#[cfg(test)]
thread_local! {
    pub(super) static WIDTH_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ParsedEncoding {
    pub blanks: SourceEncoding,
    /// Actual UTF-8 identity, not CESU-8/mobile variants sharing its blanks.
    pub utf8: bool,
    pub binary: bool,
    /// Kelvin's canonical fold is not an uppercase mapping.
    inclusive_kelvin: &'static [u8],
    /// Effective parser setting: None preserves Ruby 3.4's chilled default.
    pub frozen_strings: Option<bool>,
    utf8_mac: bool,
    cesu8: bool,
    word_expansions: &'static [(&'static [u8], usize)],
    // None for single-byte encodings; only a function pointer is retained,
    // never a pointer into the parse or its source.
    char_width: Option<CharacterWidth>,
    // Unlike diagnostic widths, literal validity must not replace zero with 1
    // or skip US-ASCII's high-byte rejection. Used by Strip's exception proof.
    validation_width: Option<CharacterWidth>,
    // Prism callback fallback for binary/ASCII and profiles without projected
    // alnum facts. Decodable legacy profiles use generated source-buffer facts.
    single_byte_alnum: Option<CharacterWidth>,
    // Classification identity is wider than literal UTF-8 identity: all fixed
    // Unicode profiles project scalar characters to the pinned Unicode classes.
    unicode_alnum: bool,
    /// POSIX classes after the pinned source buffer's legacy-to-UTF8 projection,
    /// or positive Unicode mobile-projection overrides. Prism's multibyte alnum
    /// callback classifies identifiers, even punctuation.
    legacy_alnum: &'static [(u32, u32)],
    legacy_lowercase: Option<&'static [(u32, u32)]>,
    legacy_uppercase_change: Option<&'static [(u32, u32)]>,
    legacy_uppercase: Option<&'static [(u32, u32)]>,
    legacy_word: Option<&'static [(u32, u32)]>,
    heredoc_uppercase: Option<&'static [(u32, &'static [u8])]>,
    heredoc_aliases: Option<&'static [(u32, &'static [u8])]>,
    heredoc_ascii_letters: Option<&'static [u32]>,
    // The reference buffer recognizes headers differently from Prism. Keep
    // this cop-specific profile separate from all main-parse diagnostic facts.
    match_pattern_encoding: &'static [u8],
    match_pattern_projection: source_inspection::Projection,
    gb18030: bool,
}

/// POSIX uppercase for an already source-buffer-projected scalar (Ruby Unicode 15.0).
pub(crate) fn unicode_uppercase(c: char) -> bool {
    naming_uppercase::unicode(c as u32)
}

/// POSIX lowercase for UTF-8 path scalars, independent of source encoding.
/// Reuses the pinned Ruby Unicode table rather than Rust's evolving tables.
pub(crate) fn unicode_lowercase(c: char) -> bool {
    constant_lowercase::unicode(c as u32)
}

impl ParsedEncoding {
    pub fn from_parse(result: &ParseResult<'_>) -> Self {
        let name = effective_name(result);
        let match_pattern_encoding = match_pattern_encoding(result.source());
        let ruby_prism::Node::ProgramNode { parser, .. } = result.node() else {
            unreachable!("parse root is a ProgramNode")
        };
        // SAFETY: result owns this live parser; copy only its scalar metadata.
        // Prism resolves repeated/invalid/late directives, including emacs forms.
        let frozen_strings = match unsafe { parser.as_ref().frozen_string_literal } {
            1 => Some(true),
            -1 => Some(false),
            _ => None,
        };
        Self {
            blanks: SourceEncoding::from_name(name),
            utf8: encoding_is(name, b"UTF-8"),
            binary: encoding_is(name, b"ASCII-8BIT"),
            inclusive_kelvin: if encoding_is(name, b"GB18030") {
                b"\x81\x36\xbf\x32"
            } else if SourceEncoding::from_name(name) == SourceEncoding::Utf8 {
                "K".as_bytes()
            } else {
                b""
            },
            frozen_strings,
            utf8_mac: encoding_is(name, b"UTF8-MAC"),
            cesu8: encoding_is(name, b"CESU-8"),
            word_expansions: word_expansions(name),
            char_width: character_width(name),
            validation_width: validation_width(name),
            single_byte_alnum: single_byte_alnum(name),
            unicode_alnum: legacy_case_ranges(name, escape_alnum::ranges).is_none(),
            legacy_alnum: legacy_alnum(name),
            legacy_lowercase: legacy_case_ranges(name, constant_lowercase::ranges),
            legacy_uppercase_change: legacy_case_ranges(name, constant_uppercase_change::ranges),
            legacy_uppercase: legacy_case_ranges(name, naming_uppercase::ranges),
            legacy_word: legacy_word(name),
            match_pattern_encoding,
            match_pattern_projection: source_inspection::projection(match_pattern_encoding),
            gb18030: encoding_is(name, b"GB18030"),
            heredoc_uppercase: legacy_case_ranges(name, heredoc_uppercase::legacy),
            heredoc_ascii_letters: legacy_case_ranges(name, heredoc_uppercase::ascii_letters),
            heredoc_aliases: [b"UTF8-DoCoMo".as_slice(), b"UTF8-KDDI", b"UTF8-SoftBank"]
                .into_iter()
                .find(|canonical| encoding_is(name, canonical))
                .map(heredoc_uppercase::mobile_words)
                .or_else(|| legacy_case_ranges(name, heredoc_uppercase::aliases)),
        }
    }

    /// DuplicateMatchPattern compares source strings exposed by the reference
    /// buffer, not original encoded bytes or decoded literal values. Project only
    /// the requested syntax/column-prefix slice; never mutate the file or offsets.
    pub fn match_pattern_source(mut self, bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
        let name = self.match_pattern_encoding;
        if matches!(name, b"UTF-8" | b"ASCII-8BIT") || bytes.is_ascii() {
            return std::borrow::Cow::Borrowed(bytes);
        }
        // Change only this local copy, never the shared parser/diagnostic profile.
        self.utf8_mac = name == b"UTF8-MAC";
        self.cesu8 = name == b"CESU-8";
        self.gb18030 = name == b"GB18030";
        self.char_width = character_width(name);
        if self.utf8_mac {
            return self.heredoc_normalized(bytes);
        }
        let mut out = Vec::with_capacity(bytes.len());
        let mut offset = 0;
        let ranges = self.match_pattern_projection.scalars;
        let expansions = self.match_pattern_projection.expansions;
        while offset < bytes.len() {
            let tail = &bytes[offset..];
            let width = self.width(tail).min(tail.len());
            let raw = &tail[..width];
            // Prism exposes the two CESU-8 surrogate halves separately; the
            // source buffer projects their valid pair to one UTF8 scalar.
            if self.cesu8
                && tail.len() >= 6
                && tail[0] == 0xed
                && tail[3] == 0xed
                && (0xa0..=0xaf).contains(&tail[1])
                && (0xb0..=0xbf).contains(&tail[4])
            {
                let high = (u32::from(tail[1] & 0x3f) << 6) | u32::from(tail[2] & 0x3f);
                let low = (u32::from(tail[4] & 0x3f) << 6) | u32::from(tail[5] & 0x3f);
                let code = 0x10000 + ((high - 0x800) << 10) + (low - 0xc00);
                out.extend_from_slice(
                    char::from_u32(code)
                        .unwrap()
                        .encode_utf8(&mut [0; 4])
                        .as_bytes(),
                );
                offset += 6;
                continue;
            }
            let code = if self.gb18030 && width == 4 {
                0x10000
                    + (((u32::from(raw[0]) - 0x81) * 10 + u32::from(raw[1]) - 0x30) * 126
                        + u32::from(raw[2])
                        - 0x81)
                        * 10
                    + u32::from(raw[3])
                    - 0x30
            } else {
                raw.iter().fold(0u32, |n, &b| (n << 8) | u32::from(b))
            };
            if let Some(&(start, end, unicode)) =
                ranges.get(ranges.partition_point(|&(_, end, _)| end < code))
                && (start..=end).contains(&code)
            {
                out.extend_from_slice(
                    char::from_u32(unicode + code - start)
                        .unwrap()
                        .encode_utf8(&mut [0; 4])
                        .as_bytes(),
                );
            } else if let Ok(i) = expansions.binary_search_by_key(&code, |&(key, _)| key) {
                out.extend_from_slice(expansions[i].1);
            } else {
                // ASCII and unchanged UTF8-profile scalars need no facts. A
                // legacy unit with no Unicode mapping cannot enter the pinned
                // buffer; preserve its exact bytes, never invent U+FFFD.
                out.extend_from_slice(raw);
            }
            offset += width;
        }
        std::borrow::Cow::Owned(out)
    }

    pub fn match_pattern_composes_identifiers(self) -> bool {
        self.match_pattern_encoding == b"UTF8-MAC"
    }

    pub fn match_pattern_binary(self) -> bool {
        self.match_pattern_encoding == b"ASCII-8BIT"
    }

    /// The pinned source buffer composes UTF8-MAC before applying /\\w/ or
    /// upcase. Reuse bounded generated prefixes; no general-purpose codec.
    pub fn heredoc_normalized(self, bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
        if !self.utf8_mac {
            let Some(table) = self.heredoc_aliases.filter(|table| !table.is_empty()) else {
                return std::borrow::Cow::Borrowed(bytes);
            };
            let mut out = Vec::with_capacity(bytes.len());
            let mut offset = 0;
            while offset < bytes.len() {
                let width = self.width(&bytes[offset..]).min(bytes.len() - offset);
                let raw = &bytes[offset..offset + width];
                let code = raw.iter().fold(0u32, |n, &b| (n << 8) | u32::from(b));
                let mapped = table
                    .get(table.partition_point(|&(c, _)| c < code))
                    .filter(|&&(c, _)| c == code)
                    .map_or(raw, |&(_, canonical)| canonical);
                out.extend_from_slice(mapped);
                offset += width;
            }
            return std::borrow::Cow::Owned(out);
        }
        let mut out = Vec::with_capacity(bytes.len());
        let mut offset = 0;
        while offset < bytes.len() {
            let tail = &bytes[offset..];
            let table = heredoc_uppercase::MAC;
            let mapped = (3..=tail.len().min(9)).rev().find_map(|n| {
                table
                    .binary_search_by(|(raw, _)| raw.cmp(&&tail[..n]))
                    .ok()
                    .map(|i| (n, table[i].1))
            });
            if let Some((len, replacement)) = mapped {
                out.extend_from_slice(replacement);
                offset += len;
                continue;
            }
            let scalar = |at| {
                tail.get(at..at + 3)
                    .and_then(|s| std::str::from_utf8(s).ok())
                    .and_then(|s| s.chars().next())
                    .map(u32::from)
            };
            let first = scalar(0);
            let second = scalar(3);
            let syllable = match (first, second) {
                (Some(l @ 0x1100..=0x1112), Some(v @ 0x1161..=0x1175)) => {
                    Some((0xac00 + (l - 0x1100) * 588 + (v - 0x1161) * 28, 6))
                }
                (Some(s @ 0xac00..=0xd7a3), Some(t @ 0x11a8..=0x11c2))
                    if (s - 0xac00) % 28 == 0 =>
                {
                    Some((s + t - 0x11a7, 6))
                }
                _ => None,
            };
            if let Some((mut code, mut len)) = syllable {
                if (code - 0xac00) % 28 == 0
                    && let Some(t @ 0x11a8..=0x11c2) = scalar(6)
                {
                    code += t - 0x11a7;
                    len += 3;
                }
                out.extend_from_slice(
                    char::from_u32(code)
                        .unwrap()
                        .encode_utf8(&mut [0; 4])
                        .as_bytes(),
                );
                offset += len;
                continue;
            }
            let width = self.width(tail).min(tail.len());
            out.extend_from_slice(&tail[..width]);
            offset += width;
        }
        std::borrow::Cow::Owned(out)
    }

    /// Raw bytes consumed and character count after UTF8-MAC composition.
    /// Keep source offsets intact, reusing the pinned bounded prefix facts
    /// (including algorithmic Hangul), rather than applying general NFC.
    pub fn composed_character_chunk(self, bytes: &[u8]) -> (usize, usize) {
        if self.utf8_mac {
            let width = word_array_classes::mac_word_prefix(bytes);
            if width > 0 {
                return (width, 1);
            }
            let table = heredoc_uppercase::MAC;
            if let Some(chunk) = (3..=bytes.len().min(9)).rev().find_map(|n| {
                table
                    .binary_search_by(|(raw, _)| raw.cmp(&&bytes[..n]))
                    .ok()
                    .map(|i| (n, table[i].1))
            }) {
                // Generated replacements are valid UTF-8, possibly several
                // scalars. No chunk contains a line break, so a range ending
                // inside it has the same last_line as its raw endpoint.
                let count = chunk.1.iter().filter(|&&b| b & 0xc0 != 0x80).count();
                return (chunk.0, count);
            }
        }
        (self.width(bytes).min(bytes.len()), 1)
    }

    /// Ruby's Unicode-aware /[A-Z]/i also matches long-s and Kelvin sign,
    /// but NOT dotless-i or general characters whose upcase is ASCII.
    pub fn heredoc_ascii_letter(self, bytes: &[u8]) -> bool {
        let bytes = self.heredoc_normalized(bytes);
        let bytes = bytes.as_ref();
        if bytes.len() == 1 && bytes[0].is_ascii_alphabetic() {
            return true;
        }
        if let Some(table) = self.heredoc_ascii_letters {
            if bytes.len() > 4 {
                return false;
            }
            let code = bytes.iter().fold(0u32, |n, &b| (n << 8) | u32::from(b));
            table.binary_search(&code).is_ok()
        } else {
            matches!(bytes, b"\xc5\xbf" | b"\xe2\x84\xaa")
        }
    }

    pub fn heredoc_has_ascii_word(self, bytes: &[u8]) -> bool {
        let normalized = self.heredoc_normalized(bytes);
        let mut bytes = normalized.as_ref();
        while !bytes.is_empty() {
            // The accepted mobile Word expansions are digit+keycap pairs,
            // and therefore contain an ASCII /\\w/ too (verified by the tool
            // test). UTF-8 mobile prefixes normalize above; SJIS uses raw heads.
            if bytes[0].is_ascii_alphanumeric()
                || bytes[0] == b'_'
                || self
                    .word_expansions
                    .iter()
                    .any(|&(raw, _)| bytes.starts_with(raw))
            {
                return true;
            }
            bytes = &bytes[self.width(bytes).min(bytes.len())..];
        }
        false
    }

    /// Uppercase spelling in source bytes, solely for heredoc collision checks.
    /// None means the uppercase projection is not representable in this encoding.
    pub fn heredoc_uppercase(self, bytes: &[u8]) -> Option<Vec<u8>> {
        let bytes = self.heredoc_normalized(bytes);
        let bytes = bytes.as_ref();
        let mut out = Vec::with_capacity(bytes.len());
        let mut offset = 0;
        while offset < bytes.len() {
            let tail = &bytes[offset..];
            if self.cesu8
                && tail.len() >= 6
                && tail[0] == 0xed
                && (0xa0..=0xaf).contains(&tail[1])
                && (0x80..=0xbf).contains(&tail[2])
                && tail[3] == 0xed
                && (0xb0..=0xbf).contains(&tail[4])
                && (0x80..=0xbf).contains(&tail[5])
            {
                let high = ((u32::from(tail[1]) & 0x3f) << 6) | (u32::from(tail[2]) & 0x3f);
                let low = ((u32::from(tail[4]) & 0x3f) << 6) | (u32::from(tail[5]) & 0x3f);
                let code = 0x10000 + ((high - 0x800) << 10) + (low - 0xc00);
                let table = heredoc_uppercase::UNICODE;
                if let Some((_, upper)) = table
                    .get(table.partition_point(|&(c, _)| c < code))
                    .filter(|&&(c, _)| c == code)
                {
                    for unit in upper.encode_utf16() {
                        let code = u32::from(unit);
                        out.extend_from_slice(&[
                            (0xe0 | (code >> 12)) as u8,
                            (0x80 | ((code >> 6) & 0x3f)) as u8,
                            (0x80 | (code & 0x3f)) as u8,
                        ]);
                    }
                } else {
                    out.extend_from_slice(&tail[..6]);
                }
                offset += 6;
                continue;
            }
            let width = self.width(&bytes[offset..]).min(bytes.len() - offset);
            let character = &bytes[offset..offset + width];
            if character[0].is_ascii() {
                out.push(character[0].to_ascii_uppercase());
            } else if self.uppercase_changes(character) {
                let code = character.iter().fold(0u32, |n, &b| (n << 8) | u32::from(b));
                if let Some(table) = self.heredoc_uppercase {
                    out.extend_from_slice(
                        table
                            .get(table.partition_point(|&(c, _)| c < code))
                            .filter(|&&(c, _)| c == code)?
                            .1,
                    );
                } else {
                    let code = std::str::from_utf8(character).ok()?.chars().next()? as u32;
                    let table = heredoc_uppercase::UNICODE;
                    out.extend_from_slice(
                        table
                            .get(table.partition_point(|&(c, _)| c < code))
                            .filter(|&&(c, _)| c == code)?
                            .1
                            .as_bytes(),
                    );
                }
            } else {
                out.extend_from_slice(character);
            }
            offset += width;
        }
        Some(out)
    }

    /// Width of the first character, with the same invalid-byte fallback as
    /// diagnostic columns. Call only with a nonempty slice.
    pub fn width(self, bytes: &[u8]) -> usize {
        #[cfg(test)]
        WIDTH_CALLS.set(WIDTH_CALLS.get() + 1);
        let Some(char_width) = self.char_width.filter(|_| !bytes[0].is_ascii()) else {
            return 1;
        };
        // SAFETY: bytes is nonempty and valid for its supplied length.
        // Prism callbacks read at most that length and have no state.
        unsafe { char_width(bytes.as_ptr(), bytes.len() as isize) }.max(1)
    }

    /// Bounded first projected character. Prism exposes CESU-8 surrogate halves
    /// as separate three-byte character heads; the reference buffer combines a
    /// valid pair before its POSIX predicate. Never change source widths/offsets.
    pub fn alphanumeric_prefix(self, bytes: &[u8]) -> bool {
        if self.cesu8
            && bytes.len() >= 6
            && bytes[0] == 0xed
            && (0xa0..=0xaf).contains(&bytes[1])
            && (0x80..=0xbf).contains(&bytes[2])
            && bytes[3] == 0xed
            && (0xb0..=0xbf).contains(&bytes[4])
            && (0x80..=0xbf).contains(&bytes[5])
        {
            let high = ((u32::from(bytes[1]) & 0x3f) << 6) | (u32::from(bytes[2]) & 0x3f);
            let low = ((u32::from(bytes[4]) & 0x3f) << 6) | (u32::from(bytes[5]) & 0x3f);
            return escape_alnum::unicode(0x10000 + ((high - 0x800) << 10) + (low - 0xc00));
        }
        !bytes.is_empty() && self.alphanumeric(&bytes[..self.width(bytes).min(bytes.len())])
    }

    pub fn alphanumeric(self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        if bytes[0].is_ascii_alphanumeric() {
            return true;
        }
        let encoded_alnum = if bytes.len() <= 4 && !self.legacy_alnum.is_empty() {
            let code = bytes.iter().fold(0, |code, &b| (code << 8) | u32::from(b));
            let i = self.legacy_alnum.partition_point(|&(_, end)| end < code);
            self.legacy_alnum
                .get(i)
                .is_some_and(|&(start, _)| start <= code)
        } else {
            false
        };
        if self.unicode_alnum {
            // Mobile private-use characters can project to an Alphabetic
            // scalar or a decimal + keycap sequence. Generated positive
            // overrides cover every class-changing Unicode-profile mapping.
            if encoded_alnum {
                return true;
            }
            // Pin POSIX Alphabetic/Decimal_Number to the reference Unicode
            // version, independently of regex's evolving Unicode tables.
            return std::str::from_utf8(bytes).is_ok_and(|s| {
                let mut chars = s.chars();
                chars
                    .next()
                    .is_some_and(|c| escape_alnum::unicode(c as u32))
                    && chars.next().is_none()
            });
        }
        if !self.legacy_alnum.is_empty() {
            return encoded_alnum;
        }
        // SAFETY: same immutable encoding callback contract as width above.
        self.single_byte_alnum
            .is_some_and(|classify| unsafe { classify(bytes.as_ptr(), bytes.len() as isize) != 0 })
    }

    pub fn contains_lowercase(self, bytes: &[u8]) -> bool {
        self.contains_case_class(
            bytes,
            self.legacy_lowercase,
            constant_lowercase::unicode,
            false,
        )
    }

    /// Ruby's upcase-change predicate is not POSIX lowercase: e.g. ª has no
    /// uppercase mapping, whereas titlecase ǅ changes to Ǆ. Tables use the
    /// pinned source buffer's UTF-8 projection, including legacy encodings.
    pub fn uppercase_changes(self, bytes: &[u8]) -> bool {
        self.contains_case_class(
            bytes,
            self.legacy_uppercase_change,
            constant_uppercase_change::unicode,
            false,
        )
    }

    pub fn contains_uppercase(self, bytes: &[u8]) -> bool {
        self.contains_case_class(
            bytes,
            self.legacy_uppercase,
            naming_uppercase::unicode,
            true,
        )
    }

    fn contains_case_class(
        self,
        mut bytes: &[u8],
        legacy: Option<&'static [(u32, u32)]>,
        unicode: fn(u32) -> bool,
        uppercase: bool,
    ) -> bool {
        while !bytes.is_empty() {
            let width = self.width(bytes);
            let character = &bytes[..width.min(bytes.len())];
            let lower = if character[0].is_ascii() {
                if uppercase {
                    character[0].is_ascii_uppercase()
                } else {
                    character[0].is_ascii_lowercase()
                }
            } else if let Some(ranges) = legacy {
                let code = character
                    .iter()
                    .fold(0u32, |code, &b| (code << 8) | u32::from(b));
                let i = ranges.partition_point(|&(_, end)| end < code);
                ranges.get(i).is_some_and(|&(start, _)| start <= code)
            } else if self.cesu8
                && bytes.len() >= 6
                && bytes[0] == 0xed
                && (0xa0..=0xaf).contains(&bytes[1])
                && bytes[3] == 0xed
                && (0xb0..=0xbf).contains(&bytes[4])
            {
                let high = ((u32::from(bytes[1]) & 0x3f) << 6) | (u32::from(bytes[2]) & 0x3f);
                let low = ((u32::from(bytes[4]) & 0x3f) << 6) | (u32::from(bytes[5]) & 0x3f);
                unicode(0x10000 + ((high - 0x800) << 10) + (low - 0xc00))
            } else {
                std::str::from_utf8(character).is_ok_and(|s| s.chars().any(|c| unicode(c as u32)))
            };
            if lower {
                return true;
            }
            bytes = &bytes[width.min(bytes.len())..];
        }
        false
    }

    pub fn valid(self, mut bytes: &[u8]) -> bool {
        let Some(width) = self.validation_width else {
            return false;
        };
        while !bytes.is_empty() {
            // SAFETY: immutable Prism callback, bounded by the live slice.
            let size = unsafe { width(bytes.as_ptr(), bytes.len() as isize) };
            if size == 0 || size > bytes.len() {
                return false;
            }
            bytes = &bytes[size..];
        }
        true
    }

    /// /i/ folds needed by the fixed InclusiveLanguage terms. Existing generated
    /// uppercase projections identify long s and the two st ligatures in every
    /// supported codec. Kelvin is canonically K but does not change under upcase.
    pub fn inclusive_fold(self, bytes: &[u8]) -> [u8; 2] {
        if !bytes.is_empty() && bytes == self.inclusive_kelvin {
            return [b'k', 0];
        }
        match self.heredoc_uppercase(bytes).as_deref() {
            Some(b"S") => [b's', 0],
            Some(b"ST") => *b"st",
            _ => [0, 0],
        }
    }

    pub fn word_projection_required(self) -> bool {
        self.legacy_word.is_some()
            || self.utf8_mac
            || self.cesu8
            || !self.word_expansions.is_empty()
    }

    /// Exceptional raw source prefixes whose UTF-8 buffer projection changes
    /// the number of Word units. Only class/count facts, never a general codec.
    pub fn word_prefix(self, bytes: &[u8]) -> Option<(usize, usize)> {
        if self.utf8_mac {
            let width = word_array_classes::mac_word_prefix(bytes);
            if width > 0 {
                return Some((width, 1));
            }
        }
        if self.cesu8
            && bytes.len() >= 6
            && bytes[0] == 0xed
            && (0xa0..=0xaf).contains(&bytes[1])
            && (0x80..=0xbf).contains(&bytes[2])
            && bytes[3] == 0xed
            && (0xb0..=0xbf).contains(&bytes[4])
            && (0x80..=0xbf).contains(&bytes[5])
        {
            let high = ((u32::from(bytes[1]) & 0x3f) << 6) | (u32::from(bytes[2]) & 0x3f);
            let low = ((u32::from(bytes[4]) & 0x3f) << 6) | (u32::from(bytes[5]) & 0x3f);
            let scalar = 0x10000 + ((high - 0x800) << 10) + (low - 0xc00);
            return Some((6, usize::from(word_array_classes::unicode(scalar))));
        }
        self.word_expansions
            .iter()
            .find_map(|&(raw, units)| bytes.starts_with(raw).then_some((raw.len(), units)))
    }

    pub fn word(self, bytes: &[u8], projected: bool) -> bool {
        if projected || self.legacy_word.is_none() {
            return std::str::from_utf8(bytes).is_ok_and(|s| {
                let mut chars = s.chars();
                chars
                    .next()
                    .is_some_and(|c| word_array_classes::unicode(c as u32))
                    && chars.next().is_none()
            });
        }
        if bytes.len() > 4 {
            return false;
        }
        let code = bytes.iter().fold(0, |v, &b| (v << 8) | u32::from(b));
        bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            || bytes == b"_"
            || word_array_classes::contains(self.legacy_word.unwrap(), code)
    }

    pub fn single_byte(self) -> bool {
        self.char_width.is_none()
    }

    pub fn count_chars(self, mut bytes: &[u8]) -> usize {
        if self.char_width.is_none() {
            return bytes.len();
        }
        let mut count = 0;
        while !bytes.is_empty() {
            bytes = &bytes[self.width(bytes)..];
            count += 1;
        }
        count
    }
}

fn encoding_is(name: &[u8], wanted: &[u8]) -> bool {
    // SAFETY: both pointer pairs delimit their respective live slices.
    // Alias lookup returns the same immutable encoding-table entry; comparing
    // pointers distinguishes UTF-8 aliases from CESU-8 and UTF8-MAC.
    unsafe {
        pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len()))
            == pm_encoding_find(wanted.as_ptr(), wanted.as_ptr().add(wanted.len()))
    }
}

// Parser::Source::Buffer recognizes only column-zero comments, after *any*
// shebang, and selects the first matching coding declaration (with emacs suffix
// handling). This is deliberately separate from effective_name/Prism metadata.
// Inspect at most the two header lines once per parse, never from a cop node.
fn match_pattern_encoding(source: &[u8]) -> &'static [u8] {
    if source.starts_with(b"\xef\xbb\xbf") {
        return b"UTF-8";
    }
    let mut lines = source.split(|&b| b == b'\n');
    let first = lines.next().unwrap_or_default();
    let header = if first.starts_with(b"#!") {
        lines.next().unwrap_or_default()
    } else {
        first
    };
    if !header.starts_with(b"#") {
        return b"UTF-8";
    }
    static CODING: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {
        regex::bytes::Regex::new(r"[ \t\r\n\x0b\x0c#](?:en)?coding[ \t\r\n\x0b\x0c]*[:=][ \t\r\n\x0b\x0c]*(?:(utf8-mac)|([A-Za-z0-9_-]+?)(?:-unix|-dos|-mac)|([A-Za-z0-9_-]+))").unwrap()
    });
    let Some(captures) = CODING.captures(header) else {
        return b"UTF-8";
    };
    let name = (1..=3).find_map(|i| captures.get(i)).unwrap().as_bytes();
    // SAFETY: lookup returns an immutable, process-lifetime canonical name.
    let encoding =
        unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() };
    encoding.map_or(b"UTF-8", |e| {
        unsafe { std::ffi::CStr::from_ptr(e.name) }.to_bytes()
    })
}

/// POSIX alnum for a character already in Parser::Source::Buffer's UTF-8
/// projection. DuplicateMagicComment must not use regex's evolving Unicode/N
/// classes, or original-codec predicates on projected bytes.
pub(crate) fn projected_alphanumeric(character: char) -> bool {
    escape_alnum::unicode(character as u32)
}

/// String inspection after projection always uses the pinned Unicode15 facts.
/// Binary source is inspected separately without Unicode classification.
pub(super) fn inspection_nonprint(bytes: &[u8]) -> Option<u32> {
    let code = std::str::from_utf8(bytes).ok()?.chars().next()? as u32;
    source_inspection::unicode_nonprint(code).then_some(code)
}

fn legacy_case_ranges<T: 'static>(
    name: &[u8],
    ranges: fn(&[u8]) -> &'static [T],
) -> Option<&'static [T]> {
    // SAFETY: same immutable, process-lifetime encoding lookup as below.
    let encoding =
        unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() }?;
    let name = unsafe { std::ffi::CStr::from_ptr(encoding.name) }.to_bytes();
    if name == b"UTF-8" || name.starts_with(b"UTF8") || name == b"CESU-8" {
        None
    } else {
        Some(ranges(name))
    }
}

fn legacy_alnum(name: &[u8]) -> &'static [(u32, u32)] {
    // SAFETY: alias lookup returns an immutable, process-lifetime entry with a
    // NUL-terminated canonical name. No source/parse pointer is retained.
    let Some(encoding) =
        (unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() })
    else {
        return &[];
    };
    let name = unsafe { std::ffi::CStr::from_ptr(encoding.name) }.to_bytes();
    escape_alnum::ranges(name)
}

fn legacy_word(name: &[u8]) -> Option<&'static [(u32, u32)]> {
    // SAFETY: immutable process-lifetime Prism encoding entry and canonical name.
    let encoding =
        unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() }?;
    let name = unsafe { std::ffi::CStr::from_ptr(encoding.name) }.to_bytes();
    if matches!(
        name,
        b"UTF-8"
            | b"ASCII-8BIT"
            | b"US-ASCII"
            | b"UTF8-MAC"
            | b"CESU-8"
            | b"UTF8-DoCoMo"
            | b"UTF8-KDDI"
            | b"UTF8-SoftBank"
    ) {
        None
    } else {
        Some(word_array_classes::legacy(name))
    }
}

fn word_expansions(name: &[u8]) -> &'static [(&'static [u8], usize)] {
    // SAFETY: immutable process-lifetime canonical encoding name, as above.
    let Some(encoding) =
        (unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() })
    else {
        return &[];
    };
    let name = unsafe { std::ffi::CStr::from_ptr(encoding.name) }.to_bytes();
    word_array_classes::expansions(name)
}

fn single_byte_alnum(name: &[u8]) -> Option<CharacterWidth> {
    // SAFETY: the pointer pair delimits a live slice; the result is immutable.
    let encoding =
        unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() }?;
    if encoding.multibyte {
        None
    } else {
        encoding.alnum_char
    }
}

fn validation_width(name: &[u8]) -> Option<CharacterWidth> {
    // SAFETY: lookup returns an immutable, process-lifetime encoding entry.
    unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() }?.char_width
}

fn character_width(name: &[u8]) -> Option<CharacterWidth> {
    // SAFETY: both pointers delimit the same live slice. The returned pointer
    // is null or points to Prism's immutable, process-lifetime encoding table.
    let encoding =
        unsafe { pm_encoding_find(name.as_ptr(), name.as_ptr().add(name.len())).as_ref() };
    let Some(encoding) = encoding else {
        // Prism leaves the default UTF-8 encoding in place for an unknown name
        // and emits a diagnostic. Column mapping must work for that case too.
        return character_width(b"UTF-8");
    };
    if encoding.multibyte {
        encoding.char_width
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceEncoding {
    /// Unicode blank suffixes encoded as UTF-8 (also CESU-8's BMP blanks).
    Utf8,
    SingleByteBlank(u8),
    AsciiBlanks,
}

fn effective_name<'a>(result: &ParseResult<'a>) -> &'a [u8] {
    let source = result.source();
    // A BOM locks Prism to UTF-8, even with a different encoding pragma.
    if source.starts_with(b"\xef\xbb\xbf") {
        return b"UTF-8";
    }
    let mut start = 0;
    let first_line_end = source
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(source.len());
    if source.starts_with(b"#!")
        && source[..first_line_end]
            .windows(4)
            .any(|word| word == b"ruby")
    {
        start = (first_line_end + 1).min(source.len());
    }
    start += source[start..]
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c))
        .count();
    if source.get(start) != Some(&b'#') {
        return b"UTF-8";
    }
    let Some(comment) = result
        .comments()
        .find(|c| c.location().start_offset() == start)
    else {
        return b"UTF-8";
    };
    let end = comment.location().end_offset();
    let mut has_magic_comments = false;
    let mut encoding = b"UTF-8".as_slice();
    for magic in result.magic_comments() {
        let offset = magic.value().as_ptr() as usize - source.as_ptr() as usize;
        if !(start..end).contains(&offset) {
            continue;
        }
        has_magic_comments = true;
        if magic.key().eq_ignore_ascii_case(b"coding")
            || magic.key().eq_ignore_ascii_case(b"encoding")
        {
            encoding = &source[offset..offset + magic.value().len()];
        }
    }
    if has_magic_comments {
        return encoding;
    }
    // Prism also accepts legacy `coding = NAME` or free-form comments.
    // These do not appear in magic_comments(). Mirror its fallback's
    // coding + whitespace + ':'/'=' + whitespace + encoding-name grammar.
    let text = &source[start..end];
    for (i, window) in text.windows(6).enumerate() {
        if !window.eq_ignore_ascii_case(b"coding") {
            continue;
        }
        let rest = &text[i + 6..];
        if !rest
            .first()
            .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b':' | b'='))
        {
            continue;
        }
        let rest = rest.trim_ascii_start();
        if !matches!(rest.first(), Some(b':' | b'=')) {
            // Prism stops at the first candidate followed by whitespace or
            // a separator, even if it has no valid encoding declaration.
            return b"UTF-8";
        }
        let rest = rest[1..].trim_ascii_start();
        let len = rest
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            .count();
        return &rest[..len];
    }
    b"UTF-8"
}

impl SourceEncoding {
    fn from_name(name: &[u8]) -> Self {
        let name = String::from_utf8_lossy(name).to_ascii_uppercase();
        if name.starts_with("UTF-8")
            || matches!(
                name.as_str(),
                "UTF8-MAC" | "UTF8-DOCOMO" | "UTF8-KDDI" | "UTF8-SOFTBANK" | "CESU-8" | "CP65001"
            )
        {
            return Self::Utf8;
        }
        // Non-Unicode Onigmo encodings have only these additional blanks.
        if name.starts_with("ISO-8859-")
            || name.starts_with("ISO8859-")
            || matches!(name.as_str(), "TIS-620" | "WINDOWS-874" | "CP874")
            || ["1250", "1252", "1253", "1254", "1255", "1256", "1257"]
                .iter()
                .any(|n| name == format!("CP{n}") || name == format!("WINDOWS-{n}"))
        {
            Self::SingleByteBlank(0xa0)
        } else if matches!(name.as_str(), "KOI8-R" | "KOI8-U" | "CP878") {
            Self::SingleByteBlank(0x9a)
        } else {
            Self::AsciiBlanks
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_posix_classes_follow_decoded_characters_not_identifier_widths() {
        for (name, bytes, expected) in [
            ("CP932", b"\x83\x5c".as_slice(), true),
            ("csWindows31J", b"\x83\x7d".as_slice(), true),
            ("sJiS", b"\x82\x50".as_slice(), true), // fullwidth decimal
            ("Shift_JIS", b"\xb1".as_slice(), true), // halfwidth katakana
            ("Windows-31J", b"\x81\x99".as_slice(), false), // star
            ("eucJP", b"\x8f\xab\xb1".as_slice(), true), // three-byte é
            ("EUC-JP", b"\xa1\xf9".as_slice(), false),
            ("Big5", b"\xa4\xa4".as_slice(), true),
            ("Big5", b"\xa1\x42".as_slice(), false),
            ("CP936", b"\xd6\xd0".as_slice(), true),
            ("GBK", b"\xa1\xee".as_slice(), false),
            ("UTF-8", "Ⅰ".as_bytes(), true),  // Alphabetic
            ("UTF-8", "²".as_bytes(), false), // Numeric, not Decimal_Number
        ] {
            let mut source = format!("# encoding: {name}\np \"").into_bytes();
            source.extend_from_slice(bytes);
            source.extend_from_slice(b"\"\n");
            let parsed = crate::parser::parse(&source);
            assert_eq!(parsed.errors().count(), 0, "{name}");
            let encoding = ParsedEncoding::from_parse(&parsed);
            assert_eq!(encoding.width(bytes), bytes.len(), "{name}");
            assert_eq!(encoding.alphanumeric(bytes), expected, "{name}: {bytes:?}");
        }
        for name in [
            b"Windows-31J".as_slice(),
            b"Shift_JIS",
            b"EUC-JP",
            b"Big5",
            b"GBK",
        ] {
            let ranges = legacy_alnum(name);
            assert!(!ranges.is_empty());
            assert!(ranges.iter().all(|&(start, end)| start <= end));
            assert!(ranges.windows(2).all(|w| w[0].1 < w[1].0));
        }
    }

    #[test]
    fn recovery_korean_and_gb18030_posix_classes_include_four_byte_characters() {
        for (name, bytes, expected) in [
            ("EUC-KR", b"\xb0\xa1".as_slice(), true), // 가: counterexample
            ("eucKR", b"\xa1\xa2".as_slice(), false), // 、
            ("CP949", b"\xb0\xa1".as_slice(), true),
            ("CP949", b"\x81\x41".as_slice(), true), // 갂: CP949 extension
            ("CP949", b"\xa1\xd9".as_slice(), false), // ☆
            ("GB18030", b"\xd6\xd0".as_slice(), true), // 中: counterexample
            ("GB18030", b"\x82\x37\xcf\x35".as_slice(), true), // 가
            ("GB18030", b"\x90\x30\xeb\x34".as_slice(), true), // Deseret letter
            ("GB18030", b"\x95\x32\x82\x36".as_slice(), true), // supplementary CJK
            ("GB18030", b"\x94\x33\xec\x37".as_slice(), true), // decimal 𝟙
            ("GB18030", b"\x81\x30\x85\x35".as_slice(), false), // numeric, not decimal ²
            ("GB18030", b"\x94\x39\xfc\x36".as_slice(), false), // emoji
        ] {
            let mut source = format!("# encoding: {name}\np \"\\").into_bytes();
            source.extend_from_slice(bytes);
            source.extend_from_slice(b"\"\n");
            let parsed = crate::parser::parse(&source);
            assert_eq!(parsed.errors().count(), 0, "{name}");
            let encoding = ParsedEncoding::from_parse(&parsed);
            assert_eq!(encoding.width(bytes), bytes.len(), "{name}");
            assert_eq!(encoding.alphanumeric(bytes), expected, "{name}: {bytes:?}");
        }
        for name in [b"EUC-KR".as_slice(), b"CP949", b"GB18030"] {
            let ranges = legacy_alnum(name);
            assert!(!ranges.is_empty());
            assert!(ranges.iter().all(|&(start, end)| start <= end));
            assert!(ranges.windows(2).all(|w| w[0].1 < w[1].0));
        }
    }

    #[test]
    fn recovery_review_additional_legacy_profiles_keep_posix_punctuation_distinct() {
        for (name, bytes, expected) in [
            ("GB2312", b"\xd6\xd0".as_slice(), true),
            ("GB2312", b"\xa1\xa2".as_slice(), false),
            ("Big5-HKSCS", b"\xa4\xa4".as_slice(), true),
            ("Big5-HKSCS", b"\xa1\x42".as_slice(), false),
        ] {
            let source = format!("# encoding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{name}");
            let encoding = ParsedEncoding::from_parse(&parsed);
            assert_eq!(encoding.width(bytes), bytes.len(), "{name}");
            assert_eq!(encoding.alphanumeric(bytes), expected, "{name}: {bytes:?}");
        }
    }

    #[test]
    fn recovery_review_utf8_alnum_uses_pinned_unicode_not_regex_crate_version() {
        for name in ["UTF-8", "CP65001"] {
            let source = format!("# encoding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            let encoding = ParsedEncoding::from_parse(&parsed);
            for (character, expected) in [
                ('\u{1c89}', false),
                ('\u{a7cb}', false),
                ('\u{16100}', false),
                ('\u{1c88}', true),
                ('\u{a7ca}', true),
                ('\u{160ff}', false),
                ('\u{16a40}', true),
                ('Ⅰ', true),
                ('１', true),
                ('²', false),
                ('、', false),
            ] {
                let mut bytes = [0; 4];
                let bytes = character.encode_utf8(&mut bytes).as_bytes();
                assert_eq!(
                    encoding.alphanumeric(bytes),
                    expected,
                    "{name}: {character}"
                );
            }
        }
    }

    #[test]
    fn continuation_all_unicode_alnum_profiles_and_aliases_use_pinned_classes() {
        let mut failures = Vec::new();
        for name in [
            "UTF-8",
            "CP65001",
            "UTF8-MAC",
            "UTF-8-HFS",
            "UTF8-DoCoMo",
            "UTF8-KDDI",
            "UTF8-SoftBank",
            "CESU-8",
        ] {
            let source = format!("# coding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{name}");
            let encoding = ParsedEncoding::from_parse(&parsed);
            for (character, expected) in [
                ('中', true),
                ('\u{a7ca}', true),
                ('１', true),
                ('、', false),
                ('\u{1c89}', false),
            ] {
                if encoding.alphanumeric(character.to_string().as_bytes()) != expected {
                    failures.push(format!("{name} U+{:04X}", character as u32));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn continuation_remaining_legacy_families_project_posix_classes() {
        for (name, letter, digit, punctuation) in [
            (
                "CP51932",
                b"\xc3\xe6".as_slice(),
                b"\xa3\xb1".as_slice(),
                b"\xa1\xa2".as_slice(),
            ),
            ("eucJP-ms", b"\xc3\xe6", b"\xa3\xb1", b"\xa1\xa2"),
            ("EUC-JIS-2004", b"\xc3\xe6", b"\xa3\xb1", b"\xa1\xa2"),
            ("Big5-UAO", b"\xa4\xa4", b"\xa2\xb0", b"\xa1\x42"),
            ("CP950", b"\xa4\xa4", b"\xa2\xb0", b"\xa1\x42"),
            ("CP951", b"\xa4\xa4", b"\xa2\xb0", b"\xa1\x42"),
            ("GB12345", b"\xd6\xd0", b"\xa3\xb1", b"\xa1\xa2"),
            ("SJIS-DoCoMo", b"\x92\x86", b"\x82\x50", b"\x81\x41"),
            ("SJIS-KDDI", b"\x92\x86", b"\x82\x50", b"\x81\x41"),
            ("SJIS-SoftBank", b"\x92\x86", b"\x82\x50", b"\x81\x41"),
            (
                "stateless-ISO-2022-JP",
                b"\x92\xc3\xe6",
                b"\x92\xa3\xb1",
                b"\x92\xa1\xa2",
            ),
            (
                "stateless-ISO-2022-JP-KDDI",
                b"\x92\xc3\xe6",
                b"\x92\xa3\xb1",
                b"\x92\xa1\xa2",
            ),
            ("IBM437", b"\x82", b"1", b"\xfd"), // é, decimal, superscript ²
            ("macRoman", b"\x8e", b"1", b"\xa5"), // é, decimal, bullet
        ] {
            let source = format!("# coding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{name}");
            let encoding = ParsedEncoding::from_parse(&parsed);
            assert!(encoding.alphanumeric(letter), "{name}");
            assert!(encoding.alphanumeric(digit), "{name}");
            assert!(!encoding.alphanumeric(punctuation), "{name}");
        }
    }

    #[test]
    fn continuation_mobile_unicode_projection_keeps_alphabetic_and_decimal_expansions() {
        for (name, letter, decimal, punctuation) in [
            ("UTF8-DoCoMo", '\u{e65c}', '\u{e6e2}', '\u{e63e}'),
            ("UTF8-KDDI", '\u{e4a6}', '\u{e522}', '\u{e488}'),
            ("UTF8-SoftBank", '\u{e14f}', '\u{e21c}', '\u{e04a}'),
        ] {
            let source = format!("# coding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            let encoding = ParsedEncoding::from_parse(&parsed);
            assert!(
                encoding.alphanumeric(letter.to_string().as_bytes()),
                "{name} letter"
            );
            assert!(
                encoding.alphanumeric(decimal.to_string().as_bytes()),
                "{name} decimal"
            );
            assert!(
                !encoding.alphanumeric(punctuation.to_string().as_bytes()),
                "{name} punctuation"
            );
        }
    }

    #[test]
    fn naming_uppercase_is_pinned_and_encoding_aware() {
        for (name, upper, lower) in [
            ("UTF-8", "É".as_bytes(), "é".as_bytes()),
            ("Windows-1252", b"\xc9".as_slice(), b"\xe9".as_slice()),
            ("ISO-8859-1", b"\xc9".as_slice(), b"\xe9".as_slice()),
            ("Shift_JIS", b"\x82\x60".as_slice(), b"\x82\x81".as_slice()),
            ("EUC-JP", b"\xa3\xc1".as_slice(), b"\xa3\xe1".as_slice()),
            (
                "CESU-8",
                b"\xed\xa0\x81\xed\xb0\x80".as_slice(),
                b"\xed\xa0\x81\xed\xb0\xa8".as_slice(),
            ),
        ] {
            let source = format!("# coding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            let encoding = ParsedEncoding::from_parse(&parsed);
            assert!(encoding.contains_uppercase(b"fooA"), "{name}");
            assert!(!encoding.contains_uppercase(b"foo_a1"), "{name}");
            assert!(encoding.contains_uppercase(upper), "{name}");
            assert!(!encoding.contains_uppercase(lower), "{name}");
        }
        let parsed = crate::parser::parse(b"");
        let encoding = ParsedEncoding::from_parse(&parsed);
        // Unicode 16 additions must not silently change the Unicode-15 pin.
        assert!(!encoding.contains_uppercase("\u{a7cc}".as_bytes()));
        assert!(!encoding.contains_uppercase("ǅ".as_bytes())); // titlecase, not POSIX upper
        assert!(encoding.contains_uppercase("Ⅳ".as_bytes())); // uppercase property, not only Lu
    }

    #[test]
    fn ns06_uppercase_spelling_uses_the_pinned_mapping_and_effective_alias() {
        for (name, lower, upper) in [
            ("UTF-8", "caféßǅ".as_bytes(), "CAFÉSSǄ".as_bytes()),
            ("UTF8-MAC", "sql".as_bytes(), "SQL".as_bytes()),
            ("ISO8859-1", b"caf\xe9".as_slice(), b"CAF\xc9".as_slice()),
            ("CP932", b"\x83\x60sql", b"\x83\x60SQL"),
            ("EUC-JP", b"\xa3\xe1sql", b"\xa3\xc1SQL"),
            (
                "CESU-8",
                b"\xed\xa0\x81\xed\xb0\xa8",
                b"\xed\xa0\x81\xed\xb0\x80",
            ),
            ("UTF-8", "\u{a7cd}".as_bytes(), "\u{a7cd}".as_bytes()),
        ] {
            let source = format!("# coding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0);
            assert_eq!(
                ParsedEncoding::from_parse(&parsed)
                    .heredoc_uppercase(lower)
                    .as_deref(),
                Some(upper),
                "{name}"
            );
        }
    }

    #[test]
    fn ns06_mac_composition_facts_fit_and_match_the_bounded_normalizer() {
        let parsed = crate::parser::parse(b"# coding: UTF8-MAC\n");
        let encoding = ParsedEncoding::from_parse(&parsed);
        for &(raw, canonical) in heredoc_uppercase::MAC {
            assert!((3..=9).contains(&raw.len()), "{raw:?}");
            assert_eq!(
                encoding.heredoc_normalized(raw).as_ref(),
                canonical,
                "{raw:?}"
            );
        }
        for (raw, canonical) in [("å\u{301}", "ǻ"), ("Å\u{301}", "Ǻ")] {
            assert_eq!(
                encoding.heredoc_normalized(raw.as_bytes()).as_ref(),
                canonical.as_bytes()
            );
        }
    }

    #[test]
    fn composed_chunks_count_pinned_mac_projections_without_moving_line_breaks() {
        for name in ["UTF8-MAC", "UTF-8-HFS"] {
            let source = format!("# coding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            let encoding = ParsedEncoding::from_parse(&parsed);
            let count = |mut bytes: &[u8]| {
                let mut total = 0;
                while !bytes.is_empty() {
                    let (width, characters) = encoding.composed_character_chunk(bytes);
                    assert!(width > 0 && width <= bytes.len());
                    assert!(characters > 0);
                    assert!(width == 1 || !bytes[..width].contains(&b'\n'));
                    bytes = &bytes[width..];
                    total += characters;
                }
                total
            };
            for &(raw, canonical) in heredoc_uppercase::MAC {
                assert_eq!(
                    count(raw),
                    std::str::from_utf8(canonical).unwrap().chars().count(),
                    "{name}: {raw:?}"
                );
            }
            for raw in ["e\u{301}", "a\u{302}\u{301}", "가", "각", "각"] {
                assert_eq!(count(raw.as_bytes()), 1, "{name}: {raw:?}");
            }
            assert_eq!(count("e\u{301}\nx".as_bytes()), 3);
        }
        let parsed = crate::parser::parse(b"# coding: UTF-8\n");
        let encoding = ParsedEncoding::from_parse(&parsed);
        assert_eq!(
            encoding.composed_character_chunk("e\u{301}".as_bytes()),
            (1, 1)
        );
    }

    #[test]
    fn unicode_lowercase_is_pinned_for_utf8_variants_and_cesu8() {
        for name in ["UTF-8", "CP65001", "UTF8-MAC", "UTF8-DoCoMo", "CESU-8"] {
            let source = format!("# coding: {name}\n");
            let parsed = crate::parser::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{name}");
            let encoding = ParsedEncoding::from_parse(&parsed);
            for (bytes, expected) in [
                ("A\u{a7cd}".as_bytes(), false),
                ("A\u{1c8a}".as_bytes(), false),
                ("Aé".as_bytes(), true),
            ] {
                assert_eq!(
                    encoding.contains_lowercase(bytes),
                    expected,
                    "{name}: {bytes:?}"
                );
            }
            let (lower, upper) = if name == "CESU-8" {
                (
                    b"\xed\xa0\x81\xed\xb0\xa8".as_slice(),
                    b"\xed\xa0\x81\xed\xb0\x80".as_slice(),
                )
            } else {
                ("\u{10428}".as_bytes(), "\u{10400}".as_bytes())
            };
            assert!(encoding.contains_lowercase(lower), "{name}");
            assert!(!encoding.contains_lowercase(upper), "{name}");
        }
    }

    #[test]
    fn follows_the_main_parses_effective_encoding_comment() {
        use SourceEncoding::{AsciiBlanks, SingleByteBlank, Utf8};
        for (source, expected) in [
            ("# coding: ascii-8bit\n", AsciiBlanks),
            ("# CoDiNg: ISO8859-1\n", SingleByteBlank(0xa0)),
            ("# -*- coding: CP1252 -*-\n", SingleByteBlank(0xa0)),
            ("# encoding: KOI8-U\n", SingleByteBlank(0x9a)),
            ("# coding = CP878\n", SingleByteBlank(0x9a)),
            ("# free-form coding: TIS-620\n", SingleByteBlank(0xa0)),
            ("# coding guidelines; coding: ascii-8bit\n", Utf8),
            ("# codingx; coding: ascii-8bit\n", AsciiBlanks),
            ("# coding: ascii-8bit; coding: ISO-8859-1\n", AsciiBlanks),
            ("# foo: coding:ascii-8bit\n", Utf8),
            ("# coding: CP1251\n", AsciiBlanks),
            ("# coding: Shift_JIS\n", AsciiBlanks),
            ("# coding: CP65001\n", Utf8),
            ("# coding: UTF8-MAC\n", Utf8),
            ("# coding: UTF8-DoCoMo\n", Utf8),
            ("# coding: CESU-8\n", Utf8),
            ("# coding: UTF-8-UNIX\n", Utf8),
            ("# comment\n# coding: ascii-8bit\n", Utf8),
            ("x = 0 # coding: ascii-8bit\n", Utf8),
            ("#!/usr/bin/ruby\n  # coding: ascii-8bit\n", AsciiBlanks),
            ("#!/bin/bash\n# coding: ascii-8bit\n", Utf8),
            ("=begin\n# coding: ascii-8bit\n=end\n", Utf8),
            ("\u{feff}# coding: ascii-8bit\n", Utf8),
        ] {
            let parsed = ruby_prism::parse(source.as_bytes());
            assert_eq!(parsed.errors().count(), 0, "{source:?}");
            assert_eq!(
                ParsedEncoding::from_parse(&parsed).blanks,
                expected,
                "{source:?}"
            );
        }
    }
}
