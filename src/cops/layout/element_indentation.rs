//! Character-column facts shared by the first-element indentation family
//! (`ClosingParenthesisIndentation`, `FirstArgumentIndentation`,
//! `FirstArrayElementIndentation`, `FirstHashElementIndentation`).
//!
//! Upstream measures these with parser `column`s (characters) and
//! `ProcessedSource#line_indentation`, unlike the display columns of
//! `Alignment#display_column`. Corrections reuse [`super::alignment::Alignment`].
use crate::source::SourceFile;

/// Ruby's `\s`: space, `\t`, `\n`, `\v`, `\f`, `\r`.
pub(super) fn ruby_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// The text of a 1-based line without its terminator (or a leading BOM).
pub(super) fn line(file: &SourceFile, line: u32) -> &[u8] {
    let text = file.line_text(line);
    if line == 1 {
        text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(text)
    } else {
        text
    }
}

/// `ProcessedSource#line_indentation`: the number of leading `\s` characters.
pub(super) fn line_indentation(file: &SourceFile, number: u32) -> usize {
    line(file, number)
        .iter()
        .take_while(|b| ruby_space(**b))
        .count()
}

/// A parser `column`: the number of characters before `offset` on its line.
pub(super) fn column(file: &SourceFile, offset: usize) -> usize {
    super::alignment::effective_column(file, offset)
}

/// The Ruby core extension's `String#blank?` (`lstrip.empty?`; `lstrip`
/// also removes NUL).
pub(super) fn blank(text: &[u8]) -> bool {
    text.iter().all(|b| ruby_space(*b) || *b == 0)
}

/// Apply a cop's corrections until nothing changes, like RuboCop's looping
/// `expect_correction`, validating every intermediate source.
#[cfg(test)]
pub(super) fn corrected(cop: &dyn crate::cops::Cop, source: &str) -> String {
    let mut current = source.to_owned();
    for _ in 0..10 {
        assert_eq!(
            crate::parser::parse(current.as_bytes()).errors().count(),
            0,
            "{current}"
        );
        let offenses = crate::cops::testing::correction_offenses(cop, &current);
        let applied = crate::corrections::apply(current.as_bytes(), &offenses).unwrap();
        if applied.count == 0 {
            return current;
        }
        current = String::from_utf8(applied.bytes).unwrap();
    }
    panic!("{}: corrections did not converge: {source}", cop.name());
}
